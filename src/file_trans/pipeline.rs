//! 파일과 줄 처리 흐름 및 번역 backend 연결.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use windows::Win32::System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState};

use super::eztrans::{
    BoundedTranslationCache, EZTRANS_CACHE_MAX_ENTRIES, EZTRANS_WINDOW_MAX_CHARS,
    EZTRANS_WINDOW_MAX_LINES, should_translate_line, translate_eztrans_window,
};
use super::input::{InputLine, open_utf8_translation_input, preflight_inputs, read_input_line};
use super::output::{PendingOutput, write_output};
use super::{
    FileTransJobData, FileTranslationError, FileTranslationSummary, ProgressEvent,
    validate_job_paths,
};
use crate::translation::{
    EzTransBatchTranslator, EzTransProcessPoolRegistry, PreparedJob,
    http_common::create_client,
    worker::{TranslationDispatch, TranslationRequest},
};

const PROGRESS_REPORT_INTERVAL: usize = 256;
/// non-blocking(HTTP) 엔진의 파일 번역 최대 in-flight 요청 수.
/// GUI 워커는 4로 제한하는데, 파일 번역은 워커와 독립된 자체 게이트를 사용한다.
const FILE_HTTP_CONCURRENCY: usize = 8;
/// non-blocking(HTTP) 엔진의 파일 번역 배치 크기(줄 단위).
/// EzTrans처럼 서버 창(window) 개념이 없어 줄 수 자체에는 제한이 없지만, 배치가
/// 너무 작으면(예: 이전처럼 1줄) FILE_HTTP_CONCURRENCY로 올린 JoinSet 동시성이
/// 전혀 발휘되지 못한다. 반대로 너무 크게 잡으면 진행률 보고가 배치 완료
/// 시점에 몰리는데, PROGRESS_REPORT_INTERVAL(256)과 너무 가까우면 진행률 바가
/// 뚝뚝 끊긴다. 상시 FILE_HTTP_CONCURRENCY개를 in-flight로 유지하기에 충분하면서도
/// PROGRESS_REPORT_INTERVAL보다 한참 작은 동시성의 4배(32)로 잡는다.
const FILE_HTTP_BATCH_LINES: usize = FILE_HTTP_CONCURRENCY * 4;

#[derive(Clone)]
pub(super) struct FilePipelineServices {
    http_client: reqwest::Client,
    eztrans_pools: std::sync::Arc<EzTransProcessPoolRegistry>,
}

impl FilePipelineServices {
    pub(super) fn new(
        http_client: reqwest::Client,
        eztrans_pools: std::sync::Arc<EzTransProcessPoolRegistry>,
    ) -> Self {
        Self {
            http_client,
            eztrans_pools,
        }
    }
}

/// 작업 중 시스템 절전만 막고 화면 절전은 허용하는 RAII 가드.
struct SleepBlocker;

impl SleepBlocker {
    fn new() -> Self {
        // SAFETY: SetThreadExecutionState는 현재 worker thread의 실행 상태만 바꾼다.
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
        }
        Self
    }
}

impl Drop for SleepBlocker {
    fn drop(&mut self) {
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS);
        }
    }
}

/// 파일 번역 작업을 실행하고 terminal 결과를 정확히 한 번 보고한다.
pub fn run(job_data: &FileTransJobData, report: impl Fn(ProgressEvent)) {
    let services = FilePipelineServices::new(
        create_client(),
        std::sync::Arc::new(EzTransProcessPoolRegistry::new()),
    );
    run_with_services(job_data, &services, report);
}

pub(super) fn run_with_services(
    job_data: &FileTransJobData,
    services: &FilePipelineServices,
    report: impl Fn(ProgressEvent),
) {
    let _sleep_guard = SleepBlocker::new();
    let result = run_inner(job_data, services, &report);
    report(ProgressEvent::Finished(result));
}

