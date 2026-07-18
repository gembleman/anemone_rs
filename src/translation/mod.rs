//! 번역 엔진 모듈
//!
//! 지원 엔진:
//! - EzTrans (eztrans-rs 라이브러리 사용)
//! - Google Translate (HTTPS API)
//! - DeepL (HTTPS API)
//! - Papago (Naver HTTPS API)
//!
//! 비동기 번역 지원:
//! - `worker::TranslationDispatch`: 프로세스 단일 워커 스레드 + tokio 런타임
//! - 호출자별 불투명 대상 라우팅. 완료 통지 방식은 UI 어댑터가 제공

pub mod deepl;
mod eztrans;
pub mod google;
pub(crate) mod http_common;
mod job;
pub mod llm;
pub mod manual;
pub mod papago;
pub mod settings;
pub mod worker;

pub use eztrans::EzTransTranslator;
pub use job::TranslationJobSpec;
pub use llm::LlmProvider;
pub use worker::EngineCredentials;

use std::path::Path;
use std::sync::{OnceLock, mpsc};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("알 수 없는 {kind} 값: {value}")]
pub struct EnumParseError {
    kind: &'static str,
    value: String,
}

impl EnumParseError {
    fn new(kind: &'static str, value: &str) -> Self {
        Self {
            kind,
            value: value.to_string(),
        }
    }
}

/// 번역 API에 전달할 언어 태그.
///
/// `isolang::Language`와 달리 중국어 문자 체계를 보존한다. 설정/CLI/UI에서 선택한
/// `zh-CN`과 `zh-TW`가 제공자 어댑터까지 손실 없이 전달된다.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Language {
    Jpn,
    Kor,
    Eng,
    ZhoHans,
    ZhoHant,
    Spa,
    Fra,
    Deu,
    Ita,
    Por,
    Rus,
    Ara,
    Hin,
    Tha,
    Vie,
    Ind,
    Msa,
    Nld,
    Pol,
    Tur,
    Ukr,
}

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

/// 번역 엔진 종류
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TranslationEngine {
    #[default]
    EzTrans = 0,
    Google = 1,
    DeepL = 2,
    Papago = 3,
    /// LLM 기반 번역 (제공자/모델은 LlmConfig에서 지정)
    Llm = 4,
}

impl TranslationEngine {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::EzTrans),
            1 => Some(Self::Google),
            2 => Some(Self::DeepL),
            3 => Some(Self::Papago),
            4 => Some(Self::Llm),
            _ => None,
        }
    }

    pub fn to_str(self) -> &'static str {
        match self {
            Self::EzTrans => "eztrans",
            Self::Google => "google",
            Self::DeepL => "deepl",
            Self::Papago => "papago",
            Self::Llm => "llm",
        }
    }

    /// 해당 엔진이 지원하는 소스 언어 목록
    pub fn supported_source_languages(&self) -> &'static [Language] {
        match self {
            Self::EzTrans => &[Language::Jpn], // 일본어만
            Self::Google => GOOGLE_SUPPORTED_LANGUAGES,
            Self::DeepL => DEEPL_SUPPORTED_LANGUAGES,
            Self::Papago => PAPAGO_SUPPORTED_LANGUAGES,
            // LLM은 프롬프트 기반이라 대부분의 언어 지원. Google 목록을 재사용.
            Self::Llm => GOOGLE_SUPPORTED_LANGUAGES,
        }
    }

    /// 해당 엔진이 지원하는 타겟 언어 목록
    pub fn supported_target_languages(&self) -> &'static [Language] {
        match self {
            Self::EzTrans => &[Language::Kor], // 한국어만
            Self::Google => GOOGLE_SUPPORTED_LANGUAGES,
            Self::DeepL => DEEPL_SUPPORTED_LANGUAGES,
            Self::Papago => PAPAGO_SUPPORTED_LANGUAGES,
            Self::Llm => GOOGLE_SUPPORTED_LANGUAGES,
        }
    }

    /// 제공자 계약상 실제로 번역 가능한 언어 방향인지 검사한다.
    pub fn supports_pair(&self, source: Language, target: Language) -> bool {
        if source == target
            || !self.supported_source_languages().contains(&source)
            || !self.supported_target_languages().contains(&target)
        {
            return false;
        }

        match self {
            Self::Papago => papago_supports_pair(source, target),
            _ => true,
        }
    }

    /// 단일 요청의 보수적인 문자 수 상한. 긴 파일은 호출자가 이 경계로 나눠야 한다.
    pub fn max_input_chars(self) -> usize {
        match self {
            Self::Google | Self::Papago => 5_000,
            Self::DeepL => 100_000,
            Self::Llm => 100_000,
            Self::EzTrans => 100_000,
        }
    }

    pub fn supported_targets_for(self, source: Language) -> Vec<Language> {
        self.supported_target_languages()
            .iter()
            .copied()
            .filter(|&target| self.supports_pair(source, target))
            .collect()
    }
}

