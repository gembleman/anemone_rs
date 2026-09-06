use std::path::PathBuf;

/// 파일 번역의 실패 정책을 호출자가 문자열 분석 없이 구분할 수 있는 오류.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FileTranslationError {
    #[error("사용자가 취소했습니다.")]
    Cancelled,
    #[error("잘못된 파일 번역 요청: {0}")]
    InvalidRequest(String),
    #[error("파일 경로 오류: {0}")]
    Path(String),
    #[error("입력 파일 오류: {0}")]
    Input(String),
    #[error("입력 인코딩 오류: {0}")]
    Encoding(String),
    #[error("출력 파일 오류: {0}")]
    Output(String),
    #[error("파일 번역 실행기 오류: {0}")]
    Runtime(String),
    #[error("파일 번역 backend 오류: {0}")]
    Backend(String),
    #[error("입력 파일의 한 줄이 허용 크기({limit}바이트)를 초과했습니다: {path}")]
    LineTooLong { path: PathBuf, limit: usize },
    #[error("입력 파일의 전체 줄 수가 너무 많습니다.")]
    TooManyLines,
}

impl FileTranslationError {
    pub(crate) fn path(message: impl Into<String>) -> Self {
        Self::Path(message.into())
    }

    pub(crate) fn input(message: impl Into<String>) -> Self {
        Self::Input(message.into())
    }

    pub(crate) fn encoding(message: impl Into<String>) -> Self {
        Self::Encoding(message.into())
    }

    pub(crate) fn output(message: impl Into<String>) -> Self {
        Self::Output(message.into())
    }

    pub(crate) fn backend(message: impl Into<String>) -> Self {
        Self::Backend(message.into())
    }
}

#[cfg(test)]
#[path = "../../tests/unit/file_trans/error.rs"]
mod tests;
