//! `settings.rs`가 500줄 제한을 넘지 않도록 나눈 추가 테스트.
//! 단순 필드 설정, 언어 정규화 오류 경로, `sync_runtime`을 다룬다.
use super::*;

/// 이 테스트들이 만드는 임시 디렉터리. 케이스마다 다른 이름을 써서 병렬 실행에도 안전하다.
fn eztrans_fixture_dir(case: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("anemone_eztrans_extra_{case}"));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn plain_string_fields_are_stored_verbatim_and_report_the_change() {
    let mut config = TranslationConfig::default();

    let changes = [
        TranslationSettingChange::PapagoClientId("client-id".into()),
        TranslationSettingChange::PapagoClientSecret("client-secret".into()),
        TranslationSettingChange::MysTranslaterApiKey("token".into()),
        TranslationSettingChange::LlmApiKey("llm-key".into()),
        TranslationSettingChange::LlmSystemPrompt("system".into()),
    ];
    for change in changes {
        let result = TranslationSettingsEditor::apply(&mut config, change).unwrap();
        assert!(result.changed);
        assert!(!result.runtime_sync_required);
    }

    assert_eq!(config.papago_client_id, "client-id");
    assert_eq!(config.papago_client_secret, "client-secret");
    assert_eq!(config.mys_translater_api_key, "token");
    assert_eq!(config.llm.api_key, "llm-key");
    assert_eq!(config.llm.system_prompt, "system");
}

#[test]
fn eztrans_path_changes_request_runtime_sync_via_apply() {
    let mut config = TranslationConfig::default();

    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::EzTransDictionaryPath("C:/dict/JisJK.flat.bin".into()),
    )
    .unwrap();
    assert!(result.changed);
    assert!(result.runtime_sync_required);
    assert_eq!(config.eztrans_dictionary_path, "C:/dict/JisJK.flat.bin");

    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::EzTransEhndPath("C:/dict/Ehnd".into()),
    )
    .unwrap();
    assert!(result.changed);
    assert!(result.runtime_sync_required);
    assert_eq!(config.eztrans_ehnd_path, "C:/dict/Ehnd");
}

#[test]
fn deepl_strategy_toggle_switches_between_round_robin_and_failover() {
    let mut config = TranslationConfig::default();

    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::DeepLStrategyRoundRobin(true),
    )
    .unwrap();
    assert!(result.changed);
    assert_eq!(config.deepl_strategy, "round-robin");

    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::DeepLStrategyRoundRobin(false),
    )
    .unwrap();
    assert!(result.changed);
    assert_eq!(config.deepl_strategy, "failover");
}

#[test]
fn llm_temperature_slider_clamps_and_stores_the_quantized_value() {
    let mut config = TranslationConfig::default();

    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::LlmTemperatureSlider(999),
    )
    .unwrap();
    assert!(result.changed);
    assert_eq!(config.llm.temperature, 2.0);
}

#[test]
fn llm_debounce_text_parses_and_clamps_like_max_tokens() {
    let mut config = TranslationConfig::default();

    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::LlmDebounceText("50".into()),
    )
    .unwrap();
    assert!(result.changed);
    assert_eq!(
        config.llm.debounce_ms,
        crate::config::limits::llm_debounce_ms(50)
    );
}

#[test]
fn source_language_change_rejects_a_language_the_engine_does_not_support() {
    let mut config = TranslationConfig {
        engine: "papago".into(),
        source_lang: "ko".into(),
        target_lang: "en".into(),
        ..TranslationConfig::default()
    };

    assert_eq!(
        TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::SourceLanguage(Language::Por),
        ),
        Err(TranslationSettingsError::UnsupportedSourceLanguage)
    );
    assert_eq!(config.source_lang, "ko");
}

#[test]
fn source_language_change_renormalizes_the_target_when_the_pair_is_unsupported() {
    // Papago는 Kor/Eng/Jpn/Zho 앵커 없이는 임의의 두 언어를 직접 잇지 않는다.
    // Deu와 Ita는 각각 개별적으로 지원되지만 Deu<->Ita 조합 자체는 지원하지 않는다.
    let mut config = TranslationConfig {
        engine: "papago".into(),
        source_lang: "ko".into(),
        target_lang: "it".into(),
        ..TranslationConfig::default()
    };

    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::SourceLanguage(Language::Deu),
    )
    .unwrap();

    assert!(result.changed);
    assert_eq!(config.source_lang, "de");
    // Deu와 직접 페어링되는 언어는 Kor뿐이므로 대상 언어가 자동으로 교정된다.
    assert_eq!(config.target_lang, "ko");
}

