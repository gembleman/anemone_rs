use super::*;

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
