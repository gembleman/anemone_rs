use super::*;

#[test]
fn clamps_numeric_inputs_and_reports_refresh_policy() {
    let mut config = TranslationConfig::default();
    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::LlmMaxTokensText("999999".into()),
    )
    .unwrap();
    assert_eq!(config.llm.max_tokens, 32_000);
    assert_eq!(
        result,
        SettingsApplyResult {
            changed: true,
            save_required: true,
            preview_refresh_required: true,
            runtime_sync_required: false
        }
    );
}

#[test]
fn invalid_numeric_inputs_leave_config_unchanged() {
    let mut config = TranslationConfig::default();
    let max_tokens = config.llm.max_tokens;
    let debounce_ms = config.llm.debounce_ms;

    assert_eq!(
        TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::LlmMaxTokensText("invalid".into())
        ),
        Err(TranslationSettingsError::InvalidUnsignedInteger {
            field: "max_tokens"
        })
    );
    assert_eq!(
        TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::LlmDebounceText("-1".into())
        ),
        Err(TranslationSettingsError::InvalidUnsignedInteger {
            field: "debounce_ms"
        })
    );
    assert_eq!(config.llm.max_tokens, max_tokens);
    assert_eq!(config.llm.debounce_ms, debounce_ms);
}

#[test]
fn engine_and_eztrans_changes_request_runtime_sync() {
    let mut config = TranslationConfig::default();
    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::Engine(TranslationEngine::Google),
    )
    .unwrap();
    assert!(result.runtime_sync_required);
    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::DeepLApiKey("secret".into()),
    )
    .unwrap();
    assert!(!result.runtime_sync_required);
}

#[test]
fn auxiliary_deepl_keys_are_normalized_and_kept_unique() {
    let mut config = TranslationConfig::default();
    assert!(
        TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::AddDeepLKey(" key ".into())
        )
        .unwrap()
        .changed
    );
    assert_eq!(config.deepl_keys, ["key"]);
    assert!(
        !TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::AddDeepLKey("key".into())
        )
        .unwrap()
        .changed
    );
    assert!(
        TranslationSettingsEditor::apply(&mut config, TranslationSettingChange::RemoveDeepLKey(0))
            .unwrap()
            .changed
    );
    assert!(config.deepl_keys.is_empty());
}

#[test]
fn engine_change_normalizes_unsupported_languages() {
    let mut config = TranslationConfig {
        engine: "deepl".into(),
        source_lang: "en".into(),
        target_lang: "fr".into(),
        ..TranslationConfig::default()
    };

    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::Engine(TranslationEngine::EzTrans),
    )
    .unwrap();

    assert_eq!(config.engine, "eztrans");
    assert_eq!(config.source_lang, "ja");
    assert_eq!(config.target_lang, "ko");
    assert_eq!(
        result,
        SettingsApplyResult {
            changed: true,
            save_required: true,
            preview_refresh_required: true,
            runtime_sync_required: true,
        }
    );
}

#[test]
fn engine_change_preserves_supported_languages() {
    let mut config = TranslationConfig {
        engine: "eztrans".into(),
        source_lang: "ja".into(),
        target_lang: "ko".into(),
        ..TranslationConfig::default()
    };

    TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::Engine(TranslationEngine::DeepL),
    )
    .unwrap();

    assert_eq!(config.source_lang, "ja");
    assert_eq!(config.target_lang, "ko");
}

#[test]
fn normalized_engine_change_builds_translation_job_spec() {
    let mut config = TranslationConfig {
        engine: "deepl".into(),
        source_lang: "en".into(),
        target_lang: "fr".into(),
        ..TranslationConfig::default()
    };

    TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::Engine(TranslationEngine::EzTrans),
    )
    .unwrap();

    assert!(TranslationJobSpec::from_config(&config).is_ok());
}

#[test]
fn identical_engine_language_and_string_changes_have_no_follow_up_policy() {
    let mut config = TranslationConfig::default();

    for change in [
        TranslationSettingChange::Engine(TranslationEngine::EzTrans),
        TranslationSettingChange::SourceLanguage(Language::Jpn),
        TranslationSettingChange::TargetLanguage(Language::Kor),
        TranslationSettingChange::LlmModel(config.llm.model.clone()),
    ] {
        assert_eq!(
            TranslationSettingsEditor::apply(&mut config, change).unwrap(),
            SettingsApplyResult::default()
        );
    }
}