impl std::str::FromStr for TranslationEngine {
    type Err = EnumParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "eztrans" => Ok(Self::EzTrans),
            "google" => Ok(Self::Google),
            "deepl" => Ok(Self::DeepL),
            "papago" => Ok(Self::Papago),
            "llm" => Ok(Self::Llm),
            _ => Err(EnumParseError::new("번역 엔진", value)),
        }
    }
}

/// Ncloud Papago Text Translation의 29개 지원 언어쌍.
/// 공식 표가 양방향 쌍으로 정의되어 있으므로 순서를 정규화하지 않고 대칭 검사한다.
fn papago_supports_pair(source: Language, target: Language) -> bool {
    let paired = |anchor, others: &[Language]| {
        (source == anchor && others.contains(&target))
            || (target == anchor && others.contains(&source))
    };

    paired(
        Language::Kor,
        &[
            Language::Eng,
            Language::Jpn,
            Language::ZhoHans,
            Language::ZhoHant,
            Language::Vie,
            Language::Tha,
            Language::Ind,
            Language::Fra,
            Language::Spa,
            Language::Rus,
            Language::Deu,
            Language::Ita,
        ],
    ) || paired(
        Language::Eng,
        &[
            Language::Jpn,
            Language::ZhoHans,
            Language::ZhoHant,
            Language::Vie,
            Language::Tha,
            Language::Ind,
            Language::Fra,
            Language::Spa,
            Language::Rus,
            Language::Deu,
        ],
    ) || paired(
        Language::Jpn,
        &[
            Language::ZhoHans,
            Language::ZhoHant,
            Language::Vie,
            Language::Tha,
            Language::Ind,
            Language::Fra,
        ],
    ) || matches!(
        (source, target),
        (Language::ZhoHans, Language::ZhoHant) | (Language::ZhoHant, Language::ZhoHans)
    )
}

/// Google Translate 지원 언어 (주요 언어)
pub static GOOGLE_SUPPORTED_LANGUAGES: &[Language] = &[
    Language::Jpn,     // 일본어
    Language::Kor,     // 한국어
    Language::Eng,     // 영어
    Language::ZhoHans, // 중국어 간체
    Language::ZhoHant, // 중국어 번체
    Language::Spa,     // 스페인어
    Language::Fra,     // 프랑스어
    Language::Deu,     // 독일어
    Language::Ita,     // 이탈리아어
    Language::Por,     // 포르투갈어
    Language::Rus,     // 러시아어
    Language::Ara,     // 아랍어
    Language::Hin,     // 힌디어
    Language::Tha,     // 태국어
    Language::Vie,     // 베트남어
    Language::Ind,     // 인도네시아어
    Language::Msa,     // 말레이어
    Language::Nld,     // 네덜란드어
    Language::Pol,     // 폴란드어
    Language::Tur,     // 터키어
    Language::Ukr,     // 우크라이나어
];

/// DeepL 지원 언어
pub static DEEPL_SUPPORTED_LANGUAGES: &[Language] = &[
    Language::Jpn,     // 일본어
    Language::Kor,     // 한국어
    Language::Eng,     // 영어
    Language::ZhoHans, // 중국어 간체
    Language::ZhoHant, // 중국어 번체
    Language::Spa,     // 스페인어
    Language::Fra,     // 프랑스어
    Language::Deu,     // 독일어
    Language::Ita,     // 이탈리아어
    Language::Por,     // 포르투갈어
    Language::Rus,     // 러시아어
    Language::Nld,     // 네덜란드어
    Language::Pol,     // 폴란드어
    Language::Tur,     // 터키어
    Language::Ukr,     // 우크라이나어
];