fn run_inner(
    job_data: &FileTransJobData,
    services: &FilePipelineServices,
    report: &impl Fn(ProgressEvent),
) -> Result<FileTranslationSummary, FileTranslationError> {
    validate_job_paths(&job_data.input_files, &job_data.output_files)?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| FileTranslationError::Runtime(error.to_string()))?;
    let translation = TranslationContext {
        runtime: &runtime,
        http_client: &services.http_client,
        // 배치(process_single_file 호출)마다 새로 만들지 않고 작업(job) 전체에서
        // 공유한다 — 배치 경계마다 세마포어가 새로 생기면 그 순간 in-flight가
        // 0으로 떨어져 전역 상한이 아니라 배치 내부 상한이 돼버린다.
        http_concurrency: Arc::new(tokio::sync::Semaphore::new(FILE_HTTP_CONCURRENCY)),
    };

    let file_line_counts = preflight_inputs(&job_data.input_files, &job_data.cancel_token)?;
    let total_lines = file_line_counts
        .iter()
        .try_fold(0i32, |total, &count| {
            i32::try_from(count)
                .ok()
                .and_then(|count| total.checked_add(count))
        })
        .ok_or(FileTranslationError::TooManyLines)?;

    let eztrans_pool = if let Some(config) = job_data.translation.engine().eztrans_process() {
        Some(
            services
                .eztrans_pools
                .get(config)
                .map_err(FileTranslationError::backend)?,
        )
    } else {
        None
    };

    report(ProgressEvent::TotalFiles(job_data.input_files.len() as i32));
    report(ProgressEvent::TotalLines(total_lines));

    let mut global_current_line = 0;
    let mut file_runtime = FileRuntime {
        translation: &translation,
        eztrans_pool: eztrans_pool
            .as_deref()
            .map(|pool| pool as &dyn EzTransBatchTranslator),
        eztrans_cache: BoundedTranslationCache::new(EZTRANS_CACHE_MAX_ENTRIES),
    };

    for (index, (input_path, output_path)) in job_data
        .input_files
        .iter()
        .zip(&job_data.output_files)
        .enumerate()
    {
        if job_data.cancel_token.load(Ordering::SeqCst) {
            return Err(FileTranslationError::Cancelled);
        }

        report(ProgressEvent::FileIndex((index + 1) as i32));
        send_filename(input_path, report);
        process_single_file(
            input_path,
            output_path,
            job_data,
            file_line_counts[index],
            &mut global_current_line,
            &mut file_runtime,
            report,
        )?;
    }

    Ok(FileTranslationSummary {
        total_files: job_data.input_files.len(),
        total_lines: total_lines as usize,
    })
}

pub struct TranslationContext<'a> {
    runtime: &'a tokio::runtime::Runtime,
    http_client: &'a reqwest::Client,
    /// non-blocking 엔진 요청의 in-flight 상한 게이트. `translate_lines` 호출(배치)
    /// 마다 새로 만들지 않고 이 컨텍스트 수명(=파일 번역 작업 전체) 동안 공유해야
    /// 실제로 전역 상한으로 동작한다.
    http_concurrency: Arc<tokio::sync::Semaphore>,
}

impl<'a> TranslationContext<'a> {
    #[cfg(test)]
    pub fn new(runtime: &'a tokio::runtime::Runtime, http_client: &'a reqwest::Client) -> Self {
        Self {
            runtime,
            http_client,
            http_concurrency: Arc::new(tokio::sync::Semaphore::new(FILE_HTTP_CONCURRENCY)),
        }
    }
}

struct FileRuntime<'a> {
    translation: &'a TranslationContext<'a>,
    eztrans_pool: Option<&'a dyn EzTransBatchTranslator>,
    eztrans_cache: BoundedTranslationCache,
}

