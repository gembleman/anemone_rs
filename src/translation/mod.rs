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
//! - 호출자별 hwnd 라우팅. Windows 메시지로 결과 전달 (UI 블로킹 없음)

pub mod deepl;
mod eztrans;
pub mod google;
pub(crate) mod http_common;
mod job;
pub mod llm;
pub mod papago;
pub mod settings;
pub mod worker;

pub use eztrans::EzTransTranslator;
pub use isolang::Language;
pub use job::TranslationJobSpec;
pub use llm::LlmProvider;
pub use worker::{
    EngineCredentials, shutdown, take_response, translate as request_translation,
    unregister_hwnd as unregister_translation_hwnd,
};

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
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

    #[error("API 키가 설정되지 않았습니다.")]
    MissingApiKey,

    #[error("네트워크 오류: {0}")]
    Network(String),

    #[error("API 오류 ({code}): {message}")]
    Api { code: u16, message: String },

    #[error("응답 파싱 실패: {0}")]
    Parse(String),

    #[error("엔진 오류: {0}")]
    Engine(String),
}

impl TranslationError {
    /// 재시도 가능한 에러인지 판별
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Network(_) => true,
            Self::Api { code, .. } => matches!(code, 429 | 500 | 502 | 503 | 504),
            _ => false,
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
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::EzTrans,
            1 => Self::Google,
            2 => Self::DeepL,
            3 => Self::Papago,
            4 => Self::Llm,
            _ => Self::EzTrans,
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "eztrans" => Self::EzTrans,
            "google" => Self::Google,
            "deepl" => Self::DeepL,
            "papago" => Self::Papago,
            "llm" => Self::Llm,
            _ => Self::EzTrans,
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
}

/// Google Translate 지원 언어 (주요 언어)
pub static GOOGLE_SUPPORTED_LANGUAGES: &[Language] = &[
    Language::Jpn, // 일본어
    Language::Kor, // 한국어
    Language::Eng, // 영어
    Language::Zho, // 중국어
    Language::Spa, // 스페인어
    Language::Fra, // 프랑스어
    Language::Deu, // 독일어
    Language::Ita, // 이탈리아어
    Language::Por, // 포르투갈어
    Language::Rus, // 러시아어
    Language::Ara, // 아랍어
    Language::Hin, // 힌디어
    Language::Tha, // 태국어
    Language::Vie, // 베트남어
    Language::Ind, // 인도네시아어
    Language::Msa, // 말레이어
    Language::Nld, // 네덜란드어
    Language::Pol, // 폴란드어
    Language::Tur, // 터키어
    Language::Ukr, // 우크라이나어
];

/// DeepL 지원 언어
pub static DEEPL_SUPPORTED_LANGUAGES: &[Language] = &[
    Language::Jpn, // 일본어
    Language::Kor, // 한국어
    Language::Eng, // 영어
    Language::Zho, // 중국어
    Language::Spa, // 스페인어
    Language::Fra, // 프랑스어
    Language::Deu, // 독일어
    Language::Ita, // 이탈리아어
    Language::Por, // 포르투갈어
    Language::Rus, // 러시아어
    Language::Nld, // 네덜란드어
    Language::Pol, // 폴란드어
    Language::Tur, // 터키어
    Language::Ukr, // 우크라이나어
];

/// Papago 지원 언어 (네이버 N2MT 기준)
pub static PAPAGO_SUPPORTED_LANGUAGES: &[Language] = &[
    Language::Kor, // 한국어
    Language::Eng, // 영어
    Language::Jpn, // 일본어
    Language::Zho, // 중국어 (간체/번체)
    Language::Vie, // 베트남어
    Language::Tha, // 태국어
    Language::Ind, // 인도네시아어
    Language::Fra, // 프랑스어
    Language::Spa, // 스페인어
    Language::Rus, // 러시아어
    Language::Deu, // 독일어
    Language::Ita, // 이탈리아어
    Language::Por, // 포르투갈어
];

/// 언어 코드 헬퍼 함수들
pub mod lang_utils {
    use isolang::Language;

