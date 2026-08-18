//! EzTrans 번역 엔진 래퍼.
//!
//! 번역은 `eztrans_core`가 제공하는 x86_64 순수 Rust 세션을 사용하며,
//! 평면 사전(`JisJK.flat.bin`)과 Ehnd 디렉터리를 모두 필수 자산으로 사용한다.

use std::path::Path;

use super::{Language, TranslationError, TranslationResult};
use eztrans_core::translation::{TranslateContext, TranslationSession};

/// EzTrans 번역기.
pub struct EzTransTranslator {
    session: TranslationSession,
    context: TranslateContext,
}

impl EzTransTranslator {
    /// 새 64비트 EzTrans 번역기를 생성한다.
    ///
    pub fn new(dictionary_path: &str, ehnd_path: &str) -> Result<Self, String> {
        let flat_bin = resolve_flat_dictionary(dictionary_path)?;
        let ehnd_dir = Path::new(ehnd_path);
        if !is_named_directory(ehnd_dir, "Ehnd") {
            return Err(format!(
                "EzTrans Ehnd 폴더를 찾을 수 없습니다: {}",
                ehnd_dir.display()
            ));
        }
        let session = TranslationSession::load_from_paths(&flat_bin, Some(ehnd_dir), None::<&Path>)
            .map_err(|error| format!("eztrans_core 세션 로드 실패: {error}"))?;
        let context = session.context();
        Ok(Self { session, context })
    }

    /// 일본어→한국어 번역 수행. 다른 언어 쌍은 `UnsupportedLanguagePair`.
    pub fn translate(
        &mut self,
        text: &str,
        source: Language,
        target: Language,
    ) -> TranslationResult {
        if source != Language::Jpn || target != Language::Kor {
            return Err(TranslationError::UnsupportedLanguagePair);
        }
        self.session
            .translate(&mut self.context, text)
            .map_err(|error| TranslationError::Engine(format!("번역 실패: {error}")))
    }
}

fn resolve_flat_dictionary(dictionary_path: &str) -> Result<&Path, String> {
    let configured = Path::new(dictionary_path);
    if configured.is_file()
        && configured
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("JisJK.flat.bin"))
    {
        return Ok(configured);
    }
    Err(format!(
        "eztrans_core 평면 사전(JisJK.flat.bin)을 찾을 수 없습니다 (경로: {})",
        configured.display()
    ))
}

fn is_named_directory(path: &Path, expected_name: &str) -> bool {
    path.is_dir()
        && path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(expected_name))
}
