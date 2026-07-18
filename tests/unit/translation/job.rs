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
        matches!(spec.credentials(), EngineCredentials::DeepL { keys, .. } if keys == ["first", "second"])
    );
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
