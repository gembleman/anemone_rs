use super::*;
use crate::translation::TranslationError;

#[test]
fn rejects_missing_credentials_without_exposing_values() {
    let mut config = TranslationConfig {
        engine: "deepl".into(),
        ..TranslationConfig::default()
    };
    config.deepl_api_key.clear();
    config.deepl_keys.clear();
    assert_eq!(
        PreparedJob::from_config(&config).err(),
        Some(TranslationConfigError::MissingCredential("DeepL API 키"))
    );
}

#[test]
fn from_config_cached_reuses_the_job_for_unchanged_config() {
    let mut config = TranslationConfig {
        engine: "eztrans".into(),
        eztrans_postprocess_dictionary: vec![EzTransPostprocessEntry {
            source: "foo".into(),
            target: "bar".into(),
        }],
        ..TranslationConfig::default()
    };
    config.eztrans_dictionary_path = "engine.dll".into();
    config.eztrans_dat_path = "dat".into();

    let first = PreparedJob::from_config_cached(&config).unwrap();
    let second = PreparedJob::from_config_cached(&config).unwrap();
    assert!(
        Arc::ptr_eq(&first, &second),
        "같은 설정은 같은 PreparedJob 인스턴스를 재사용해야 합니다"
    );
    // engine_id는 캐시된 인스턴스 안에서 메모이즈되어 사전 해시가 재계산되지 않는다.
    let id = first.engine().cache_engine_id();
    assert_eq!(second.engine().cache_engine_id(), id);
    assert_ne!(id, "eztrans");
}

#[test]
fn from_config_cached_rebuilds_when_the_dictionary_changes() {
    let mut config = TranslationConfig {
        engine: "eztrans".into(),
        ..TranslationConfig::default()
    };
    config.eztrans_dictionary_path = "engine.dll".into();
    config.eztrans_dat_path = "dat".into();
    config.eztrans_postprocess_dictionary = vec![EzTransPostprocessEntry {
        source: "A".into(),
        target: "B".into(),
    }];

    let with_dictionary_a = PreparedJob::from_config_cached(&config).unwrap();
    config.eztrans_postprocess_dictionary = vec![EzTransPostprocessEntry {
        source: "C".into(),
        target: "D".into(),
    }];
    let with_dictionary_b = PreparedJob::from_config_cached(&config).unwrap();

    assert!(
        !Arc::ptr_eq(&with_dictionary_a, &with_dictionary_b),
        "사전이 바뀌면 새로 빌드해야 합니다"
    );
    assert_ne!(
        with_dictionary_a.engine().cache_engine_id(),
        with_dictionary_b.engine().cache_engine_id()
    );
}

#[test]
fn from_config_cached_rebuilds_when_llm_model_changes() {
    let mut config = TranslationConfig {
        engine: "llm".into(),
        llm: crate::config::LlmConfig {
            model: "model-a".into(),
            api_key: "key".into(),
            ..crate::config::LlmConfig::default()
        },
        ..TranslationConfig::default()
    };

    let model_a = PreparedJob::from_config_cached(&config).unwrap();
    config.llm.model = "model-b".into();
    let model_b = PreparedJob::from_config_cached(&config).unwrap();

    assert!(!Arc::ptr_eq(&model_a, &model_b));
    assert_eq!(model_a.engine().cache_engine_id(), "llm:model-a");
    assert_eq!(model_b.engine().cache_engine_id(), "llm:model-b");
}