/// Ncloud Papago Text Translation 지원 언어
pub static PAPAGO_SUPPORTED_LANGUAGES: &[Language] = &[
    Language::Kor,     // 한국어
    Language::Eng,     // 영어
    Language::Jpn,     // 일본어
    Language::ZhoHans, // 중국어 간체
    Language::ZhoHant, // 중국어 번체
    Language::Vie,     // 베트남어
    Language::Tha,     // 태국어
    Language::Ind,     // 인도네시아어
    Language::Fra,     // 프랑스어
    Language::Spa,     // 스페인어
    Language::Rus,     // 러시아어
    Language::Deu,     // 독일어
    Language::Ita,     // 이탈리아어
];

/// 언어 코드 헬퍼 함수들
pub mod lang_utils {
    use super::{Language, TranslationError};

    /// ISO 639-1 코드로 Language 가져오기 (예: "ja", "ko", "en")
    pub fn from_code(code: &str) -> Option<Language> {
        match code.to_lowercase().as_str() {
            "ja" | "jpn" => Some(Language::Jpn),
            "ko" | "kor" => Some(Language::Kor),
            "en" | "eng" => Some(Language::Eng),
            "zh" | "zho" | "chi" | "zh-cn" | "zh-hans" | "zhs" => Some(Language::ZhoHans),
            "zh-tw" | "zh-hant" | "zht" => Some(Language::ZhoHant),
            "es" | "spa" => Some(Language::Spa),
            "fr" | "fra" | "fre" => Some(Language::Fra),
            "de" | "deu" | "ger" => Some(Language::Deu),
            "it" | "ita" => Some(Language::Ita),
            "pt" | "por" => Some(Language::Por),
            "ru" | "rus" => Some(Language::Rus),
            "ar" | "ara" => Some(Language::Ara),
            "hi" | "hin" => Some(Language::Hin),
            "th" | "tha" => Some(Language::Tha),
            "vi" | "vie" => Some(Language::Vie),
            "id" | "ind" => Some(Language::Ind),
            "ms" | "msa" | "may" => Some(Language::Msa),
            "nl" | "nld" | "dut" => Some(Language::Nld),
            "pl" | "pol" => Some(Language::Pol),
            "tr" | "tur" => Some(Language::Tur),
            "uk" | "ukr" => Some(Language::Ukr),
            _ => None,
        }
    }

    /// Language를 ISO 639-1 코드로 변환
    pub fn to_code(lang: Language) -> &'static str {
        match lang {
            Language::Jpn => "ja",
            Language::Kor => "ko",
            Language::Eng => "en",
            Language::ZhoHans => "zh-CN",
            Language::ZhoHant => "zh-TW",
            Language::Spa => "es",
            Language::Fra => "fr",
            Language::Deu => "de",
            Language::Ita => "it",
            Language::Por => "pt",
            Language::Rus => "ru",
            Language::Ara => "ar",
            Language::Hin => "hi",
            Language::Tha => "th",
            Language::Vie => "vi",
            Language::Ind => "id",
            Language::Msa => "ms",
            Language::Nld => "nl",
            Language::Pol => "pl",
            Language::Tur => "tr",
            Language::Ukr => "uk",
        }
    }

    /// Language를 한국어 이름으로 변환
    pub fn to_korean_name(lang: Language) -> &'static str {
        match lang {
            Language::Jpn => "일본어",
            Language::Kor => "한국어",
            Language::Eng => "영어",
            Language::ZhoHans => "중국어(간체)",
            Language::ZhoHant => "중국어(번체)",
            Language::Spa => "스페인어",
            Language::Fra => "프랑스어",
            Language::Deu => "독일어",
            Language::Ita => "이탈리아어",
            Language::Por => "포르투갈어",
            Language::Rus => "러시아어",
            Language::Ara => "아랍어",
            Language::Hin => "힌디어",
            Language::Tha => "태국어",
            Language::Vie => "베트남어",
            Language::Ind => "인도네시아어",
            Language::Msa => "말레이어",
            Language::Nld => "네덜란드어",
            Language::Pol => "폴란드어",
            Language::Tur => "터키어",
            Language::Ukr => "우크라이나어",
        }
    }

    /// Google Translate API 언어 코드로 변환
    pub fn to_google_code(lang: Language) -> Result<&'static str, TranslationError> {
        Ok(match lang {
            Language::ZhoHans => "zh-CN",
            Language::ZhoHant => "zh-TW",
            _ => to_code(lang),
        })
    }

    /// DeepL API 언어 코드로 변환
    pub fn to_deepl_code(lang: Language) -> Result<&'static str, TranslationError> {
        match lang {
            Language::Jpn => Ok("JA"),
            Language::Kor => Ok("KO"),
            Language::Eng => Ok("EN"),
            Language::ZhoHans => Ok("ZH-HANS"),
            Language::ZhoHant => Ok("ZH-HANT"),
            Language::Spa => Ok("ES"),
            Language::Fra => Ok("FR"),
            Language::Deu => Ok("DE"),
            Language::Ita => Ok("IT"),
            Language::Por => Ok("PT"),
            Language::Rus => Ok("RU"),
            Language::Nld => Ok("NL"),
            Language::Pol => Ok("PL"),
            Language::Tur => Ok("TR"),
            Language::Ukr => Ok("UK"),
            Language::Ara
            | Language::Hin
            | Language::Tha
            | Language::Vie
            | Language::Ind
            | Language::Msa => Err(TranslationError::UnsupportedLanguage {
                engine: "DeepL",
                language: lang,
            }),
        }
    }

    /// Papago API 언어 코드로 변환
    pub fn to_papago_code(lang: Language) -> Result<&'static str, TranslationError> {
        match lang {
            Language::Kor => Ok("ko"),
            Language::Eng => Ok("en"),
            Language::Jpn => Ok("ja"),
            Language::ZhoHans => Ok("zh-CN"),
            Language::ZhoHant => Ok("zh-TW"),
            Language::Vie => Ok("vi"),
            Language::Tha => Ok("th"),
            Language::Ind => Ok("id"),
            Language::Fra => Ok("fr"),
            Language::Spa => Ok("es"),
            Language::Rus => Ok("ru"),
            Language::Deu => Ok("de"),
            Language::Ita => Ok("it"),
            Language::Ara
            | Language::Hin
            | Language::Por
            | Language::Msa
            | Language::Nld
            | Language::Pol
            | Language::Tur
            | Language::Ukr => Err(TranslationError::UnsupportedLanguage {
                engine: "Papago",
                language: lang,
            }),
        }
    }
}