#[test]
fn target_language_change_rejects_a_language_the_pair_does_not_support() {
    let mut config = TranslationConfig {
        engine: "papago".into(),
        source_lang: "ko".into(),
        target_lang: "en".into(),
        ..TranslationConfig::default()
    };

    assert_eq!(
        TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::TargetLanguage(Language::Por),
        ),
        Err(TranslationSettingsError::UnsupportedTargetLanguage)
    );
    assert_eq!(config.target_lang, "en");
}

#[test]
fn target_language_change_accepts_a_supported_pair() {
    let mut config = TranslationConfig {
        engine: "papago".into(),
        source_lang: "ko".into(),
        target_lang: "en".into(),
        ..TranslationConfig::default()
    };

    let result = TranslationSettingsEditor::apply(
        &mut config,
        TranslationSettingChange::TargetLanguage(Language::Jpn),
    )
    .unwrap();
    assert!(result.changed);
    assert_eq!(config.target_lang, "ja");
}

#[test]
fn language_changes_report_invalid_current_config_when_the_engine_string_is_unparseable() {
    let mut config = TranslationConfig {
        engine: "not-a-real-engine".into(),
        ..TranslationConfig::default()
    };

    assert!(matches!(
        TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::SourceLanguage(Language::Jpn),
        ),
        Err(TranslationSettingsError::InvalidCurrentConfig(_))
    ));
    assert!(matches!(
        TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::TargetLanguage(Language::Kor),
        ),
        Err(TranslationSettingsError::InvalidCurrentConfig(_))
    ));
}

#[test]
fn llm_provider_change_reports_invalid_current_config_when_the_provider_string_is_unparseable() {
    let mut config = TranslationConfig::default();
    config.llm.provider = "not-a-real-provider".into();

    assert!(matches!(
        TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::LlmProvider(LlmProvider::Anthropic),
        ),
        Err(TranslationSettingsError::InvalidCurrentConfig(_))
    ));
}

#[test]
fn selecting_a_missing_custom_api_reports_invalid_current_config() {
    let mut config = TranslationConfig {
        custom_api: "first".into(),
        custom_apis: vec![crate::config::CustomApiConfig {
            name: "first".into(),
            ..Default::default()
        }],
        ..TranslationConfig::default()
    };

    assert!(matches!(
        TranslationSettingsEditor::apply(
            &mut config,
            TranslationSettingChange::SelectCustomApi("missing".into()),
        ),
        Err(TranslationSettingsError::InvalidCurrentConfig(_))
    ));
}

#[test]
fn sync_runtime_is_a_no_op_for_non_eztrans_engines() {
    let config = TranslationConfig {
        engine: "google".into(),
        ..TranslationConfig::default()
    };
    assert!(TranslationSettingsEditor::sync_runtime(&config).is_ok());
}

#[test]
fn sync_runtime_reports_missing_eztrans_paths_as_an_error() {
    let config = TranslationConfig {
        engine: "eztrans".into(),
        eztrans_dictionary_path: String::new(),
        eztrans_ehnd_path: String::new(),
        ..TranslationConfig::default()
    };
    assert!(TranslationSettingsEditor::sync_runtime(&config).is_err());
}

#[test]
fn sync_runtime_reports_a_nonexistent_eztrans_dictionary_as_an_error() {
    let directory = eztrans_fixture_dir("sync_runtime_missing_dictionary");
    let config = TranslationConfig {
        engine: "eztrans".into(),
        eztrans_dictionary_path: directory
            .join("JisJK.flat.bin")
            .to_string_lossy()
            .into_owned(),
        eztrans_ehnd_path: directory.join("Ehnd").to_string_lossy().into_owned(),
        ..TranslationConfig::default()
    };
    // 사전/Ehnd 파일이 실제로 존재하지 않으므로 PreparedJob 준비 단계에서 실패해야 한다.
    assert!(TranslationSettingsEditor::sync_runtime(&config).is_err());
    let _ = std::fs::remove_dir_all(&directory);
}
