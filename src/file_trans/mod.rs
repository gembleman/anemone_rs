//! UI와 독립된 파일 번역 코어.
//!
//! 작업 설정, 경로 검증, 기본 출력 경로 생성과 실제 파일 처리를 제공한다.
//! 진행 상황은 [`FileTranslationProgress`]로 전달하므로 Win32 UI에 의존하지 않는다.

mod error;
mod eztrans;
mod input;
mod output;
mod pipeline;
mod progress;
mod supervisor;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::translation::PreparedJob;

pub use error::FileTranslationError;
pub(crate) use eztrans::split_eztrans_batch;
pub(crate) use input::read_utf8_preview;
pub(crate) use pipeline::run;
pub use progress::{FileTranslationProgress, FileTranslationSummary};
pub use supervisor::{
    CancelHandle, FileTranslationSupervisor, FileTranslationTask, ShutdownReport,
};

pub(crate) type ProgressEvent = FileTranslationProgress;
pub(crate) type FileTransTask = FileTranslationTask;

#[cfg(feature = "benchmark")]
pub mod benchmark_support {
    pub use super::eztrans::{BoundedTranslationCache, translate_eztrans_window};
    pub use super::input::{read_input_line, validate_and_count_reader};
}

/// 출력 형식.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum WriteType {
    /// 번역만 기록한다.
    #[default]
    TranslationOnly,
    /// 원문 다음 줄에 번역을 기록한다.
    OriginalAndTrans,
    /// 원문과 번역 뒤에 빈 줄을 기록한다.
    OriginalTransNewline,
}

/// UI와 무관한 파일 번역 작업 설정.
pub struct FileTranslationRequest {
    pub input_files: Vec<PathBuf>,
    pub output_files: Vec<PathBuf>,
    pub write_type: WriteType,
    pub no_trans_linefeed: bool,
    pub cancel_token: Arc<AtomicBool>,
    pub translation: PreparedJob,
}

pub(crate) type FileTransJobData = FileTranslationRequest;

/// Windows 파일 시스템의 대소문자 비구분 규칙에 맞춰 비교할 절대 경로 키를 만든다.
fn normalized_path_key(path: &Path) -> Result<String, FileTranslationError> {
    let absolute = std::path::absolute(path).map_err(|e| {
        FileTranslationError::path(format!(
            "경로를 절대 경로로 변환할 수 없습니다: {}\n{e}",
            path.display()
        ))
    })?;
    let normalized = fs::canonicalize(&absolute).unwrap_or(absolute);
    let text = normalized.to_string_lossy();
    let text = text
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| text.strip_prefix(r"\\?\").map(str::to_owned))
        .unwrap_or_else(|| text.into_owned());
    Ok(text.to_lowercase())
}

/// 입력과 출력 경로가 서로 겹치거나 출력 경로끼리 중복되는지 검사한다.
pub(crate) fn validate_job_paths(
    inputs: &[PathBuf],
    outputs: &[PathBuf],
) -> Result<(), FileTranslationError> {
    if inputs.len() != outputs.len() {
        return Err(FileTranslationError::InvalidRequest(
            "입력 파일과 출력 파일 수가 일치하지 않습니다.".to_string(),
        ));
    }

    let input_keys = inputs
        .iter()
        .map(|path| normalized_path_key(path).map(|key| (key, path)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut output_keys = HashSet::with_capacity(outputs.len());

    for output in outputs {
        let output_key = normalized_path_key(output)?;
        if let Some((_, input)) = input_keys.iter().find(|(key, _)| key == &output_key) {
            return Err(FileTranslationError::path(format!(
                "출력 파일이 입력 파일과 같습니다.\n입력: {}\n출력: {}",
                input.display(),
                output.display()
            )));
        }
        if !output_keys.insert(output_key) {
            return Err(FileTranslationError::path(format!(
                "출력 파일 경로가 중복됩니다.\n{}",
                output.display()
            )));
        }
    }

    Ok(())
}

/// 입력 순서대로 충돌하지 않는 기본 출력 경로를 만든다.
pub(crate) fn default_output_paths(
    inputs: &[PathBuf],
) -> Result<Vec<PathBuf>, FileTranslationError> {
    let input_keys = inputs
        .iter()
        .map(|path| normalized_path_key(path))
        .collect::<Result<HashSet<_>, _>>()?;
    let mut output_keys = HashSet::with_capacity(inputs.len());
    let mut outputs = Vec::with_capacity(inputs.len());

    for input in inputs {
        let stem = input.file_stem().unwrap_or_default().to_string_lossy();
        let parent = input.parent().unwrap_or(Path::new(""));
        let mut suffix = 1usize;
        loop {
            let filename = if suffix == 1 {
                format!("{stem}_번역.txt")
            } else {
                format!("{stem}_번역_{suffix}.txt")
            };
            let candidate = parent.join(filename);
            let key = normalized_path_key(&candidate)?;
            if !input_keys.contains(&key) && output_keys.insert(key) {
                outputs.push(candidate);
                break;
            }
            suffix += 1;
        }
    }

    Ok(outputs)
}

#[cfg(test)]
#[path = "../../tests/unit/file_trans/mod.rs"]
mod tests;

#[cfg(test)]
use eztrans::{BoundedTranslationCache, partition_eztrans_batches, translate_eztrans_window};
#[cfg(test)]
use input::{InputLine, LineEnding, read_input_line, validate_and_count_reader};
#[cfg(test)]
use output::{PendingOutput, write_output};
#[cfg(test)]
use pipeline::{TranslationContext, translate_line};

#[cfg(test)]
#[path = "../../tests/unit/file_trans/worker.rs"]
mod worker_tests;
