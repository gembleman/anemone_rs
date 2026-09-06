//! Win32 UI와 독립적인 번역 설정 편집 규칙.

use crate::config::TranslationConfig;
use crate::config::limits;
#[cfg(test)]
use crate::translation::PreparedJob;
use crate::translation::llm::ReasoningEffort;
use crate::translation::{DeepLApiTier, Language, LlmProvider, TranslationEngine};

pub enum TranslationSettingChange {
    Engine(TranslationEngine),
    SourceLanguage(Language),
    TargetLanguage(Language),
    LlmProvider(LlmProvider),
    DeepLStrategyRoundRobin(bool),
    AddDeepLKey { tier: DeepLApiTier, key: String },
    RemoveDeepLKey(usize),
    PapagoClientId(String),
    PapagoClientSecret(String),
    MysTranslaterApiKey(String),
    EzTransDictionaryPath(String),
    EzTransEhndPath(String),
    LlmModel(String),
    LlmApiKey(String),
    LlmSystemPrompt(String),
    LlmMaxTokensText(String),
    LlmDebounceText(String),
    LlmTemperatureText(String),
    LlmTemperatureSlider(i32),
    LlmReasoningEffort(Option<ReasoningEffort>),
    SelectCustomApi(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TranslationSettingsChangeResult {
    pub changed: bool,
    pub runtime_sync_required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TranslationSettingsError {
    #[error("{field} 값은 부호 없는 정수여야 합니다.")]
    InvalidUnsignedInteger { field: &'static str },
    #[error("{field} 값은 유한한 숫자여야 합니다.")]
    InvalidFloatingPoint { field: &'static str },
    #[error("선택한 번역 엔진이 소스 언어를 지원하지 않습니다.")]
    UnsupportedSourceLanguage,
    #[error("선택한 번역 엔진이 대상 언어를 지원하지 않습니다.")]
    UnsupportedTargetLanguage,
    #[error("{engine:?} 엔진에 지원 언어가 없습니다.")]
    NoSupportedLanguage { engine: TranslationEngine },
    #[error("현재 번역 설정이 잘못되었습니다: {0}")]
    InvalidCurrentConfig(String),
    #[error("DeepL API Free 키는 ':fx'로 끝나야 합니다.")]
    InvalidDeepLFreeKey,
    #[error("':fx'로 끝나는 키는 DeepL API Free 유형으로 추가해야 합니다.")]
    InvalidDeepLProKey,
}

pub struct TranslationSettingsEditor;

impl TranslationSettingsEditor {
    pub fn apply(
        config: &mut TranslationConfig,
        change: TranslationSettingChange,
    ) -> Result<TranslationSettingsChangeResult, TranslationSettingsError> {
        use TranslationSettingChange::*;

        let runtime_sync_required = matches!(
            change,
            Engine(_) | EzTransDictionaryPath(_) | EzTransEhndPath(_)
        );
        // 각 variant를 정확히 한 번만 매치하고, 해당 payload만 헬퍼로 넘긴다.
        // 헬퍼는 자기 그룹 밖 variant를 받을 일이 구조적으로 없으므로 catch-all이 없다.
        let changed = match change {
            Engine(value) => apply_engine_change(config, value)?,
            SourceLanguage(value) => apply_source_language_change(config, value)?,
            TargetLanguage(value) => apply_target_language_change(config, value)?,
            LlmProvider(value) => apply_llm_provider_change(config, value)?,
            DeepLStrategyRoundRobin(value) => apply_deepl_strategy_change(config, value),
            AddDeepLKey { tier, key } => apply_add_deepl_key(config, tier, key)?,
            RemoveDeepLKey(index) => apply_remove_deepl_key(config, index),
            PapagoClientId(value) => set_if_changed(&mut config.papago_client_id, value),
            PapagoClientSecret(value) => set_if_changed(&mut config.papago_client_secret, value),
            MysTranslaterApiKey(value) => set_if_changed(&mut config.mys_translater_api_key, value),
            EzTransDictionaryPath(value) => {
                set_if_changed(&mut config.eztrans_dictionary_path, value)
            }
            EzTransEhndPath(value) => set_if_changed(&mut config.eztrans_ehnd_path, value),
            LlmModel(value) => set_if_changed(&mut config.llm.model, value),
            LlmApiKey(value) => set_if_changed(&mut config.llm.api_key, value),
            LlmSystemPrompt(value) => set_if_changed(&mut config.llm.system_prompt, value),
            LlmMaxTokensText(value) => apply_llm_max_tokens(config, value)?,
            LlmDebounceText(value) => apply_llm_debounce(config, value)?,
            LlmTemperatureText(value) => apply_llm_temperature_text(config, value)?,
            LlmTemperatureSlider(value) => apply_llm_temperature_slider(config, value),
            LlmReasoningEffort(value) => set_if_changed(&mut config.llm.reasoning_effort, value),
            SelectCustomApi(value) => config.select_custom_api(&value).map_err(|error| {
                TranslationSettingsError::InvalidCurrentConfig(error.to_string())
            })?,
        };
        Ok(TranslationSettingsChangeResult {
            changed,
            runtime_sync_required: changed && runtime_sync_required,
        })
    }

    /// 설정된 EzTrans 사전 경로가 쓸 수 있는 평면 사전인지 확인한다.
    ///
    /// 경로는 `JisJK.flat.bin` 파일을 직접 가리켜야 한다.
    pub fn eztrans_dictionary_invalid(configured: &str) -> bool {
        path_invalid(configured, |path| {
            path.is_file()
                && path
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("JisJK.flat.bin"))
        })
    }

    /// 설정된 EzTrans Ehnd 경로가 실제 Ehnd 폴더인지 확인한다.
    pub fn eztrans_ehnd_invalid(configured: &str) -> bool {
        path_invalid(configured, |path| {
            path.is_dir()
                && path
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("Ehnd"))
        })
    }

    /// EzTrans가 선택되면 두 필수 경로를 포함해 런타임 초기화를 검증한다.
    #[cfg(test)]
    pub fn sync_runtime(config: &TranslationConfig) -> Result<(), String> {
        if config.get_engine().map_err(|error| error.to_string())? != TranslationEngine::EzTrans {
            return Ok(());
        }
        PreparedJob::from_config(config)
            .map_err(|error| error.to_string())?
            .prepare()
            .map_err(|error| error.to_string())
    }
}

/// 엔진 전환. 새 엔진이 현재 언어쌍을 지원하지 않으면 소스/대상 언어를
/// 새 엔진이 지원하는 값으로 정규화한다.
fn apply_engine_change(
    config: &mut TranslationConfig,
    value: TranslationEngine,
) -> Result<bool, TranslationSettingsError> {
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
    let source =
        crate::translation::lang_utils::from_code(&config.source_lang).ok_or_else(|| {
            TranslationSettingsError::InvalidCurrentConfig("소스 언어를 해석할 수 없습니다".into())
        })?;
    let target =
        crate::translation::lang_utils::from_code(&config.target_lang).ok_or_else(|| {
            TranslationSettingsError::InvalidCurrentConfig("대상 언어를 해석할 수 없습니다".into())
        })?;
    if !value.supports_pair(source, target) {
        let replacement = value
            .supported_targets_for(source)
            .into_iter()
            .next()
            .ok_or(TranslationSettingsError::NoSupportedLanguage { engine: value })?;
        changed |= set_if_changed(
            &mut config.target_lang,
            crate::translation::lang_utils::to_code(replacement).to_string(),
        );
    }
    Ok(changed)
}

/// 소스 언어 변경. 현재 엔진이 지원하지 않으면 오류, 새 소스가 현재 대상과
/// 짝을 이루지 못하면 대상 언어를 새 소스가 지원하는 값으로 정규화한다.
fn apply_source_language_change(
    config: &mut TranslationConfig,
    value: Language,
) -> Result<bool, TranslationSettingsError> {
    let engine = config
        .get_engine()
        .map_err(|error| TranslationSettingsError::InvalidCurrentConfig(error.to_string()))?;
    if !engine.supported_source_languages().contains(&value) {
        return Err(TranslationSettingsError::UnsupportedSourceLanguage);
    }
    let mut changed = set_if_changed(
        &mut config.source_lang,
        crate::translation::lang_utils::to_code(value).to_string(),
    );
    let target = config
        .get_target_language()
        .map_err(|error| TranslationSettingsError::InvalidCurrentConfig(error.to_string()))?;
    if !engine.supports_pair(value, target) {
        let replacement = engine
            .supported_targets_for(value)
            .into_iter()
            .next()
            .ok_or(TranslationSettingsError::NoSupportedLanguage { engine })?;
        changed |= set_if_changed(
            &mut config.target_lang,
            crate::translation::lang_utils::to_code(replacement).to_string(),
        );
    }
    Ok(changed)
}

/// 대상 언어 변경. 현재 엔진 + 소스 언어와 짝을 이루지 못하면 오류.
fn apply_target_language_change(
    config: &mut TranslationConfig,
    value: Language,
) -> Result<bool, TranslationSettingsError> {
    let engine = config
        .get_engine()
        .map_err(|error| TranslationSettingsError::InvalidCurrentConfig(error.to_string()))?;
    let source = config
        .get_source_language()
        .map_err(|error| TranslationSettingsError::InvalidCurrentConfig(error.to_string()))?;
    if !engine.supports_pair(source, value) {
        return Err(TranslationSettingsError::UnsupportedTargetLanguage);
    }
    Ok(set_if_changed(
        &mut config.target_lang,
        crate::translation::lang_utils::to_code(value).to_string(),
    ))
}

fn apply_llm_provider_change(
    config: &mut TranslationConfig,
    value: LlmProvider,
) -> Result<bool, TranslationSettingsError> {
    let before = config
        .llm
        .get_provider()
        .map_err(|error| TranslationSettingsError::InvalidCurrentConfig(error.to_string()))?;
    config.llm.set_provider(value);
    Ok(before != value)
}

/// DeepL 키 선택 전략 토글.
fn apply_deepl_strategy_change(config: &mut TranslationConfig, round_robin: bool) -> bool {
    set_if_changed(
        &mut config.deepl_strategy,
        if round_robin {
            "round-robin".into()
        } else {
            "failover".into()
        },
    )
}

/// DeepL 키 추가. 키 접미사가 선택한 유형과 어긋나면 거부하고, 빈 값이나
/// 이미 등록된 키는 변경 없이 넘어간다.
fn apply_add_deepl_key(
    config: &mut TranslationConfig,
    tier: DeepLApiTier,
    key: String,
) -> Result<bool, TranslationSettingsError> {
    let value = key.trim();
    if !value.is_empty() && DeepLApiTier::from_api_key(value) != tier {
        return Err(match tier {
            DeepLApiTier::Free => TranslationSettingsError::InvalidDeepLFreeKey,
            DeepLApiTier::Pro => TranslationSettingsError::InvalidDeepLProKey,
        });
    }
    if value.is_empty() || config.deepl_keys.iter().any(|key| key == value) {
        Ok(false)
    } else {
        config.deepl_keys.push(value.to_string());
        Ok(true)
    }
}

/// DeepL 키 삭제. 범위를 벗어난 index는 변경 없이 무시한다.
fn apply_remove_deepl_key(config: &mut TranslationConfig, index: usize) -> bool {
    if index >= config.deepl_keys.len() {
        false
    } else {
        config.deepl_keys.remove(index);
        true
    }
}

/// LLM 최대 토큰 수. 문자열을 파싱한 뒤 허용 범위로 clamp한다.
fn apply_llm_max_tokens(
    config: &mut TranslationConfig,
    value: String,
) -> Result<bool, TranslationSettingsError> {
    let value = parse_unsigned(&value, "max_tokens")?;
    Ok(set_if_changed(
        &mut config.llm.max_tokens,
        limits::llm_max_tokens(value),
    ))
}

/// LLM 디바운스(ms). 문자열을 파싱한 뒤 허용 범위로 clamp한다.
fn apply_llm_debounce(
    config: &mut TranslationConfig,
    value: String,
) -> Result<bool, TranslationSettingsError> {
    let value = parse_unsigned(&value, "debounce_ms")?;
    Ok(set_if_changed(
        &mut config.llm.debounce_ms,
        limits::llm_debounce_ms(value),
    ))
}

/// 직접 입력한 temperature. 슬라이더 눈금으로 환산해 슬라이더 입력과 같은
/// 양자화를 거치게 한다.
fn apply_llm_temperature_text(
    config: &mut TranslationConfig,
    value: String,
) -> Result<bool, TranslationSettingsError> {
    let value = parse_finite_float(&value, "temperature")?;
    let slider_value = limits::llm_temperature_to_slider(value);
    Ok(set_if_changed(
        &mut config.llm.temperature,
        limits::llm_temperature_slider(slider_value),
    ))
}

/// 슬라이더로 조정한 temperature.
fn apply_llm_temperature_slider(config: &mut TranslationConfig, value: i32) -> bool {
    set_if_changed(
        &mut config.llm.temperature,
        limits::llm_temperature_slider(value),
    )
}

/// 설정에 적힌 EzTrans 경로가 `valid` 조건을 만족하지 못하면 `true`.
/// 상대 경로는 런타임과 동일하게 데이터 디렉터리 기준으로 해석한다.
fn path_invalid(configured: &str, valid: fn(&std::path::Path) -> bool) -> bool {
    let configured = configured.trim();
    configured.is_empty()
        || !valid(std::path::Path::new(
            &crate::translation::resolve_configured_eztrans_path(configured),
        ))
}

fn parse_finite_float(value: &str, field: &'static str) -> Result<f32, TranslationSettingsError> {
    value
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or(TranslationSettingsError::InvalidFloatingPoint { field })
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

#[cfg(test)]
#[path = "../../tests/unit/translation/settings_extra.rs"]
mod tests_extra;
