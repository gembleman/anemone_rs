//! UI와 독립된 파일 번역 코어.
//!
//! 작업 설정, 경로 검증, 기본 출력 경로 생성과 실제 파일 처리를 제공한다.
//! 진행 상황은 [`ProgressEvent`] 콜백으로 전달하므로 Win32 UI에 의존하지 않는다.

mod worker;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;

use crate::translation::{EngineCredentials, Language, TranslationEngine};

pub(crate) use worker::run;

/// 파일 번역 작업을 취소하는 스레드 안전한 핸들.
#[derive(Clone)]
pub(crate) struct CancelHandle(Arc<AtomicBool>);

impl CancelHandle {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// 워커 수명과 진행 이벤트 수신기를 소유하는 파일 번역 작업.
pub(crate) struct FileTransTask {
    cancel: CancelHandle,
    events: Receiver<ProgressEvent>,
    worker: Option<JoinHandle<()>>,
}

impl FileTransTask {
    pub(crate) fn cancel(&self) {
        self.cancel.cancel();
    }
    pub(crate) fn drain_events(&self) -> Vec<ProgressEvent> {
        self.events.try_iter().collect()
    }
}

impl Drop for FileTransTask {
    fn drop(&mut self) {
        self.cancel();
        // UI 스레드를 막지 않도록 완료 대기는 하지 않는다. JoinHandle은 작업 객체가
        // 소유하며 drop 시 분리되고, 취소 토큰은 워커가 임시 파일을 정리하게 한다.
        self.worker.take();
    }
}

/// UI와 CLI가 공유하는 파일 번역 실행자.
pub(crate) struct FileTransRunner;

impl FileTransRunner {
    pub(crate) fn start(mut job: FileTransJobData) -> FileTransTask {
        let (sender, events) = mpsc::channel();
        let cancel = CancelHandle(Arc::new(AtomicBool::new(false)));
        job.cancel_token = cancel.0.clone();
        let worker = std::thread::spawn(move || {
            run(&job, |event| {
                let _ = sender.send(event);
            })
        });
        FileTransTask {
            cancel,
            events,
            worker: Some(worker),
        }
    }
}

/// 출력 형식.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum WriteType {
    /// 번역만 기록한다.
    #[default]
    TranslationOnly,
    /// 원문 다음 줄에 번역을 기록한다.
    OriginalAndTrans,
    /// 원문과 번역 뒤에 빈 줄을 기록한다.
    OriginalTransNewline,
}

/// UI와 무관한 파일 번역 작업 설정.
pub(crate) struct FileTransJobData {
    pub input_files: Vec<PathBuf>,
    pub output_files: Vec<PathBuf>,
    pub write_type: WriteType,
    pub no_trans_linefeed: bool,
    pub cancel_token: Arc<AtomicBool>,
    pub engine: TranslationEngine,
    pub source_lang: Language,
    pub target_lang: Language,
    pub credentials: EngineCredentials,
    /// EzTrans 사용 시 필요한 DLL/DAT 경로. 다른 엔진에서는 빈 문자열이어도 무방하다.
    pub eztrans_dll_path: String,
    pub eztrans_dat_path: String,
}

/// 파일 번역 작업이 UI 또는 CLI 호출자에게 전달하는 진행 이벤트.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProgressEvent {
    TotalFiles(i32),
    TotalLines(i32),
    FileIndex(i32),
    FileName(String),
    FileLines(i32),
    FileProgress(i32),
    TotalProgress(i32),
    Complete,
    Error(String),
}

/// Windows 파일 시스템의 대소문자 비구분 규칙에 맞춰 비교할 절대 경로 키를 만든다.
fn normalized_path_key(path: &Path) -> Result<String, String> {
    let absolute = std::path::absolute(path).map_err(|e| {
        format!(
            "경로를 절대 경로로 변환할 수 없습니다: {}\n{e}",
            path.display()
        )
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
pub(crate) fn validate_job_paths(inputs: &[PathBuf], outputs: &[PathBuf]) -> Result<(), String> {
    if inputs.len() != outputs.len() {
        return Err("입력 파일과 출력 파일 수가 일치하지 않습니다.".to_string());
    }

    let input_keys = inputs
        .iter()
        .map(|path| normalized_path_key(path).map(|key| (key, path)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut output_keys = HashSet::with_capacity(outputs.len());

    for output in outputs {
        let output_key = normalized_path_key(output)?;
        if let Some((_, input)) = input_keys.iter().find(|(key, _)| key == &output_key) {
            return Err(format!(
                "출력 파일이 입력 파일과 같습니다.\n입력: {}\n출력: {}",
                input.display(),
                output.display()
            ));
        }
        if !output_keys.insert(output_key) {
            return Err(format!(
                "출력 파일 경로가 중복됩니다.\n{}",
                output.display()
            ));
        }
    }

    Ok(())
}

/// 입력 순서대로 충돌하지 않는 기본 출력 경로를 만든다.
pub(crate) fn default_output_paths(inputs: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
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
mod tests {
    use super::{CancelHandle, FileTransTask, default_output_paths, validate_job_paths};
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, mpsc};

    #[test]
    fn rejects_output_matching_input_after_normalization() {
        let input = PathBuf::from(r"C:\work\text\input.txt");
        let output = PathBuf::from(r"c:\WORK\text\.\input.txt");

        assert!(validate_job_paths(&[input], &[output]).is_err());
    }

    #[test]
    fn rejects_duplicate_outputs_after_normalization() {
        let inputs = [
            PathBuf::from(r"C:\work\a.txt"),
            PathBuf::from(r"C:\work\b.txt"),
        ];
        let outputs = [
            PathBuf::from(r"C:\work\result.txt"),
            PathBuf::from(r"c:\WORK\.\result.txt"),
        ];

        assert!(validate_job_paths(&inputs, &outputs).is_err());
    }

    #[test]
    fn makes_distinct_defaults_for_equal_stems() {
        let inputs = [
            PathBuf::from(r"C:\work\a.txt"),
            PathBuf::from(r"C:\work\a.log"),
        ];

        let outputs = default_output_paths(&inputs).unwrap();

        assert_eq!(outputs[0], PathBuf::from(r"C:\work\a_번역.txt"));
        assert_eq!(outputs[1], PathBuf::from(r"C:\work\a_번역_2.txt"));
        validate_job_paths(&inputs, &outputs).unwrap();
    }

    #[test]
    fn dropping_task_requests_cancellation_without_joining() {
        let token = Arc::new(AtomicBool::new(false));
        let (_sender, receiver) = mpsc::channel();
        let task = FileTransTask {
            cancel: CancelHandle(token.clone()),
            events: receiver,
            worker: None,
        };
        drop(task);
        assert!(token.load(std::sync::atomic::Ordering::SeqCst));
    }
}
