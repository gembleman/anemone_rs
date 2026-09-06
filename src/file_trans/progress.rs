use super::FileTranslationError;

/// 성공한 파일 번역 작업의 최종 집계.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileTranslationSummary {
    pub total_files: usize,
    pub total_lines: usize,
}

/// 파일 번역 작업이 UI 또는 CLI 호출자에게 전달하는 진행 이벤트.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileTranslationProgress {
    TotalFiles(i32),
    TotalLines(i32),
    FileIndex(i32),
    FileName(String),
    FileLines(i32),
    FileProgress(i32),
    TotalProgress(i32),
    /// 작업당 정확히 한 번 전달되는 terminal 결과.
    Finished(Result<FileTranslationSummary, FileTranslationError>),
}

impl FileTranslationProgress {
    /// terminal 이벤트까지 수집하는 테스트·벤치 하네스만 쓴다.
    #[cfg(any(test, feature = "benchmark"))]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Finished(_))
    }
}
