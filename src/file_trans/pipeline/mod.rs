//! 파일과 줄 처리 흐름 및 번역 backend 연결.
//!
//! 실제 배치 수집/쓰기 단계는 [`batch`]에, non-blocking 엔진 동시 번역은
//! [`translate`]에 있다. 이 파일은 작업 전체(파일 목록 순회, 런타임/세마포어
//! 수명, 진행 이벤트 흐름)를 오케스트레이션한다.

mod batch;
mod translate;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use windows_sys::Win32::System::Power::{
    ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
};

use batch::process_single_file;
#[cfg(test)]
pub use translate::translate_line;
#[cfg(test)]
use translate::translate_lines;

use super::eztrans::{BoundedTranslationCache, EZTRANS_CACHE_MAX_ENTRIES};
#[cfg(test)]
use super::input::InputLine;
use super::input::preflight_inputs;
use super::{
    FileTranslationError, FileTranslationProgress, FileTranslationRequest, FileTranslationSummary,
    validate_job_paths,
};
use crate::translation::{
    EzTransBatchTranslator, EzTransProcessPoolRegistry, http_common::create_client,
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
pub fn run(job_data: &FileTranslationRequest, report: impl Fn(FileTranslationProgress)) {
    let services = FilePipelineServices::new(
        create_client(),
        std::sync::Arc::new(EzTransProcessPoolRegistry::new()),
    );
    run_with_services(job_data, &services, report);
}

pub(super) fn run_with_services(
    job_data: &FileTranslationRequest,
    services: &FilePipelineServices,
    report: impl Fn(FileTranslationProgress),
) {
    let _sleep_guard = SleepBlocker::new();
    let result = run_inner(job_data, services, &report);
    report(FileTranslationProgress::Finished(result));
}

fn run_inner(
    job_data: &FileTranslationRequest,
    services: &FilePipelineServices,
    report: &impl Fn(FileTranslationProgress),
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

    report(FileTranslationProgress::TotalFiles(
        job_data.input_files.len() as i32,
    ));
    report(FileTranslationProgress::TotalLines(total_lines));

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

        report(FileTranslationProgress::FileIndex((index + 1) as i32));
        send_filename(input_path, report);
        process_single_file(
            FileJob {
                input_path,
                output_path,
                job_data,
                line_count: file_line_counts[index],
            },
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

/// 파일 하나를 처리하는 동안 필요한 번역 관련 상태(런타임, EzTrans 풀/캐시).
struct FileRuntime<'a> {
    translation: &'a TranslationContext<'a>,
    eztrans_pool: Option<&'a dyn EzTransBatchTranslator>,
    eztrans_cache: BoundedTranslationCache,
}

/// [`batch::process_single_file`] 호출에 필요한 파일 단위 인자를 묶은 컨텍스트.
///
/// 이전에는 입력/출력 경로, job 데이터, 줄 수를 개별 파라미터로 넘겨 함수
/// 시그니처가 7개 인자까지 늘어났다. 관련 있는 값을 하나로 묶어 파라미터 수와
/// 호출부 복잡도를 줄인다.
struct FileJob<'a> {
    input_path: &'a Path,
    output_path: &'a Path,
    job_data: &'a FileTranslationRequest,
    line_count: usize,
}

fn send_filename(path: &Path, report: &impl Fn(FileTranslationProgress)) {
    let filename = path.file_name().map_or_else(
        || "unknown".to_string(),
        |name| name.to_string_lossy().to_string(),
    );
    report(FileTranslationProgress::FileName(filename));
}

#[cfg(test)]
#[path = "../../../tests/unit/file_trans/pipeline.rs"]
mod tests;