fn process_single_file(
    input_path: &Path,
    output_path: &Path,
    job_data: &FileTransJobData,
    line_count: usize,
    global_current: &mut i32,
    runtime: &mut FileRuntime<'_>,
    report: &impl Fn(ProgressEvent),
) -> Result<(), FileTranslationError> {
    let mut reader =
        open_utf8_translation_input(input_path).map_err(FileTranslationError::input)?;
    let mut pending_output = PendingOutput::create(output_path)?;
    pending_output
        .writer()
        .write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|error| FileTranslationError::output(error.to_string()))?;

    report(ProgressEvent::FileLines(line_count as i32));
    let mut next_line = read_input_line(&mut reader, input_path, true)?;
    let mut line_index = 0usize;

    while next_line.is_some() {
        let mut lines = Vec::new();
        let mut window_chars = 0usize;

        while let Some(line) = next_line.take() {
            let blocking = job_data.translation.engine().is_blocking();
            if !blocking {
                // non-blocking 엔진은 EzTrans 같은 서버 창(window) 개념이 없어
                // separator/window char 카운트는 필요 없다. 다만 JoinSet 동시성이
                // 실제로 발휘되려면 여러 줄을 모아 한 번에 translate_lines로 넘겨야
                // 하므로, FILE_HTTP_BATCH_LINES에 도달하거나 파일 끝(next_line이
                // None)에 도달할 때까지 계속 모은다.
                lines.push(line);
                next_line = read_input_line(&mut reader, input_path, false)?;
                if lines.len() >= FILE_HTTP_BATCH_LINES {
                    break;
                }
                continue;
            }

            let separator_chars = usize::from(!lines.is_empty());
            let line_chars = line.text.chars().count();
            let exceeds_window = !lines.is_empty()
                && (lines.len() >= EZTRANS_WINDOW_MAX_LINES
                    || window_chars
                        .saturating_add(separator_chars)
                        .saturating_add(line_chars)
                        > EZTRANS_WINDOW_MAX_CHARS);
            if exceeds_window {
                next_line = Some(line);
                break;
            }

            window_chars = window_chars
                .saturating_add(separator_chars)
                .saturating_add(line_chars);
            lines.push(line);
            next_line = read_input_line(&mut reader, input_path, false)?;
        }

        if job_data.cancel_token.load(Ordering::SeqCst) {
            return Err(FileTranslationError::Cancelled);
        }
        let translated_lines = if job_data.translation.engine().is_blocking() {
            let pool = runtime.eztrans_pool.ok_or_else(|| {
                FileTranslationError::backend("EzTrans 파일 번역 helper 풀이 준비되지 않았습니다")
            })?;
            translate_eztrans_window(&lines, job_data, pool, &mut runtime.eztrans_cache)?
        } else {
            translate_lines(&lines, job_data, runtime.translation)?
        };

        let batch_len = lines.len();
        for (batch_index, (line, translated)) in lines.into_iter().zip(translated_lines).enumerate()
        {
            write_output(
                pending_output.writer(),
                &line.text,
                &translated,
                job_data.write_type,
                line.ending,
                batch_index + 1 < batch_len || next_line.is_some(),
            )?;

            line_index += 1;
            *global_current += 1;
            if line_index == 1
                || line_index == line_count
                || line_index.is_multiple_of(PROGRESS_REPORT_INTERVAL)
            {
                report(ProgressEvent::FileProgress(line_index as i32));
                report(ProgressEvent::TotalProgress(*global_current));
            }

            // 배치 하나에 여러 줄이 모여도(non-blocking 배치, EzTrans 창) 취소
            // 응답성은 줄 단위를 유지해야 한다. 배치 시작 전 한 번만 검사하면
            // 배치가 커질수록 취소가 늦게 반영되고, 이미 번역된 나머지 줄까지
            // 써버린 뒤 persist까지 끝나버릴 수 있다 — 여기서 즉시 끊어
            // 남은 줄을 쓰지 않고 pending_output을 persist하지 않은 채 반환한다.
            if job_data.cancel_token.load(Ordering::SeqCst) {
                return Err(FileTranslationError::Cancelled);
            }
        }
    }

    pending_output.persist()
}

