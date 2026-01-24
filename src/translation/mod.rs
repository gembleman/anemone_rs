//! 번역 엔진 모듈
//!
//! 지원 엔진:
//! - EzTrans (eztrans-rs 라이브러리 사용)
//! - Google Translate (HTTP API)
//! - DeepL (HTTP API)

mod eztrans;
mod google;
mod deepl;

pub use eztrans::EzTransTranslator;
pub use google::GoogleTranslator;
pub use deepl::DeepLTranslator;

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

    pub fn name(&self) -> &'static str {
        match self {
            Self::EzTrans => "EzTrans",
            Self::Google => "Google",
            Self::DeepL => "DeepL",
        }
    }
}

/// 언어 코드
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Language {
    #[default]
    Japanese = 0,
    Korean = 1,
    English = 2,
    ChineseSimplified = 3,
    ChineseTraditional = 4,
}

impl Language {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Japanese,
            1 => Self::Korean,
            2 => Self::English,
            3 => Self::ChineseSimplified,
            4 => Self::ChineseTraditional,
            _ => Self::Japanese,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Japanese => "일본어",
            Self::Korean => "한국어",
            Self::English => "영어",
            Self::ChineseSimplified => "중국어(간체)",
            Self::ChineseTraditional => "중국어(번체)",
        }
    }

    /// Google Translate API 언어 코드
    pub fn google_code(&self) -> &'static str {
        match self {
            Self::Japanese => "ja",
            Self::Korean => "ko",
            Self::English => "en",
            Self::ChineseSimplified => "zh-CN",
            Self::ChineseTraditional => "zh-TW",
        }
    }

    /// DeepL API 언어 코드
    pub fn deepl_code(&self) -> &'static str {
        match self {
            Self::Japanese => "JA",
            Self::Korean => "KO",
            Self::English => "EN",
            Self::ChineseSimplified => "ZH",
            Self::ChineseTraditional => "ZH",
        }
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
            source_lang: Language::Japanese,
            target_lang: Language::Korean,
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
                self.google.translate(text, self.source_lang, self.target_lang)
            }
            TranslationEngine::DeepL => {
                self.deepl.translate(text, self.source_lang, self.target_lang)
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
