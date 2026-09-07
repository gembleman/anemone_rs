//! [`PreparedJob::from_config_cached`]가 쓰는 설정 fingerprint 계산.

use crate::config::TranslationConfig;
use crate::translation::TranslationEngine;

use super::TranslationConfigError;

/// `PreparedJob::from_config`가 소비하는 설정 필드만 반영한 fingerprint.
/// 잘못된 hit(다른 설정이 같은 키)를 막으려면 from_config의 모든 입력을
/// 빠짐없이 해시해야 한다. DefaultHasher면 충분하다 — 입력이 앱 자체 설정이라
/// 공격 대상이 아니다. 사전은 매 요청 한 번 해시되는데, 이는 automaton 재빌드
/// (벤치: 사전 2만 개에서 줄당 27.6ms)보다 2자릿수 이상 저렴한 비용이다.
pub(super) fn config_fingerprint(
    config: &TranslationConfig,
) -> Result<u64, TranslationConfigError> {
    hash_config(config, true)
}

/// 요청마다 전체 후처리 사전을 읽지 않는 빠른 설정 표식.
///
/// 사전은 Vec를 통째로 해시하지 않고 할당 주소·크기·용량과 양 끝 항목만
/// 확인한다. 앱 설정은 변경 시 새 draft를 통째로 적용하므로 이 표식은 일반적인
/// 설정 변경을 모두 구분하고, 같은 설정으로 반복 요청할 때의 hot path를 일정한
/// 비용으로 유지한다. 실제 사전 해시는 표식이 바뀐 경우에만 계산한다.
pub(super) fn config_stamp(config: &TranslationConfig) -> Result<u64, TranslationConfigError> {
    hash_config(config, false)
}

fn hash_config(
    config: &TranslationConfig,
    include_dictionary_contents: bool,
) -> Result<u64, TranslationConfigError> {
    use std::hash::{Hash, Hasher};

    let engine = config
        .get_engine()
        .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
    let source = config
        .get_source_language()
        .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
    let target = config
        .get_target_language()
        .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
    let mut hasher = std::hash::DefaultHasher::new();
    (engine as u8).hash(&mut hasher);
    source.hash(&mut hasher);
    target.hash(&mut hasher);
    match engine {
        TranslationEngine::EzTrans => {
            config.eztrans_dictionary_path.hash(&mut hasher);
            config.eztrans_ehnd_path.hash(&mut hasher);
            config.eztrans_process_count.hash(&mut hasher);
            if include_dictionary_contents {
                config.eztrans_postprocess_dictionary.hash(&mut hasher);
            } else {
                config
                    .eztrans_postprocess_dictionary
                    .as_ptr()
                    .hash(&mut hasher);
                config
                    .eztrans_postprocess_dictionary
                    .len()
                    .hash(&mut hasher);
                config
                    .eztrans_postprocess_dictionary
                    .capacity()
                    .hash(&mut hasher);
                config
                    .eztrans_postprocess_dictionary
                    .first()
                    .hash(&mut hasher);
                config
                    .eztrans_postprocess_dictionary
                    .last()
                    .hash(&mut hasher);
            }
        }
        TranslationEngine::Google => {}
        TranslationEngine::DeepL => {
            config.deepl_effective_keys().hash(&mut hasher);
            (config.deepl_strategy() as u8).hash(&mut hasher);
        }
        TranslationEngine::Papago => {
            config.papago_client_id.hash(&mut hasher);
            config.papago_client_secret.hash(&mut hasher);
        }
        TranslationEngine::Llm => {
            let params = config
                .llm
                .to_call_params()
                .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
            (params.provider as u8).hash(&mut hasher);
            params.model.hash(&mut hasher);
            params.api_key.hash(&mut hasher);
            params.base_url.hash(&mut hasher);
            params.system_prompt.hash(&mut hasher);
            params.temperature.to_bits().hash(&mut hasher);
            params.top_p.to_bits().hash(&mut hasher);
            params.frequency_penalty.to_bits().hash(&mut hasher);
            params.presence_penalty.to_bits().hash(&mut hasher);
            params.max_tokens.hash(&mut hasher);
            params
                .reasoning_effort
                .map(|effort| effort as u8)
                .hash(&mut hasher);
            for entry in &params.glossary {
                entry.source.hash(&mut hasher);
                entry.target.hash(&mut hasher);
            }
        }
        TranslationEngine::MysTranslater => {
            config.mys_translater_url.hash(&mut hasher);
            config.mys_translater_api_key.hash(&mut hasher);
        }
        TranslationEngine::Custom => {
            let custom = config
                .active_custom_api()
                .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
            custom.name.hash(&mut hasher);
            custom.url.hash(&mut hasher);
            custom.api_key.hash(&mut hasher);
            custom.auth_header.hash(&mut hasher);
            custom.auth_scheme.hash(&mut hasher);
            custom.headers.hash(&mut hasher);
            custom.request_template.hash(&mut hasher);
            custom.response_path.hash(&mut hasher);
        }
    }
    Ok(hasher.finish())
}