/// non-blocking 엔진 줄 번역을 제한된 동시성으로 실행하고 결과를 입력 순서로 재조립한다.
///
/// 1줄 = 1 HTTP 요청인 구조는 그대로지만, 줄 단위 완주 대기(`block_on`)를 없애고
/// 최대 `FILE_HTTP_CONCURRENCY`개의 요청을 동시에 in-flight로 둔다. current_thread
/// 런타임이어도 I/O await 지점에서 인터리빙되므로 줄당 왕복 대신 배치 단위로
/// 지연이 겹친다. 취소 시 JoinSet을 drop해 in-flight 요청을 함께 취소한다.
fn translate_lines(
    lines: &[InputLine],
    job_data: &FileTransJobData,
    translation: &TranslationContext<'_>,
) -> Result<Vec<String>, FileTranslationError> {
    if lines.is_empty() {
        return Ok(Vec::new());
    }
    let originals: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
    translation.runtime.block_on(translate_lines_async(
        &originals,
        job_data,
        translation.http_client,
        Arc::clone(&translation.http_concurrency),
    ))
}

async fn translate_lines_async(
    lines: &[&str],
    job_data: &FileTransJobData,
    client: &reqwest::Client,
    concurrency: std::sync::Arc<tokio::sync::Semaphore>,
) -> Result<Vec<String>, FileTranslationError> {
    let mut tasks = tokio::task::JoinSet::new();
    // JoinSet::join_next()의 Err(JoinError)는 어떤 태스크가 panic했는지 값으로
    // 알려주지 않는다. spawn 시점에 발급되는 task::Id를 index로 되짚을 수 있게
    // 맵으로 남겨 둔다 — panic한 줄을 조용히 원문으로 흘리지 않기 위해서다.
    let mut task_indices: HashMap<tokio::task::Id, usize> = HashMap::new();
    for (index, &line) in lines.iter().enumerate() {
        if !should_translate_line(line, job_data.no_trans_linefeed) {
            continue;
        }
        let task_client = client.clone();
        let task_job = job_data.translation.clone();
        let task_cancel = Arc::clone(&job_data.cancel_token);
        let task_concurrency = Arc::clone(&concurrency);
        let text: Arc<str> = Arc::from(line);
        let abort_handle = tasks.spawn(async move {
            (
                index,
                translate_line_task(text, task_job, task_client, task_cancel, task_concurrency).await,
            )
        });
        task_indices.insert(abort_handle.id(), index);
    }

    let mut results: Vec<Option<String>> = vec![None; lines.len()];
    let outcome: Result<(), FileTranslationError> = tokio::select! {
        _ = wait_for_cancellation(&job_data.cancel_token) => Err(FileTranslationError::Cancelled),
        _ = async {
            while let Some(joined) = tasks.join_next_with_id().await {
                match joined {
                    Ok((_, (index, translated))) => results[index] = Some(translated),
                    Err(join_error) => {
                        tracing::warn!("file translation task panicked: {join_error}");
                        // should_translate_line이 false라 애초에 spawn하지 않은 줄은
                        // task_indices에 없으므로 여기서 건드리지 않는다 — 그 경우
                        // results[index]가 None으로 남아 원문 유지되는 것이 정상이다.
                        // 반면 panic한 태스크는 반드시 실패 표식을 남겨야 한다: None을
                        // 그대로 두면 최종 조립에서 원문으로 대체돼 "번역 실패"가 아니라
                        // "번역 안 된 원문"이 조용히 출력된다.
                        if let Some(&index) = task_indices.get(&join_error.id()) {
                            results[index] = Some(format!("[번역 실패: 작업 패닉: {join_error}]"));
                        }
                    }
                }
            }
        } => Ok(()),
    };
    outcome?;

    // 드레인 브랜치가 wait_for_cancellation(20ms 폴링)보다 먼저 끝나면 취소가
    // 진행 중이어도 위 select 전체가 Ok(())로 빠질 수 있다. 그 사이 개별 태스크가
    // 취소를 감지해 남긴 "[번역 실패: 취소됨]" 결과가 정상 번역인 것처럼
    // write_output까지 흘러가지 않도록, 드레인이 끝난 뒤에도 취소 상태를 다시
    // 확인해 취소면 결과를 버리고 명시적으로 실패시킨다.
    if job_data.cancel_token.load(Ordering::SeqCst) {
        return Err(FileTranslationError::Cancelled);
    }

    Ok(results
        .into_iter()
        .zip(lines)
        .map(|(result, &original)| match result {
            Some(translated) => translated,
            None => original.to_string(),
        })
        .collect())
}

