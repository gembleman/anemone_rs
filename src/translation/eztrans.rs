//! EzTrans 번역 엔진 래퍼
//!
//! eztrans-rs 라이브러리를 사용하여 일본어-한국어 번역 수행

use super::{TranslationError, TranslationResult};
use eztrans_rs::EzTransEngine;
use isolang::Language;
use std::sync::Mutex;

/// EzTrans 번역기
pub struct EzTransTranslator {
    engine: Mutex<EzTransEngine>,
}

// Mutex로 감싸서 Send + Sync 구현
unsafe impl Send for EzTransTranslator {}
unsafe impl Sync for EzTransTranslator {}

impl EzTransTranslator {
    /// 새 EzTrans 번역기 생성
    ///
    /// # Arguments
    /// * `dll_path` - J2KEngine.dll 경로
    /// * `dat_path` - Dat 폴더 경로
    pub fn new(dll_path: &str, dat_path: &str) -> Result<Self, String> {
        // 엔진 로드
        let engine = EzTransEngine::new(dll_path).map_err(|e| format!("DLL 로드 실패: {:?}", e))?;

        // 초기화 (EHND 모드)
        engine
            .initialize_ex("CSUSER123455", dat_path)
            .map_err(|e| format!("초기화 실패: {:?}", e))?;

        Ok(Self {
            engine: Mutex::new(engine),
        })
    }

    /// 일본어→한국어 번역 수행. 다른 언어 쌍은 `UnsupportedLanguagePair`.
    pub fn translate(&self, text: &str, source: Language, target: Language) -> TranslationResult {
        if source != Language::Jpn || target != Language::Kor {
            return Err(TranslationError::UnsupportedLanguagePair);
        }

        let engine = self
            .engine
            .lock()
            .map_err(|e| TranslationError::Engine(format!("엔진 잠금 실패: {}", e)))?;

        engine
            .default_translate(text)
            .map_err(|e| TranslationError::Engine(format!("번역 실패: {:?}", e)))
    }
}
