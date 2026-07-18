//! 호출 환경과 무관한 번역 실행 사양 구성.

use crate::config::TranslationConfig;
use crate::translation::worker::EngineCredentials;
use crate::translation::{Language, TranslationEngine, prepare_eztrans};

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
        let engine = config
            .get_engine()
            .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
        let source = config
            .get_source_language()
            .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
        let target = config
            .get_target_language()
            .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
        Self::with_engine_languages(config, engine, source, target)
    }

    /// CLI 등의 명시적 엔진/언어 재정의도 동일한 자격증명 구성 규칙을 사용한다.
    pub fn with_engine_languages(
        config: &TranslationConfig,
        engine: TranslationEngine,
        source_lang: Language,
        target_lang: Language,
    ) -> Result<Self, TranslationConfigError> {
        if !engine.supports_pair(source_lang, target_lang) {
            return Err(TranslationConfigError::UnsupportedLanguagePair {
                engine: engine.to_str(),
            });
        }
        let credentials =
            match engine {
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
                    EngineCredentials::Llm(config.llm.to_call_params().map_err(|error| {
                        TranslationConfigError::InvalidSetting(error.to_string())
                    })?)
                }
                TranslationEngine::Custom => {
                    let params = crate::translation::custom::CustomApiCallParams {
                        url: config.custom.url.clone(),
                        api_key: config.custom.api_key.clone(),
                        auth_header: config.custom.auth_header.clone(),
                        auth_scheme: config.custom.auth_scheme.clone(),
                        headers: config.custom.headers.clone(),
                        request_template: config.custom.request_template.clone(),
                        response_path: config.custom.response_path.clone(),
                    };
                    params
                        .validate()
                        .map_err(TranslationConfigError::InvalidSetting)?;
                    EngineCredentials::Custom(params)
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
        prepare_eztrans(&self.eztrans_dll_path, &self.eztrans_dat_path)
            .map_err(TranslationPrepareError::EzTransInitialization)
    }

    /// 준비가 끝난 실행 사양을 요청/배치 작업이 복사 없이 소유하도록 분해한다.
    pub fn into_parts(self) -> (TranslationEngine, Language, Language, EngineCredentials) {
        (
            self.engine,
            self.source_lang,
            self.target_lang,
            self.credentials,
        )
    }

    #[cfg(test)]
    pub fn engine(&self) -> TranslationEngine {
        self.engine
    }
    #[cfg(test)]
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
    #[error("잘못된 번역 설정: {0}")]
    InvalidSetting(String),
}

#[derive(Debug, thiserror::Error)]
pub enum TranslationPrepareError {
    #[error("EzTrans 초기화 실패: {0}")]
    EzTransInitialization(String),
}

#[cfg(test)]
#[path = "../../tests/unit/translation/job.rs"]
mod tests;
