//! 설정값 → 엔진별 [`PreparedEngineKind`] 구성 규칙.

use crate::config::TranslationConfig;
use crate::translation::custom::CustomApiCallParams;
use crate::translation::mys_translater::MysTranslaterCallParams;
use crate::translation::{EzTransProcessConfig, Language, TranslationEngine};

use super::{PreparedEngineKind, PreparedJob, TranslationConfigError};

impl PreparedJob {
    /// CLI 등의 명시적 엔진/언어 재정의도 동일한 backend 구성 규칙을 사용한다.
    pub fn with_engine_languages(
        config: &TranslationConfig,
        engine: TranslationEngine,
        source: Language,
        target: Language,
    ) -> Result<Self, TranslationConfigError> {
        let kind = match engine {
            TranslationEngine::EzTrans => {
                if config.eztrans_dictionary_path.trim().is_empty()
                    || config.eztrans_ehnd_path.trim().is_empty()
                {
                    return Err(TranslationConfigError::MissingEzTransPath);
                }
                PreparedEngineKind::EzTrans {
                    process: EzTransProcessConfig {
                        dictionary_path: super::resolve_configured_eztrans_path(
                            &config.eztrans_dictionary_path,
                        ),
                        ehnd_path: super::resolve_configured_eztrans_path(
                            &config.eztrans_ehnd_path,
                        ),
                        process_count: crate::config::limits::eztrans_process_count(
                            config.eztrans_process_count,
                        ) as usize,
                    },
                    postprocess_dictionary: config.eztrans_postprocess_dictionary.clone(),
                    postprocess_matcher: super::super::postprocess::EzTransPostprocessMatcher::new(
                        &config.eztrans_postprocess_dictionary,
                    ),
                }
            }
            TranslationEngine::Google => PreparedEngineKind::Google,
            TranslationEngine::DeepL => {
                let keys = config
                    .deepl_effective_keys()
                    .into_iter()
                    .filter(|key| !key.trim().is_empty())
                    .collect::<Vec<_>>();
                if keys.is_empty() {
                    return Err(TranslationConfigError::MissingCredential("DeepL API 키"));
                }
                PreparedEngineKind::DeepL {
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
                PreparedEngineKind::Papago {
                    client_id: config.papago_client_id.clone(),
                    client_secret: config.papago_client_secret.clone(),
                }
            }
            TranslationEngine::Llm => {
                if config.llm.api_key.trim().is_empty() {
                    return Err(TranslationConfigError::MissingCredential("LLM API 키"));
                }
                PreparedEngineKind::Llm(
                    config.llm.to_call_params().map_err(|error| {
                        TranslationConfigError::InvalidSetting(error.to_string())
                    })?,
                )
            }
            TranslationEngine::MysTranslater => {
                if config.mys_translater_url.trim().is_empty()
                    || config.mys_translater_api_key.trim().is_empty()
                {
                    return Err(TranslationConfigError::MissingCredential(
                        "MyS Translater 서버 URL/API 토큰",
                    ));
                }
                let params = MysTranslaterCallParams {
                    base_url: config.mys_translater_url.clone(),
                    api_token: config.mys_translater_api_key.clone(),
                };
                params
                    .validate()
                    .map_err(TranslationConfigError::InvalidSetting)?;
                PreparedEngineKind::MysTranslater(params)
            }
            TranslationEngine::Custom => {
                let custom = config
                    .active_custom_api()
                    .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
                let params = CustomApiCallParams {
                    url: custom.url.clone(),
                    api_key: custom.api_key.clone(),
                    auth_header: custom.auth_header.clone(),
                    auth_scheme: custom.auth_scheme.clone(),
                    headers: custom.headers.clone(),
                    request_template: custom.request_template.clone(),
                    response_path: custom.response_path.clone(),
                };
                params
                    .validate()
                    .map_err(TranslationConfigError::InvalidSetting)?;
                PreparedEngineKind::Custom(params)
            }
        };

        Self::from_kind(kind, source, target)
    }
}