/// 번역 결과 타입
pub type TranslationResult = Result<String, TranslationError>;

/// EzTrans 인스턴스 보관용 글로벌 매니저.
///
/// EzTrans 는 외부 32-bit DLL 을 mmap 하는 무거운 객체라서 프로세스당 하나만
/// 유지한다. 다른 엔진(Google/DeepL/Papago/LLM)은 stateless 한 HTTP 호출이라
/// 보관할 필요가 없다.
struct EzTransState {
    engine: Option<EzTransTranslator>,
    /// 현재 로드된 엔진의 (dll_path, dat_path). 동일하면 재로드 스킵, 다르면 폐기 후 재로드.
    loaded_paths: Option<(String, String)>,
    /// `SetDefaultDllDirectories(... USER_DIRS)` 환경에서 EzTrans DLL 의 같은 폴더
    /// 의존성을 찾기 위해 등록한 DLL 검색 폴더들.
    registered_dll_dirs: Vec<String>,
}

impl EzTransState {
    fn new() -> Self {
        Self {
            engine: None,
            loaded_paths: None,
            registered_dll_dirs: Vec::new(),
        }
    }

    /// EzTrans 초기화.
    ///
    /// 동일한 경로로 이미 초기화되어 있으면 스킵. 경로가 바뀌었으면 기존 엔진을
    /// 폐기하고 새 경로로 재로드한다. 사용자가 설정 다이얼로그에서 dll/dat 경로를
    /// 바꿔도 즉시 반영되도록 하기 위함.
    pub fn init(&mut self, dll_path: &str, dat_path: &str) -> Result<(), String> {
        if let Some((loaded_dll, loaded_dat)) = &self.loaded_paths {
            if loaded_dll == dll_path && loaded_dat == dat_path && self.engine.is_some() {
                return Ok(());
            }
            // 경로가 바뀌었거나 엔진이 사라진 상태 — 기존 인스턴스 폐기.
            self.engine = None;
            self.loaded_paths = None;
        }
        self.ensure_dll_directory_registered(dll_path)?;
        let engine = EzTransTranslator::new(dll_path, dat_path)?;
        self.engine = Some(engine);
        self.loaded_paths = Some((dll_path.to_string(), dat_path.to_string()));
        Ok(())
    }

