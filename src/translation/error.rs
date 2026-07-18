use super::language::Language;
use thiserror::Error;

/// 번역 에러 타입
#[derive(Debug, Clone, Error)]
pub enum TranslationError {
    #[error("빈 텍스트입니다.")]
    EmptyText,

    #[error("엔진이 초기화되지 않았습니다: {0}")]
    EngineNotInitialized(&'static str),

    #[error("지원하지 않는 언어 쌍입니다.")]
    UnsupportedLanguagePair,

    #[error("{engine} 엔진이 지원하지 않는 언어입니다: {language:?}")]
    UnsupportedLanguage {
        engine: &'static str,
        language: Language,
    },

    #[error("API 키가 설정되지 않았습니다.")]
    MissingApiKey,

    #[error("네트워크 오류: {0}")]
    Network(String),

    #[error("HTTP 응답이 허용 크기({limit}바이트)를 초과했습니다.")]
    ResponseTooLarge { limit: usize },

    #[error("API 오류 ({code}): {message}")]
    Api {
        code: u16,
        message: String,
        retry_after: Option<std::time::Duration>,
    },

    #[error("{provider} 응답이 출력 한도에 도달해 절단되었습니다 ({reason}).")]
    OutputTruncated {
        provider: &'static str,
        reason: String,
    },

    #[error("API 사용량 제한 ({code}): {message}")]
    RateLimited {
        code: u16,
        message: String,
        retry_after: Option<std::time::Duration>,
    },

    #[error("응답 파싱 실패: {0}")]
    Parse(String),

    #[error("엔진 오류: {0}")]
    Engine(String),

    #[error("{engine} 입력이 허용 길이를 초과했습니다 ({length}/{max}자).")]
    InputTooLong {
        engine: &'static str,
        length: usize,
        max: usize,
    },
}

impl TranslationError {
    /// 재시도 가능한 에러인지 판별
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Network(_) => true,
            Self::Api { code, .. } => matches!(code, 429 | 500 | 502 | 503 | 504 | 529),
            Self::RateLimited { .. } => true,
            _ => false,
        }
    }

    /// 로그에 원문, 응답 본문, 자격증명을 싣지 않는 안정적인 오류 분류.
    pub fn log_category(&self) -> &'static str {
        match self {
            Self::EmptyText => "empty_text",
            Self::EngineNotInitialized(_) => "engine_not_initialized",
            Self::UnsupportedLanguagePair => "unsupported_language_pair",
            Self::UnsupportedLanguage { .. } => "unsupported_language",
            Self::MissingApiKey => "missing_api_key",
            Self::Network(_) => "network",
            Self::ResponseTooLarge { .. } => "response_too_large",
            Self::Api { .. } | Self::RateLimited { .. } => "api",
            Self::OutputTruncated { .. } => "output_truncated",
            Self::Parse(_) => "parse",
            Self::Engine(_) => "engine",
            Self::InputTooLong { .. } => "input_too_long",
        }
    }

    pub fn log_status_code(&self) -> Option<u16> {
        match self {
            Self::Api { code, .. } | Self::RateLimited { code, .. } if *code != 0 => Some(*code),
            _ => None,
        }
    }

    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Self::Api { retry_after, .. } | Self::RateLimited { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

/// 번역 결과 타입
pub type TranslationResult = Result<String, TranslationError>;