#[test]
fn validates_eztrans_paths_and_language_pair() {
    let mut config = TranslationConfig {
        engine: "eztrans".into(),
        ..TranslationConfig::default()
    };
    config.eztrans_dictionary_path.clear();
    assert_eq!(
        PreparedJob::from_config(&config).err(),
        Some(TranslationConfigError::MissingEzTransPath)
    );
    let result = PreparedJob::with_engine_languages(
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
    let spec = PreparedJob::from_config(&config).unwrap();
    assert_eq!(spec.engine().engine(), TranslationEngine::DeepL);
    assert!(
        matches!(spec.engine().kind(), PreparedEngineKind::DeepL { keys, .. }
            if keys.iter().map(String::as_str).eq(["first", "second"]))
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

    let job = PreparedJob::from_config(&config).unwrap();
    let job_debug = format!("{job:?}");
    assert!(!job_debug.contains(secret));
    assert!(job_debug.contains("Llm"));
    let PreparedEngineKind::Llm(params) = job.engine().kind() else {
        panic!("expected LLM credentials");
    };
    let debug = format!("{params:?}");

    assert!(!debug.contains(secret));
    assert!(debug.contains("[REDACTED]"));
}

#[test]
fn prepared_engine_exposes_capabilities_without_credentials() {
    let google = PreparedJob::google(Language::Jpn, Language::Kor).unwrap();
    assert!(!google.engine().is_blocking());
    assert!(!google.engine().supports_batch());
    assert_eq!(
        google.languages(),
        LanguagePair::new(Language::Jpn, Language::Kor)
    );

    let eztrans = PreparedJob::eztrans(
        "engine.dll".into(),
        "dat".into(),
        99,
        Language::Jpn,
        Language::Kor,
    )
    .unwrap();
    assert!(eztrans.engine().is_blocking());
    assert!(eztrans.engine().supports_batch());
    assert_eq!(
        eztrans.engine().eztrans_process().unwrap().process_count,
        16
    );
}

#[test]
fn file_job_carries_normalized_eztrans_process_configuration() {
    let config = TranslationConfig {
        eztrans_process_count: 99,
        ..TranslationConfig::default()
    };
    let spec = PreparedJob::from_config(&config).unwrap();
    assert_eq!(spec.engine().engine(), TranslationEngine::EzTrans);
    let process = spec.engine().eztrans_process().unwrap();
    assert_eq!(process.process_count, 16);
    assert_eq!(process.dictionary_path, config.eztrans_dictionary_path);
    assert_eq!(process.dat_path, config.eztrans_dat_path);
}

#[test]
fn eztrans_job_carries_and_applies_its_own_postprocess_dictionary() {
    let mut config = TranslationConfig::default();
    config.eztrans_postprocess_dictionary = vec![crate::config::EzTransPostprocessEntry {
        source: "번역 전".into(),
        target: "번역 후".into(),
    }];
    let job = PreparedJob::from_config(&config).unwrap();

    assert_eq!(job.postprocess("번역 전 문장".into()), "번역 후 문장");
    assert_ne!(job.engine().cache_engine_id(), "eztrans");

    config.eztrans_postprocess_dictionary.clear();
    let without_dictionary = PreparedJob::from_config(&config).unwrap();
    assert_eq!(without_dictionary.postprocess("번역 전".into()), "번역 전");
    assert_eq!(without_dictionary.engine().cache_engine_id(), "eztrans");
}

#[test]
fn relative_eztrans_paths_resolve_from_the_executable_data_directory() {
    let config = TranslationConfig {
        eztrans_dictionary_path: r"eztrans_dll\JisJK.flat.bin".into(),
        eztrans_dat_path: r"eztrans_dll\Dat".into(),
        ..TranslationConfig::default()
    };

    let job = PreparedJob::from_config(&config).unwrap();
    let process = job.engine().eztrans_process().unwrap();

    assert_eq!(
        process.dictionary_path,
        crate::runtime::data_dir()
            .join(r"eztrans_dll\JisJK.flat.bin")
            .to_string_lossy()
    );
    assert_eq!(
        process.dat_path,
        crate::runtime::data_dir()
            .join(r"eztrans_dll\Dat")
            .to_string_lossy()
    );
    assert_eq!(
        config.eztrans_dictionary_path,
        r"eztrans_dll\JisJK.flat.bin"
    );
    assert_eq!(config.eztrans_dat_path, r"eztrans_dll\Dat");
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
    let spec = PreparedJob::from_config(&config).unwrap();
    assert_eq!(spec.engine().engine(), TranslationEngine::Custom);
    assert!(matches!(
        spec.engine().kind(),
        PreparedEngineKind::Custom(_)
    ));

    config.custom.request_template = "invalid json".into();
    assert!(matches!(
        PreparedJob::from_config(&config),
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
    let spec = PreparedJob::from_config(&config).unwrap();
    assert!(matches!(
        spec.engine().kind(),
        PreparedEngineKind::Custom(params)
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
        PreparedJob::from_config(&config),
        Err(TranslationConfigError::InvalidSetting(message)) if message.contains("중복")
    ));

    config.custom_apis[1].name = "other".into();
    config.custom_api = "missing".into();
    assert!(matches!(
        PreparedJob::from_config(&config),
        Err(TranslationConfigError::InvalidSetting(message)) if message.contains("찾을 수 없습니다")
    ));
}

#[test]
fn cloned_prepared_jobs_share_credentials_without_reallocating() {
    let mut config = TranslationConfig {
        engine: "llm".into(),
        ..TranslationConfig::default()
    };
    config.llm.api_key = "secret".repeat(32);
    config.llm.system_prompt = "prompt".repeat(256);

    let spec = PreparedJob::from_config(&config).unwrap();
    let prompt_ptr = match spec.engine().kind() {
        PreparedEngineKind::Llm(parameters) => parameters.system_prompt.as_ptr(),
        _ => panic!("expected LLM credentials"),
    };
    let cloned = spec.clone();
    let moved_ptr = match cloned.engine().kind() {
        PreparedEngineKind::Llm(parameters) => parameters.system_prompt.as_ptr(),
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
            PreparedJob::with_engine_languages(&config, TranslationEngine::Papago, source, target,)
                .is_ok()
        );
        assert!(
            PreparedJob::with_engine_languages(&config, TranslationEngine::Papago, target, source,)
                .is_ok()
        );
    }

    for (source, target) in [
        (Language::Deu, Language::Ita),
        (Language::Rus, Language::Spa),
        (Language::Eng, Language::Eng),
    ] {
        assert_eq!(
            PreparedJob::with_engine_languages(&config, TranslationEngine::Papago, source, target,)
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
        PreparedJob::with_engine_languages(
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
            PreparedJob::from_config(&config),
            Err(TranslationConfigError::InvalidSetting(_))
        ));
    }
}
