use super::*;
use crate::translation::TranslationError;
use secrecy::ExposeSecret;

#[test]
fn rejects_missing_credentials_without_exposing_values() {
    let mut config = TranslationConfig {
        engine: "deepl".into(),
        ..TranslationConfig::default()
    };
    config.deepl_api_key.clear();
    config.deepl_keys.clear();
    assert_eq!(
        TranslationJobSpec::from_config(&config).err(),
        Some(TranslationConfigError::MissingCredential("DeepL API 키"))
    );
}

#[test]
fn validates_eztrans_paths_and_language_pair() {
    let mut config = TranslationConfig {
        engine: "eztrans".into(),
        ..TranslationConfig::default()
    };
    config.eztrans_dll_path.clear();
    assert_eq!(
        TranslationJobSpec::from_config(&config).err(),
        Some(TranslationConfigError::MissingEzTransPath)
    );
    let result = TranslationJobSpec::with_engine_languages(
        &TranslationConfig::default(),
        TranslationEngine::EzTrans,
        Language::Kor,
        Language::Jpn,
    );
    assert_eq!(
        result.err(),
        Some(TranslationConfigError::UnsupportedLanguagePair { engine: "eztrans" })
    );
}

#[test]
fn builds_deepl_credentials_from_the_effective_key_list() {
    let config = TranslationConfig {
        engine: "deepl".into(),
        deepl_keys: vec!["first".into(), "second".into()],
        ..TranslationConfig::default()
    };
    let spec = TranslationJobSpec::from_config(&config).unwrap();
    assert_eq!(spec.engine(), TranslationEngine::DeepL);
    assert!(
        matches!(spec.credentials(), EngineCredentials::DeepL { keys, .. }
            if keys.iter().map(|key| key.expose_secret()).eq(["first", "second"]))
    );
}

#[test]
fn runtime_llm_params_redact_api_key_from_debug_output() {
    let secret = "debug-output-must-not-contain-this-key";
    let config = TranslationConfig {
        engine: "llm".into(),
        llm: crate::config::LlmConfig {
            api_key: secret.into(),
            ..crate::config::LlmConfig::default()
        },
        ..TranslationConfig::default()
    };

    let credentials = TranslationJobSpec::from_config(&config)
        .unwrap()
        .credentials();
    let EngineCredentials::Llm(params) = credentials else {
        panic!("expected LLM credentials");
    };
    let debug = format!("{params:?}");

    assert!(!debug.contains(secret));
    assert!(debug.contains("[REDACTED]"));
}

#[test]
fn file_job_carries_normalized_eztrans_process_configuration() {
    let config = TranslationConfig {
        eztrans_process_count: 99,
        ..TranslationConfig::default()
    };
    let spec = TranslationJobSpec::from_config(&config).unwrap();
    let (engine, _, _, _, process) = spec.into_file_parts();
    assert_eq!(engine, TranslationEngine::EzTrans);
    let process = process.unwrap();
    assert_eq!(process.process_count, 16);
    assert_eq!(process.dll_path, config.eztrans_dll_path);
    assert_eq!(process.dat_path, config.eztrans_dat_path);
}

#[test]
fn builds_and_validates_custom_api_credentials() {
    let mut config = TranslationConfig {
        engine: "custom".into(),
        ..TranslationConfig::default()
    };
    config.custom.url = "https://example.com/translate".into();
    config.custom.api_key = "secret".into();
    config.custom.headers = r#"{"X-Source":"{source}"}"#.into();
    config.custom.request_template = r#"{"q":"{text}"}"#.into();
    let spec = TranslationJobSpec::from_config(&config).unwrap();
    assert_eq!(spec.engine(), TranslationEngine::Custom);
    assert!(matches!(spec.credentials(), EngineCredentials::Custom(_)));

    config.custom.request_template = "invalid json".into();
    assert!(matches!(
        TranslationJobSpec::from_config(&config),
        Err(TranslationConfigError::InvalidSetting(_))
    ));
}

#[test]
fn builds_credentials_from_the_selected_named_custom_api() {
    let primary = crate::config::CustomApiConfig {
        name: "primary".into(),
        url: "https://primary.example/translate".into(),
        ..Default::default()
    };
    let backup = crate::config::CustomApiConfig {
        name: "backup".into(),
        url: "https://backup.example/translate".into(),
        ..Default::default()
    };

    let config = TranslationConfig {
        engine: "custom".into(),
        custom_api: "backup".into(),
        custom_apis: vec![primary, backup],
        ..TranslationConfig::default()
    };
    let spec = TranslationJobSpec::from_config(&config).unwrap();
    assert!(matches!(
        spec.credentials(),
        EngineCredentials::Custom(params)
            if params.url == "https://backup.example/translate"
    ));
}