    fn ensure_dll_directory_registered(&mut self, dll_path: &str) -> Result<(), String> {
        let dir = eztrans_dll_search_dir(dll_path)?;
        if self.registered_dll_dirs.iter().any(|d| d == &dir) {
            return Ok(());
        }

        let wide: Vec<u16> = dir.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is a null-terminated UTF-16 string valid for this call.
        // Windows copies the directory path into the process DLL directory list.
        unsafe {
            let cookie = windows::Win32::System::LibraryLoader::AddDllDirectory(
                windows::core::PCWSTR(wide.as_ptr()),
            );
            if cookie.is_null() {
                let err = windows::Win32::Foundation::GetLastError();
                return Err(format!("EzTrans DLL 폴더 등록 실패: Win32 {}", err.0));
            }
        }
        self.registered_dll_dirs.push(dir);
        Ok(())
    }
}

fn eztrans_dll_search_dir(dll_path: &str) -> Result<String, String> {
    let path = Path::new(dll_path);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "EzTrans DLL 폴더를 확인할 수 없습니다.".to_string())?;
    let dir = if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("현재 폴더 확인 실패: {e}"))?
            .join(parent)
    };
    dir.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "EzTrans DLL 폴더 경로가 UTF-8 이 아닙니다.".to_string())
}

enum EzTransCommand {
    Init {
        dll_path: String,
        dat_path: String,
        response: mpsc::Sender<Result<(), String>>,
    },
    Translate {
        text: String,
        source: Language,
        target: Language,
        response: mpsc::Sender<TranslationResult>,
    },
}

struct EzTransActor {
    sender: mpsc::Sender<EzTransCommand>,
}

impl EzTransActor {
    fn spawn() -> Self {
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("anemone-eztrans".into())
            .spawn(move || {
                // DLL 객체는 이 스레드 안에서 생성되고 여기서만 호출/해제된다.
                let mut state = EzTransState::new();
                while let Ok(command) = receiver.recv() {
                    match command {
                        EzTransCommand::Init {
                            dll_path,
                            dat_path,
                            response,
                        } => {
                            let _ = response.send(state.init(&dll_path, &dat_path));
                        }
                        EzTransCommand::Translate {
                            text,
                            source,
                            target,
                            response,
                        } => {
                            let result = state
                                .engine
                                .as_ref()
                                .ok_or(TranslationError::EngineNotInitialized("EzTrans"))
                                .and_then(|engine| engine.translate(&text, source, target));
                            let _ = response.send(result);
                        }
                    }
                }
            })
            .expect("EzTrans actor thread spawn failed");
        Self { sender }
    }

    fn init(&self, dll_path: &str, dat_path: &str) -> Result<(), String> {
        let (response, receiver) = mpsc::channel();
        self.sender
            .send(EzTransCommand::Init {
                dll_path: dll_path.to_string(),
                dat_path: dat_path.to_string(),
                response,
            })
            .map_err(|_| "EzTrans 전용 스레드가 종료되었습니다".to_string())?;
        receiver
            .recv()
            .map_err(|_| "EzTrans 초기화 응답을 받지 못했습니다".to_string())?
    }

    fn translate(&self, text: &str, source: Language, target: Language) -> TranslationResult {
        let (response, receiver) = mpsc::channel();
        self.sender
            .send(EzTransCommand::Translate {
                text: text.to_string(),
                source,
                target,
                response,
            })
            .map_err(|_| TranslationError::Engine("EzTrans 전용 스레드가 종료되었습니다".into()))?;
        receiver
            .recv()
            .map_err(|_| TranslationError::Engine("EzTrans 번역 응답을 받지 못했습니다".into()))?
    }
}

static EZTRANS_ACTOR: OnceLock<EzTransActor> = OnceLock::new();

fn eztrans_actor() -> &'static EzTransActor {
    EZTRANS_ACTOR.get_or_init(EzTransActor::spawn)
}

pub fn prepare_eztrans(dll_path: &str, dat_path: &str) -> Result<(), String> {
    eztrans_actor().init(dll_path, dat_path)
}

/// EzTrans 로 번역 수행 (워커 스레드에서 호출용).
///
/// 글로벌 매니저의 EzTrans 인스턴스를 사용한다. 초기화되어 있지 않으면
/// `EngineNotInitialized` 를 돌려준다.
pub fn translate_with_eztrans(text: &str, source: Language, target: Language) -> TranslationResult {
    eztrans_actor().translate(text, source, target)
}
