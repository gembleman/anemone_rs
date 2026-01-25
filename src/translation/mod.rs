//! 번역 엔진 모듈
//!
//! 지원 엔진:
//! - EzTrans (eztrans-rs 라이브러리 사용)
//! - Google Translate (HTTP API)
//! - DeepL (HTTP API)

mod deepl;
mod detect;
mod eztrans;
mod google;

pub use deepl::DeepLTranslator;
pub use detect::{detect_language, is_source_language};
pub use eztrans::EzTransTranslator;
pub use google::GoogleTranslator;
pub use isolang::Language;

use std::sync::{Arc, Mutex, OnceLock};

/// 번역 엔진 종류
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TranslationEngine {
    #[default]
    EzTrans = 0,
    Google = 1,
    DeepL = 2,
}

impl TranslationEngine {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::EzTrans,
            1 => Self::Google,
            2 => Self::DeepL,
            _ => Self::EzTrans,
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "eztrans" => Self::EzTrans,
            "google" => Self::Google,
            "deepl" => Self::DeepL,
            _ => Self::EzTrans,
        }
    }

    pub fn to_str(&self) -> &'static str {
        match self {
            Self::EzTrans => "eztrans",
            Self::Google => "google",
            Self::DeepL => "deepl",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::EzTrans => "EzTrans",
            Self::Google => "Google",
            Self::DeepL => "DeepL",
        }
    }

    /// 해당 엔진이 지원하는 소스 언어 목록
    pub fn supported_source_languages(&self) -> &'static [Language] {
        match self {
            Self::EzTrans => &[Language::Jpn], // 일본어만
            Self::Google => &GOOGLE_SUPPORTED_LANGUAGES,
            Self::DeepL => &DEEPL_SUPPORTED_LANGUAGES,
        }
    }

    /// 해당 엔진이 지원하는 타겟 언어 목록
    pub fn supported_target_languages(&self) -> &'static [Language] {
        match self {
            Self::EzTrans => &[Language::Kor], // 한국어만
            Self::Google => &GOOGLE_SUPPORTED_LANGUAGES,
            Self::DeepL => &DEEPL_SUPPORTED_LANGUAGES,
        }
    }

    /// 엔진이 해당 언어 쌍을 지원하는지 확인
    pub fn supports_language_pair(&self, source: Language, target: Language) -> bool {
        self.supported_source_languages().contains(&source)
            && self.supported_target_languages().contains(&target)
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

    /// 기본 언어 (일본어)
    pub fn default_source() -> Language {
        Language::Jpn
    }

    /// 기본 타겟 언어 (한국어)
    pub fn default_target() -> Language {
        Language::Kor
    }
}

/// 번역 결과
#[derive(Debug)]
pub enum TranslationResult {
    Success(String),
    Error(String),
}

/// 번역 인터페이스
pub trait Translator: Send + Sync {
    /// 번역 수행
    fn translate(&self, text: &str, source: Language, target: Language) -> TranslationResult;

    /// 엔진 이름
    fn engine_name(&self) -> &'static str;

    /// 사용 가능 여부
    fn is_available(&self) -> bool;
}

/// 글로벌 번역 매니저
pub struct TranslationManager {
    eztrans: Option<EzTransTranslator>,
    google: GoogleTranslator,
    deepl: DeepLTranslator,
    current_engine: TranslationEngine,
    source_lang: Language,
    target_lang: Language,
}

impl TranslationManager {
    /// 새 매니저 생성
    pub fn new() -> Self {
        Self {
            eztrans: None,
            google: GoogleTranslator::new(),
            deepl: DeepLTranslator::new(String::new()),
            current_engine: TranslationEngine::EzTrans,
            source_lang: Language::Jpn,
            target_lang: Language::Kor,
        }
    }

    /// EzTrans 초기화
    pub fn init_eztrans(&mut self, dll_path: &str, dat_path: &str) -> Result<(), String> {
        match EzTransTranslator::new(dll_path, dat_path) {
            Ok(engine) => {
                self.eztrans = Some(engine);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// DeepL API 키 설정
    pub fn set_deepl_api_key(&mut self, api_key: String) {
        self.deepl = DeepLTranslator::new(api_key);
    }

    /// 현재 엔진 설정
    pub fn set_engine(&mut self, engine: TranslationEngine) {
        self.current_engine = engine;
    }

    /// 소스 언어 설정
    pub fn set_source_language(&mut self, lang: Language) {
        self.source_lang = lang;
    }

    /// 타겟 언어 설정
    pub fn set_target_language(&mut self, lang: Language) {
        self.target_lang = lang;
    }

    /// 현재 엔진 가져오기
    pub fn current_engine(&self) -> TranslationEngine {
        self.current_engine
    }

    /// 소스 언어 가져오기
    pub fn source_language(&self) -> Language {
        self.source_lang
    }

    /// 타겟 언어 가져오기
    pub fn target_language(&self) -> Language {
        self.target_lang
    }

    /// 번역 수행
    pub fn translate(&self, text: &str) -> TranslationResult {
        match self.current_engine {
            TranslationEngine::EzTrans => {
                if let Some(ref engine) = self.eztrans {
                    engine.translate(text, self.source_lang, self.target_lang)
                } else {
                    TranslationResult::Error("EzTrans 엔진이 초기화되지 않았습니다.".to_string())
                }
            }
            TranslationEngine::Google => {
                self.google
                    .translate(text, self.source_lang, self.target_lang)
            }
            TranslationEngine::DeepL => {
                self.deepl
                    .translate(text, self.source_lang, self.target_lang)
            }
        }
    }

    /// 현재 엔진 사용 가능 여부
    pub fn is_current_engine_available(&self) -> bool {
        match self.current_engine {
            TranslationEngine::EzTrans => self.eztrans.as_ref().is_some_and(|e| e.is_available()),
            TranslationEngine::Google => self.google.is_available(),
            TranslationEngine::DeepL => self.deepl.is_available(),
        }
    }
}

impl Default for TranslationManager {
    fn default() -> Self {
        Self::new()
    }
}

// 글로벌 인스턴스
static TRANSLATION_MANAGER: OnceLock<Arc<Mutex<TranslationManager>>> = OnceLock::new();

/// 글로벌 번역 매니저 가져오기
pub fn get_translation_manager() -> Arc<Mutex<TranslationManager>> {
    TRANSLATION_MANAGER
        .get_or_init(|| Arc::new(Mutex::new(TranslationManager::new())))
        .clone()
}
