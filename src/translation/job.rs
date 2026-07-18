//! 호출 환경과 무관한 번역 실행 사양 구성.

use crate::config::TranslationConfig;
use crate::translation::worker::EngineCredentials;
use crate::translation::{Language, TranslationEngine, get_eztrans_manager};

/// 비밀 자격증명을 포함할 수 있는 실행 사양. 의도적으로 `Debug`를 구현하지 않는다.
#[derive(Clone)]
pub struct TranslationJobSpec {
    engine: TranslationEngine,
    source_lang: Language,
    target_lang: Language,
    credentials: EngineCredentials,
    eztrans_dll_path: String,
    eztrans_dat_path: String,
}

impl TranslationJobSpec {
    pub fn from_config(config: &TranslationConfig) -> Result<Self, TranslationConfigError> {
        Self::with_engine_languages(
            config,
            config.get_engine(),
            config.get_source_language(),
            config.get_target_language(),
        )
    }

    /// CLI 등의 명시적 엔진/언어 재정의도 동일한 자격증명 구성 규칙을 사용한다.
    pub fn with_engine_languages(
        config: &TranslationConfig,
        engine: TranslationEngine,
        source_lang: Language,
        target_lang: Language,
    ) -> Result<Self, TranslationConfigError> {
        if !engine.supported_source_languages().contains(&source_lang)
            || !engine.supported_target_languages().contains(&target_lang)
        {
            return Err(TranslationConfigError::UnsupportedLanguagePair {
                engine: engine.to_str(),
            });
        }
        let credentials = match engine {
            TranslationEngine::EzTrans | TranslationEngine::Google => EngineCredentials::None,
            TranslationEngine::DeepL => {
                let keys = config.deepl_effective_keys();
                if keys.is_empty() {
                    return Err(TranslationConfigError::MissingCredential("DeepL API 키"));
                }
                EngineCredentials::DeepL {
                    keys,
                    strategy: config.deepl_strategy(),
                }
            }
            TranslationEngine::Papago => {
                if config.papago_client_id.trim().is_empty()
                    || config.papago_client_secret.trim().is_empty()
                {
                    return Err(TranslationConfigError::MissingCredential(
                        "Papago client_id/client_secret",
                    ));
                }
                EngineCredentials::Papago {
                    client_id: config.papago_client_id.clone(),
                    client_secret: config.papago_client_secret.clone(),
                }
            }
            TranslationEngine::Llm => {
                if config.llm.api_key.trim().is_empty() {
                    return Err(TranslationConfigError::MissingCredential("LLM API 키"));
                }
                EngineCredentials::Llm(config.llm.to_call_params())
            }
        };
        if engine == TranslationEngine::EzTrans
            && (config.eztrans_dll_path.trim().is_empty()
                || config.eztrans_dat_path.trim().is_empty())
        {
            return Err(TranslationConfigError::MissingEzTransPath);
        }
        Ok(Self {
            engine,
            source_lang,
            target_lang,
            credentials,
            eztrans_dll_path: config.eztrans_dll_path.clone(),
            eztrans_dat_path: config.eztrans_dat_path.clone(),
        })
    }

    /// EzTrans처럼 사전 초기화가 필요한 엔진을 준비한다.
    pub fn prepare(&self) -> Result<(), TranslationPrepareError> {
        if self.engine != TranslationEngine::EzTrans {
            return Ok(());
        }
        let manager = get_eztrans_manager();
        let mut manager = manager
            .lock()
            .map_err(|_| TranslationPrepareError::ManagerLock)?;
        manager
            .init(&self.eztrans_dll_path, &self.eztrans_dat_path)
            .map_err(TranslationPrepareError::EzTransInitialization)
    }

    pub fn engine(&self) -> TranslationEngine {
        self.engine
    }
    pub fn source_lang(&self) -> Language {
        self.source_lang
    }
    pub fn target_lang(&self) -> Language {
        self.target_lang
    }
    pub fn credentials(&self) -> EngineCredentials {
        self.credentials.clone()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TranslationConfigError {
    #[error("{0}가 설정되지 않았습니다.")]
    MissingCredential(&'static str),
    #[error("EzTrans DLL/DAT 경로가 설정되지 않았습니다.")]
    MissingEzTransPath,
    #[error("{engine} 엔진은 선택한 언어 조합을 지원하지 않습니다.")]
    UnsupportedLanguagePair { engine: &'static str },
}

#[derive(Debug, thiserror::Error)]
pub enum TranslationPrepareError {
    #[error("EzTrans 매니저 잠금 실패")]
    ManagerLock,
    #[error("EzTrans 초기화 실패: {0}")]
    EzTransInitialization(String),
}

#[cfg(test)]
#[path = "../../tests/unit/translation/job.rs"]
mod tests;