/// 한 줄을 제한 동시성으로 번역한다. 빈 줄/줄바꿈 전용 줄은 유지하고
/// HTTP 실패는 표식으로 바꿔 계속한다. Err 반환은 없어 JoinSet 드레인이
/// 항상 완주한다 — 취소는 바깥 select의 JoinSet drop으로 처리된다.
async fn translate_line_task(
    text: Arc<str>,
    job: PreparedJob,
    client: reqwest::Client,
    cancel_token: Arc<std::sync::atomic::AtomicBool>,
    concurrency: Arc<tokio::sync::Semaphore>,
) -> String {
    let _permit = match concurrency.acquire_owned().await {
        Ok(permit) => permit,
        Err(_) => return "[번역 실패: 동시성 게이트가 닫혔습니다]".to_string(),
    };
    if cancel_token.load(Ordering::SeqCst) {
        return "[번역 실패: 취소됨]".to_string();
    }
    let request = TranslationRequest {
        id: 0,
        text,
        job,
    };
    match TranslationDispatch::translate_async(&request, &client).await {
        Ok(translated) => translated,
        Err(error) => {
            tracing::warn!(
                category = error.log_category(),
                status_code = ?error.log_status_code(),
                "file translation line failed"
            );
            format!("[번역 실패: {error}]")
        }
    }
}

/// 한 줄을 동기 번역한다. 빈 줄은 유지하고 HTTP 실패는 표식으로 바꿔 계속한다.
/// 실제 파이프라인은 `translate_lines`(제한 동시성)를 쓰고, 이 함수는 단일 줄
/// 취소 동작을 검증하는 테스트 전용으로 남긴다.
#[cfg(test)]
pub fn translate_line(
    line: &str,
    job_data: &FileTransJobData,
    translation: &TranslationContext<'_>,
) -> Result<String, FileTranslationError> {
    if !should_translate_line(line, job_data.no_trans_linefeed) {
        return Ok(line.to_string());
    }

    let request = TranslationRequest {
        id: 0,
        text: std::sync::Arc::from(line),
        job: job_data.translation.clone(),
    };

    let result = translation.runtime.block_on(async {
        tokio::select! {
            result = TranslationDispatch::translate_async(&request, translation.http_client) => Some(result),
            () = wait_for_cancellation(&job_data.cancel_token) => None,
        }
    });

    match result {
        None => Err(FileTranslationError::Cancelled),
        Some(Ok(translated)) => Ok(translated),
        Some(Err(error)) => {
            tracing::warn!(
                category = error.log_category(),
                status_code = ?error.log_status_code(),
                input_bytes = line.len(),
                "file translation line failed"
            );
            Ok(format!("[번역 실패: {error}]"))
        }
    }
}

async fn wait_for_cancellation(token: &std::sync::atomic::AtomicBool) {
    while !token.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

fn send_filename(path: &Path, report: &impl Fn(ProgressEvent)) {
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    report(ProgressEvent::FileName(filename));
}

#[cfg(test)]
#[path = "../../tests/unit/file_trans/pipeline.rs"]
mod tests;
