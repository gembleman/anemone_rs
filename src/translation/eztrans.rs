//! EzTrans 번역 엔진 래퍼
//!
//! eztrans-rs 라이브러리를 사용하여 일본어-한국어 번역 수행

use super::{Language, TranslationResult, Translator};
use eztrans_rs::EzTransEngine;
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
        let engine = EzTransEngine::new(dll_path)
            .map_err(|e| format!("DLL 로드 실패: {:?}", e))?;

        // 초기화 (EHND 모드)
        engine
            .initialize_ex("CSUSER123455", dat_path)
            .map_err(|e| format!("초기화 실패: {:?}", e))?;

        Ok(Self {
            engine: Mutex::new(engine),
        })
    }
}

impl Translator for EzTransTranslator {
    fn translate(&self, text: &str, source: Language, target: Language) -> TranslationResult {
        // EzTrans는 일본어→한국어만 지원
        if source != Language::Japanese || target != Language::Korean {
            return TranslationResult::Error(
                "EzTrans는 일본어→한국어 번역만 지원합니다.".to_string(),
            );
        }

        let engine = match self.engine.lock() {
            Ok(e) => e,
            Err(e) => return TranslationResult::Error(format!("엔진 잠금 실패: {}", e)),
        };

        match engine.default_translate(text) {
            Ok(result) => TranslationResult::Success(result),
            Err(e) => TranslationResult::Error(format!("번역 실패: {:?}", e)),
        }
    }

    fn engine_name(&self) -> &'static str {
        "EzTrans"
    }

    fn is_available(&self) -> bool {
        self.engine.lock().is_ok()
    }
}