#[test]
fn rejects_ambiguous_or_missing_named_custom_api_selection() {
    let first = crate::config::CustomApiConfig {
        name: "same".into(),
        url: "https://first.example/translate".into(),
        ..Default::default()
    };
    let mut second = first.clone();
    second.url = "https://second.example/translate".into();

    let mut config = TranslationConfig {
        engine: "custom".into(),
        custom_api: "same".into(),
        custom_apis: vec![first, second],
        ..TranslationConfig::default()
    };
    assert!(matches!(
        TranslationJobSpec::from_config(&config),
        Err(TranslationConfigError::InvalidSetting(message)) if message.contains("중복")
    ));

    config.custom_apis[1].name = "other".into();
    config.custom_api = "missing".into();
    assert!(matches!(
        TranslationJobSpec::from_config(&config),
        Err(TranslationConfigError::InvalidSetting(message)) if message.contains("찾을 수 없습니다")
    ));
}

#[test]
fn consuming_job_spec_moves_credentials_without_reallocating() {
    let mut config = TranslationConfig {
        engine: "llm".into(),
        ..TranslationConfig::default()
    };
    config.llm.api_key = "secret".repeat(32);
    config.llm.system_prompt = "prompt".repeat(256);

    let spec = TranslationJobSpec::from_config(&config).unwrap();
    let prompt_ptr = match &spec.credentials {
        EngineCredentials::Llm(parameters) => parameters.system_prompt.as_ptr(),
        _ => panic!("expected LLM credentials"),
    };
    let (_, _, _, credentials) = spec.into_parts();
    let moved_ptr = match &credentials {
        EngineCredentials::Llm(parameters) => parameters.system_prompt.as_ptr(),
        _ => panic!("expected LLM credentials"),
    };

    assert_eq!(prompt_ptr, moved_ptr);
}

#[test]
fn validates_papago_pairs_from_the_ncloud_contract() {
    let config = TranslationConfig {
        papago_client_id: "id".into(),
        papago_client_secret: "secret".into(),
        ..TranslationConfig::default()
    };

    for (source, target) in [
        (Language::Kor, Language::Ita),
        (Language::Eng, Language::Deu),
        (Language::Jpn, Language::Vie),
    ] {
        assert!(
            TranslationJobSpec::with_engine_languages(
                &config,
                TranslationEngine::Papago,
                source,
                target,
            )
            .is_ok()
        );
        assert!(
            TranslationJobSpec::with_engine_languages(
                &config,
                TranslationEngine::Papago,
                target,
                source,
            )
            .is_ok()
        );
    }

    for (source, target) in [
        (Language::Deu, Language::Ita),
        (Language::Rus, Language::Spa),
        (Language::Eng, Language::Eng),
    ] {
        assert_eq!(
            TranslationJobSpec::with_engine_languages(
                &config,
                TranslationEngine::Papago,
                source,
                target,
            )
            .err(),
            Some(TranslationConfigError::UnsupportedLanguagePair { engine: "papago" })
        );
    }
}

#[test]
fn papago_contract_contains_exactly_29_unordered_pairs() {
    let languages = TranslationEngine::Papago.supported_source_languages();
    let count = languages
        .iter()
        .enumerate()
        .flat_map(|(index, &source)| {
            languages[index + 1..]
                .iter()
                .copied()
                .map(move |target| (source, target))
        })
        .filter(|&(source, target)| TranslationEngine::Papago.supports_pair(source, target))
        .count();
    assert_eq!(count, 29);
}

#[test]
fn retries_anthropic_overload_status() {
    let error = TranslationError::Api {
        code: 529,
        message: "overloaded".into(),
        retry_after: None,
    };
    assert!(error.is_retryable());
}

#[test]
fn preserves_chinese_script_tags_and_papago_pair() {
    assert_eq!(
        crate::translation::lang_utils::from_code("zh-Hans"),
        Some(Language::ZhoHans)
    );
    assert_eq!(
        crate::translation::lang_utils::from_code("zh-TW"),
        Some(Language::ZhoHant)
    );
    assert_eq!(
        crate::translation::lang_utils::to_papago_code(Language::ZhoHant).unwrap(),
        "zh-TW"
    );

    let config = TranslationConfig {
        papago_client_id: "id".into(),
        papago_client_secret: "secret".into(),
        ..TranslationConfig::default()
    };
    assert!(
        TranslationJobSpec::with_engine_languages(
            &config,
            TranslationEngine::Papago,
            Language::ZhoHans,
            Language::ZhoHant,
        )
        .is_ok()
    );
}

#[test]
fn job_spec_rejects_invalid_config_values() {
    for config in [
        TranslationConfig {
            engine: "typo-engine".into(),
            ..TranslationConfig::default()
        },
        TranslationConfig {
            source_lang: "not-a-language".into(),
            ..TranslationConfig::default()
        },
        TranslationConfig {
            engine: "llm".into(),
            llm: crate::config::LlmConfig {
                provider: "typo-provider".into(),
                api_key: "key".into(),
                ..crate::config::LlmConfig::default()
            },
            ..TranslationConfig::default()
        },
    ] {
        assert!(matches!(
            TranslationJobSpec::from_config(&config),
            Err(TranslationConfigError::InvalidSetting(_))
        ));
    }
}
