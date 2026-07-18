//! Win32 UI와 독립적인 번역 설정 편집 규칙.

use crate::config::TranslationConfig;
use crate::translation::{Language, LlmProvider, TranslationEngine, TranslationJobSpec};

pub enum TranslationSettingChange {
    Engine(TranslationEngine),
    SourceLanguage(Language),
    TargetLanguage(Language),
    LlmProvider(LlmProvider),
    DeepLStrategyRoundRobin(bool),
    DeepLApiKey(String),
    AddDeepLKey(String),
    RemoveDeepLKey(usize),
    PapagoClientId(String),
    PapagoClientSecret(String),
    EzTransDllPath(String),
    EzTransDatPath(String),
    LlmModel(String),
    LlmApiKey(String),
    LlmBaseUrl(String),
    LlmSystemPrompt(String),
    LlmMaxTokensText(String),
    LlmDebounceText(String),
    LlmTemperatureSlider(i32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SettingsApplyResult {
    pub changed: bool,
    pub save_required: bool,
    pub preview_refresh_required: bool,
    pub runtime_sync_required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TranslationSettingsError {
    #[error("{field} 값은 부호 없는 정수여야 합니다.")]
    InvalidUnsignedInteger { field: &'static str },
    #[error("선택한 번역 엔진이 소스 언어를 지원하지 않습니다.")]
    UnsupportedSourceLanguage,
    #[error("선택한 번역 엔진이 대상 언어를 지원하지 않습니다.")]
    UnsupportedTargetLanguage,
    #[error("{engine:?} 엔진에 지원 언어가 없습니다.")]
    NoSupportedLanguage { engine: TranslationEngine },
}

pub struct TranslationSettingsEditor;

impl TranslationSettingsEditor {
    pub fn apply(
        config: &mut TranslationConfig,
        change: TranslationSettingChange,
    ) -> Result<SettingsApplyResult, TranslationSettingsError> {
        let runtime_sync_required = matches!(
            change,
            TranslationSettingChange::Engine(_)
                | TranslationSettingChange::EzTransDllPath(_)
                | TranslationSettingChange::EzTransDatPath(_)
        );
        let changed = match change {
            TranslationSettingChange::Engine(value) => {
                if value.supported_source_languages().is_empty()
                    || value.supported_target_languages().is_empty()
                {
                    return Err(TranslationSettingsError::NoSupportedLanguage { engine: value });
                }
                let mut changed = set_if_changed(&mut config.engine, value.to_str().to_string());
                changed |= normalize_language(
                    &mut config.source_lang,
                    value.supported_source_languages(),
                    value,
                )?;
                changed |= normalize_language(
                    &mut config.target_lang,
                    value.supported_target_languages(),
                    value,
                )?;
                changed
            }
            TranslationSettingChange::SourceLanguage(value) => {
                if !config
                    .get_engine()
                    .supported_source_languages()
                    .contains(&value)
                {
                    return Err(TranslationSettingsError::UnsupportedSourceLanguage);
                }
                set_if_changed(
                    &mut config.source_lang,
                    crate::translation::lang_utils::to_code(value).to_string(),
                )
            }
            TranslationSettingChange::TargetLanguage(value) => {
                if !config
                    .get_engine()
                    .supported_target_languages()
                    .contains(&value)
                {
                    return Err(TranslationSettingsError::UnsupportedTargetLanguage);
                }
                set_if_changed(
                    &mut config.target_lang,
                    crate::translation::lang_utils::to_code(value).to_string(),
                )
            }
            TranslationSettingChange::LlmProvider(value) => {
                let before = config.llm.get_provider();
                config.llm.set_provider(value);
                before != value
            }
            TranslationSettingChange::DeepLStrategyRoundRobin(value) => set_if_changed(
                &mut config.deepl_strategy,
                if value {
                    "round-robin".into()
                } else {
                    "failover".into()
                },
            ),
            TranslationSettingChange::DeepLApiKey(value) => {
                set_if_changed(&mut config.deepl_api_key, value)
            }
            TranslationSettingChange::AddDeepLKey(value) => {
                let value = value.trim();
                if value.is_empty() || config.deepl_keys.iter().any(|key| key == value) {
                    false
                } else {
                    config.deepl_keys.push(value.to_string());
                    true
                }
            }
            TranslationSettingChange::RemoveDeepLKey(index) => {
                if index >= config.deepl_keys.len() {
                    false
                } else {
                    config.deepl_keys.remove(index);
                    true
                }
            }
            TranslationSettingChange::PapagoClientId(value) => {
                set_if_changed(&mut config.papago_client_id, value)
            }
            TranslationSettingChange::PapagoClientSecret(value) => {
                set_if_changed(&mut config.papago_client_secret, value)
            }
            TranslationSettingChange::EzTransDllPath(value) => {
                set_if_changed(&mut config.eztrans_dll_path, value)
            }
            TranslationSettingChange::EzTransDatPath(value) => {
                set_if_changed(&mut config.eztrans_dat_path, value)
            }
            TranslationSettingChange::LlmModel(value) => {
                set_if_changed(&mut config.llm.model, value)
            }
            TranslationSettingChange::LlmApiKey(value) => {
                set_if_changed(&mut config.llm.api_key, value)
            }
            TranslationSettingChange::LlmBaseUrl(value) => {
                set_if_changed(&mut config.llm.base_url, value)
            }
            TranslationSettingChange::LlmSystemPrompt(value) => {
                set_if_changed(&mut config.llm.system_prompt, value)
            }
            TranslationSettingChange::LlmMaxTokensText(value) => {
                let value = parse_unsigned(&value, "max_tokens")?;
                set_if_changed(&mut config.llm.max_tokens, value.clamp(1, 32_000))
            }
            TranslationSettingChange::LlmDebounceText(value) => {
                let value = parse_unsigned(&value, "debounce_ms")?;
                set_if_changed(&mut config.llm.debounce_ms, value.clamp(0, 10_000))
            }
            TranslationSettingChange::LlmTemperatureSlider(value) => set_if_changed(
                &mut config.llm.temperature,
                (value as f32 / 100.0).clamp(0.0, 2.0),
            ),
        };
        Ok(SettingsApplyResult {
            changed,
            save_required: changed,
            preview_refresh_required: changed,
            runtime_sync_required: changed && runtime_sync_required,
        })
    }

    /// 선택된 EzTrans 설정이 완전할 때만 런타임 초기화를 시도한다.
    pub fn sync_runtime(config: &TranslationConfig) -> Result<(), String> {
        if config.get_engine() != TranslationEngine::EzTrans
            || config.eztrans_dll_path.trim().is_empty()
            || config.eztrans_dat_path.trim().is_empty()
        {
            return Ok(());
        }
        TranslationJobSpec::from_config(config)
            .map_err(|error| error.to_string())?
            .prepare()
            .map_err(|error| error.to_string())
    }
}

fn normalize_language(
    configured_code: &mut String,
    supported: &[Language],
    engine: TranslationEngine,
) -> Result<bool, TranslationSettingsError> {
    let configured = crate::translation::lang_utils::from_code(configured_code);
    if configured.is_some_and(|language| supported.contains(&language)) {
        return Ok(false);
    }
    let language = supported
        .first()
        .copied()
        .ok_or(TranslationSettingsError::NoSupportedLanguage { engine })?;
    Ok(set_if_changed(
        configured_code,
        crate::translation::lang_utils::to_code(language).to_string(),
    ))
}

fn parse_unsigned(value: &str, field: &'static str) -> Result<u32, TranslationSettingsError> {
    value
        .trim()
        .parse::<u32>()
        .map_err(|_| TranslationSettingsError::InvalidUnsignedInteger { field })
}

fn set_if_changed<T: PartialEq>(target: &mut T, value: T) -> bool {
    if *target == value {
        return false;
    }
    *target = value;
    true
}

#[cfg(test)]
#[path = "../../tests/unit/translation/settings.rs"]
mod tests;
