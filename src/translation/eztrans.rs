//! EzTrans 번역 엔진 래퍼.
//!
//! 번역은 `eztrans_core`가 제공하는 x86_64 순수 Rust 세션을 사용하며,
//! 기존 설정의 DLL 경로 인자는
//! 호환성을 위해 평면 사전(`JisJK.flat.bin`) 경로를 찾는 기준으로만 사용한다.

use std::path::{Path, PathBuf};

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
    /// 기존 설정 파일과의 호환성을 위해 경로 인자는 그대로 받는다. 값이
    /// `.dll`이면 같은 디렉터리의 `JisJK.flat.bin`을 평면 사전으로 사용한다.
    /// `dat_path`의 형제 `Ehnd` 디렉터리는 필터/사용자 사전 로드에 사용한다.
    pub fn new(dictionary_path: &str, dat_path: &str) -> Result<Self, String> {
        let flat_bin = resolve_flat_dictionary(dictionary_path, dat_path)?;
        let dat_dir = Path::new(dat_path);
        let ehnd_dir = dat_dir
            .parent()
            .map(|parent| parent.join("Ehnd"))
            .filter(|path| path.is_dir());
        let session =
            TranslationSession::load_from_paths(&flat_bin, ehnd_dir.as_ref(), Some(dat_dir))
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

fn resolve_flat_dictionary(dictionary_path: &str, dat_path: &str) -> Result<PathBuf, String> {
    let configured = Path::new(dictionary_path);
    let mut candidates = Vec::new();
    if configured
        .extension()
        .is_some_and(|extension| !extension.eq_ignore_ascii_case("dll"))
    {
        candidates.push(configured.to_path_buf());
    }
    if let Some(parent) = configured
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        candidates.push(parent.join("JisJK.flat.bin"));
    }
    let dat_parent = Path::new(dat_path)
        .parent()
        .ok_or_else(|| "EzTrans Dat 상위 디렉터리를 확인할 수 없습니다.".to_string())?;
    candidates.push(dat_parent.join("JisJK.flat.bin"));
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            format!(
                "eztrans_core 평면 사전(JisJK.flat.bin)을 찾을 수 없습니다 (기준: {})",
                configured.display()
            )
        })
}
