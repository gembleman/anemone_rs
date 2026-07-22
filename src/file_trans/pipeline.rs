//! 파일과 줄 처리 흐름 및 번역 backend 연결.

use std::io::Write;
use std::path::Path;
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
    EzTransBatchTranslator, EzTransProcessPoolRegistry,
    http_common::create_client,
    worker::{TranslationDispatch, TranslationRequest},
};

const PROGRESS_REPORT_INTERVAL: usize = 256;

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
}

impl<'a> TranslationContext<'a> {
    #[cfg(test)]
    pub fn new(runtime: &'a tokio::runtime::Runtime, http_client: &'a reqwest::Client) -> Self {
        Self {
            runtime,
            http_client,
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
            let separator_chars = usize::from(!lines.is_empty());
            let line_chars = line.text.chars().count();
            let exceeds_window = !lines.is_empty()
                && job_data.translation.engine().is_blocking()
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

            if !job_data.translation.engine().is_blocking() {
                break;
            }
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
        }
    }

    pending_output.persist()
}

fn translate_lines(
    lines: &[InputLine],
    job_data: &FileTransJobData,
    translation: &TranslationContext<'_>,
) -> Result<Vec<String>, FileTranslationError> {
    lines
        .iter()
        .map(|line| translate_line(&line.text, job_data, translation))
        .collect()
}

/// 한 줄을 동기 번역한다. 빈 줄은 유지하고 HTTP 실패는 표식으로 바꿔 계속한다.
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
