use super::*;

#[test]
fn clamps_numeric_inputs_and_reports_domain_change() {
    let mut config = TranslationConfig::default();
    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::LlmMaxTokensText("999999".into()),
    )
    .unwrap();
    assert_eq!(config.llm.max_tokens, 32_000);
    assert_eq!(
        result,
        TranslationSettingsChangeResult {
            changed: true,
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
        TranslationSettingChange::AddDeepLKey {
            tier: DeepLApiTier::Pro,
            key: "secret".into(),
        },
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
            TranslationSettingChange::AddDeepLKey {
                tier: DeepLApiTier::Pro,
                key: " key ".into(),
            }
        )
        .unwrap()
        .changed
    );
    assert_eq!(config.deepl_keys, ["key"]);
    assert!(
        !TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::AddDeepLKey {
                tier: DeepLApiTier::Pro,
                key: "key".into(),
            }
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
fn temperature_text_updates_and_clamps_to_the_slider_range() {
    let mut config = TranslationConfig::default();

    TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::LlmTemperatureText("0.726".into()),
    )
    .unwrap();
    assert_eq!(config.llm.temperature, 0.73);

    TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::LlmTemperatureText("9".into()),
    )
    .unwrap();
    assert_eq!(config.llm.temperature, 2.0);
}

#[test]
fn invalid_temperature_text_leaves_config_unchanged() {
    let mut config = TranslationConfig::default();
    let temperature = config.llm.temperature;

    assert_eq!(
        TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::LlmTemperatureText("NaN".into()),
        ),
        Err(TranslationSettingsError::InvalidFloatingPoint {
            field: "temperature"
        })
    );
    assert_eq!(config.llm.temperature, temperature);
}

#[test]
fn reasoning_effort_selection_updates_llm_config() {
    use crate::translation::llm::ReasoningEffort;

    let mut config = TranslationConfig::default();
    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::LlmReasoningEffort(Some(ReasoningEffort::High)),
    )
    .unwrap();
    assert!(result.changed);
    assert_eq!(config.llm.reasoning_effort, Some(ReasoningEffort::High));

    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::LlmReasoningEffort(None),
    )
    .unwrap();
    assert!(result.changed);
    assert_eq!(config.llm.reasoning_effort, None);
}

#[test]
fn llm_provider_change_restores_each_providers_settings() {
    use crate::config::LlmGlossaryEntry;
    use crate::translation::llm::ReasoningEffort;

    let mut config = TranslationConfig::default();
    config.llm.model = "openai-model".into();
    config.llm.api_key = "openai-key".into();
    config.llm.base_url = "https://openai.example/v1".into();
    config.llm.system_prompt = "openai prompt".into();
    config.llm.temperature = 0.21;
    config.llm.top_p = 0.81;
    config.llm.frequency_penalty = 0.31;
    config.llm.presence_penalty = -0.41;
    config.llm.max_tokens = 2_001;
    config.llm.reasoning_effort = Some(ReasoningEffort::High);
    config.llm.glossary = vec![LlmGlossaryEntry {
        source: "OpenAI".into(),
        target: "오픈AI".into(),
    }];
    config.llm.debounce_ms = 201;

    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::LlmProvider(LlmProvider::Anthropic),
    )
    .unwrap();
    assert!(result.changed);
    assert_eq!(config.llm.model, "");
    assert_eq!(config.llm.api_key, "");
    assert_eq!(config.llm.temperature, 0.3);

    config.llm.model = "anthropic-model".into();
    config.llm.api_key = "anthropic-key".into();
    config.llm.base_url = "https://anthropic.example/v1".into();
    config.llm.system_prompt = "anthropic prompt".into();
    config.llm.temperature = 0.72;
    config.llm.top_p = 0.92;
    config.llm.frequency_penalty = 0.12;
    config.llm.presence_penalty = -0.22;
    config.llm.max_tokens = 4_002;
    config.llm.reasoning_effort = Some(ReasoningEffort::Low);
    config.llm.glossary = vec![LlmGlossaryEntry {
        source: "Claude".into(),
        target: "클로드".into(),
    }];
    config.llm.debounce_ms = 402;

    TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::LlmProvider(LlmProvider::OpenAi),
    )
    .unwrap();
    assert_eq!(config.llm.model, "openai-model");
    assert_eq!(config.llm.api_key, "openai-key");
    assert_eq!(config.llm.base_url, "https://openai.example/v1");
    assert_eq!(config.llm.system_prompt, "openai prompt");
    assert_eq!(config.llm.temperature, 0.21);
    assert_eq!(config.llm.top_p, 0.81);
    assert_eq!(config.llm.frequency_penalty, 0.31);
    assert_eq!(config.llm.presence_penalty, -0.41);
    assert_eq!(config.llm.max_tokens, 2_001);
    assert_eq!(config.llm.reasoning_effort, Some(ReasoningEffort::High));
    assert_eq!(config.llm.glossary[0].source, "OpenAI");
    assert_eq!(config.llm.debounce_ms, 201);

    TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::LlmProvider(LlmProvider::Anthropic),
    )
    .unwrap();
    assert_eq!(config.llm.model, "anthropic-model");
    assert_eq!(config.llm.api_key, "anthropic-key");
    assert_eq!(config.llm.base_url, "https://anthropic.example/v1");
    assert_eq!(config.llm.system_prompt, "anthropic prompt");
    assert_eq!(config.llm.temperature, 0.72);
    assert_eq!(config.llm.top_p, 0.92);
    assert_eq!(config.llm.frequency_penalty, 0.12);
    assert_eq!(config.llm.presence_penalty, -0.22);
    assert_eq!(config.llm.max_tokens, 4_002);
    assert_eq!(config.llm.reasoning_effort, Some(ReasoningEffort::Low));
    assert_eq!(config.llm.glossary[0].source, "Claude");
    assert_eq!(config.llm.debounce_ms, 402);
}

