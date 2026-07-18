use super::*;

#[test]
fn clamps_numeric_inputs_and_reports_refresh_policy() {
    let mut config = TranslationConfig::default();
    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::LlmMaxTokensText("999999".into()),
    );
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
    assert!(
        !TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::LlmMaxTokensText("invalid".into())
        )
        .changed
    );
}

#[test]
fn engine_and_eztrans_changes_request_runtime_sync() {
    let mut config = TranslationConfig::default();
    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::Engine(TranslationEngine::Google),
    );
    assert!(result.runtime_sync_required);
    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::DeepLApiKey("secret".into()),
    );
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
        .changed
    );
    assert_eq!(config.deepl_keys, ["key"]);
    assert!(
        !TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::AddDeepLKey("key".into())
        )
        .changed
    );
    assert!(
        TranslationSettingsEditor::apply(&mut config, TranslationSettingChange::RemoveDeepLKey(0))
            .changed
    );
    assert!(config.deepl_keys.is_empty());
}
