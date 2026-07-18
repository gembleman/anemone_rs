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

pub struct TranslationSettingsEditor;

impl TranslationSettingsEditor {
    pub fn apply(
        config: &mut TranslationConfig,
        change: TranslationSettingChange,
    ) -> SettingsApplyResult {
        let runtime_sync_required = matches!(
            change,
            TranslationSettingChange::Engine(_)
                | TranslationSettingChange::EzTransDllPath(_)
                | TranslationSettingChange::EzTransDatPath(_)
        );
        let changed = match change {
            TranslationSettingChange::Engine(value) => {
                set_if_changed(&mut config.engine, value.to_str().to_string())
            }
            TranslationSettingChange::SourceLanguage(value) => set_if_changed(
                &mut config.source_lang,
                crate::translation::lang_utils::to_code(value).to_string(),
            ),
            TranslationSettingChange::TargetLanguage(value) => set_if_changed(
                &mut config.target_lang,
                crate::translation::lang_utils::to_code(value).to_string(),
            ),
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
                value.trim().parse::<u32>().ok().is_some_and(|value| {
                    set_if_changed(&mut config.llm.max_tokens, value.clamp(1, 32_000))
                })
            }
            TranslationSettingChange::LlmDebounceText(value) => {
                value.trim().parse::<u32>().ok().is_some_and(|value| {
                    set_if_changed(&mut config.llm.debounce_ms, value.clamp(0, 10_000))
                })
            }
            TranslationSettingChange::LlmTemperatureSlider(value) => set_if_changed(
                &mut config.llm.temperature,
                (value as f32 / 100.0).clamp(0.0, 2.0),
            ),
        };
        SettingsApplyResult {
            changed,
            save_required: changed,
            preview_refresh_required: changed,
            runtime_sync_required: changed && runtime_sync_required,
        }
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