    /// ISO 639-1 코드로 Language 가져오기 (예: "ja", "ko", "en")
    pub fn from_code(code: &str) -> Option<Language> {
        // 먼저 639-1 (2글자) 시도
        if let Some(lang) = Language::from_639_1(code) {
            return Some(lang);
        }
        // 639-3 (3글자) 시도
        if let Some(lang) = Language::from_639_3(code) {
            return Some(lang);
        }
        // 특수 케이스 처리
        match code.to_lowercase().as_str() {
            "zh-cn" | "zh-hans" | "zhs" => Some(Language::Zho),
            "zh-tw" | "zh-hant" | "zht" => Some(Language::Zho), // 번체도 Zho로 매핑
            _ => None,
        }
    }

    /// Language를 ISO 639-1 코드로 변환
    pub fn to_code(lang: Language) -> &'static str {
        lang.to_639_1().unwrap_or(lang.to_639_3())
    }

    /// Language를 한국어 이름으로 변환
    pub fn to_korean_name(lang: Language) -> &'static str {
        match lang {
            Language::Jpn => "일본어",
            Language::Kor => "한국어",
            Language::Eng => "영어",
            Language::Zho => "중국어",
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
            _ => lang.to_name(),
        }
    }

    /// Google Translate API 언어 코드로 변환
    pub fn to_google_code(lang: Language) -> &'static str {
        match lang {
            Language::Zho => "zh-CN", // 중국어 간체 기본
            _ => lang.to_639_1().unwrap_or("en"),
        }
    }

    /// DeepL API 언어 코드로 변환
    pub fn to_deepl_code(lang: Language) -> &'static str {
        match lang {
            Language::Jpn => "JA",
            Language::Kor => "KO",
            Language::Eng => "EN",
            Language::Zho => "ZH",
            Language::Spa => "ES",
            Language::Fra => "FR",
            Language::Deu => "DE",
            Language::Ita => "IT",
            Language::Por => "PT",
            Language::Rus => "RU",
            Language::Nld => "NL",
            Language::Pol => "PL",
            Language::Tur => "TR",
            Language::Ukr => "UK",
            _ => "EN",
        }
    }

    /// Papago API 언어 코드로 변환
    pub fn to_papago_code(lang: Language) -> &'static str {
        match lang {
            Language::Kor => "ko",
            Language::Eng => "en",
            Language::Jpn => "ja",
            Language::Zho => "zh-CN", // 간체 기본
            Language::Vie => "vi",
            Language::Tha => "th",
            Language::Ind => "id",
            Language::Fra => "fr",
            Language::Spa => "es",
            Language::Rus => "ru",
            Language::Deu => "de",
            Language::Ita => "it",
            Language::Por => "pt",
            _ => "en",
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
pub struct EzTransManager {
    engine: Option<EzTransTranslator>,
    /// 현재 로드된 엔진의 (dll_path, dat_path). 동일하면 재로드 스킵, 다르면 폐기 후 재로드.
    loaded_paths: Option<(String, String)>,
    /// `SetDefaultDllDirectories(... USER_DIRS)` 환경에서 EzTrans DLL 의 같은 폴더
    /// 의존성을 찾기 위해 등록한 DLL 검색 폴더들.
    registered_dll_dirs: Vec<String>,
}

impl EzTransManager {
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

static EZTRANS_MANAGER: OnceLock<Arc<Mutex<EzTransManager>>> = OnceLock::new();

/// EzTrans 매니저 (필요 시 초기화) 가져오기
pub fn get_eztrans_manager() -> Arc<Mutex<EzTransManager>> {
    EZTRANS_MANAGER
        .get_or_init(|| Arc::new(Mutex::new(EzTransManager::new())))
        .clone()
}

/// EzTrans 로 번역 수행 (워커 스레드에서 호출용).
///
/// 글로벌 매니저의 EzTrans 인스턴스를 사용한다. 초기화되어 있지 않으면
/// `EngineNotInitialized` 를 돌려준다.
pub fn translate_with_eztrans(text: &str, source: Language, target: Language) -> TranslationResult {
    let manager = get_eztrans_manager();
    if let Ok(mgr) = manager.lock()
        && let Some(ref engine) = mgr.engine
    {
        return engine.translate(text, source, target);
    }
    Err(TranslationError::EngineNotInitialized("EzTrans"))
}