#[test]
fn deepl_key_type_must_match_the_key_suffix() {
    let mut config = TranslationConfig::default();
    assert_eq!(
        TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::AddDeepLKey {
                tier: DeepLApiTier::Free,
                key: "pro-key".into(),
            }
        ),
        Err(TranslationSettingsError::InvalidDeepLFreeKey)
    );
    assert_eq!(
        TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::AddDeepLKey {
                tier: DeepLApiTier::Pro,
                key: "free-key:fx".into(),
            }
        ),
        Err(TranslationSettingsError::InvalidDeepLProKey)
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
        TranslationSettingsChangeResult {
            changed: true,
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

    assert!(PreparedJob::from_config(&config).is_ok());
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
            TranslationSettingsChangeResult::default()
        );
    }
}

#[test]
fn custom_api_selection_changes_only_the_registered_name() {
    let first = crate::config::CustomApiConfig {
        name: "first".into(),
        ..Default::default()
    };
    let second = crate::config::CustomApiConfig {
        name: "second".into(),
        ..Default::default()
    };
    let mut config = TranslationConfig {
        custom_api: "first".into(),
        custom_apis: vec![first, second],
        ..TranslationConfig::default()
    };

    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::SelectCustomApi("second".into()),
    )
    .unwrap();

    assert_eq!(config.custom_api, "second");
    assert_eq!(config.custom_apis[0].name, "first");
    assert_eq!(config.custom_apis[1].name, "second");
    assert!(result.changed);
    assert!(!result.runtime_sync_required);
}

/// 이 테스트들이 만드는 임시 디렉터리. 케이스마다 다른 이름을 써서 병렬 실행에도 안전하다.
fn eztrans_fixture_dir(case: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("anemone_eztrans_{case}"));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn an_unset_eztrans_path_is_not_reported_as_invalid() {
    assert!(!TranslationSettingsEditor::eztrans_dictionary_invalid(""));
    assert!(!TranslationSettingsEditor::eztrans_dictionary_invalid(
        "   "
    ));
    assert!(!TranslationSettingsEditor::eztrans_dat_invalid(""));
    assert!(!TranslationSettingsEditor::eztrans_dat_invalid("   "));
}

#[test]
fn the_flat_dictionary_name_is_accepted_regardless_of_casing() {
    let directory = eztrans_fixture_dir("dictionary_ok");
    let dictionary = directory.join("JISJK.FLAT.BIN");
    std::fs::write(&dictionary, b"stub").unwrap();

    assert!(!TranslationSettingsEditor::eztrans_dictionary_invalid(
        &dictionary.to_string_lossy()
    ));

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_flat_file_with_an_unexpected_name_is_reported_as_invalid() {
    let directory = eztrans_fixture_dir("dictionary_wrong_name");
    let unrelated = directory.join("SomeOther.bin");
    std::fs::write(&unrelated, b"stub").unwrap();

    assert!(TranslationSettingsEditor::eztrans_dictionary_invalid(
        &unrelated.to_string_lossy()
    ));

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_directory_is_not_accepted_as_the_dictionary() {
    let directory = eztrans_fixture_dir("dictionary_is_dir");

    assert!(TranslationSettingsEditor::eztrans_dictionary_invalid(
        &directory.to_string_lossy()
    ));

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_dat_folder_holding_the_translation_dictionary_is_accepted() {
    let directory = eztrans_fixture_dir("dat_ok");
    std::fs::write(directory.join("JisJK.da"), b"stub").unwrap();

    assert!(!TranslationSettingsEditor::eztrans_dat_invalid(
        &directory.to_string_lossy()
    ));

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_folder_without_the_translation_dictionary_is_reported_as_invalid() {
    let directory = eztrans_fixture_dir("dat_no_dictionary");

    assert!(TranslationSettingsEditor::eztrans_dat_invalid(
        &directory.to_string_lossy()
    ));

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_file_is_not_accepted_as_the_dat_folder() {
    let directory = eztrans_fixture_dir("dat_is_file");
    let file = directory.join("JisJK.da");
    std::fs::write(&file, b"stub").unwrap();

    assert!(TranslationSettingsEditor::eztrans_dat_invalid(
        &file.to_string_lossy()
    ));

    let _ = std::fs::remove_dir_all(&directory);
}
