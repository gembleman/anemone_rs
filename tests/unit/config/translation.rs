use super::*;
use crate::translation::DeepLStrategy;

fn config_with_custom_apis(apis: Vec<CustomApiConfig>, selected: &str) -> TranslationConfig {
    TranslationConfig {
        custom_apis: apis,
        custom_api: selected.to_string(),
        ..TranslationConfig::default()
    }
}

fn named_api(name: &str) -> CustomApiConfig {
    CustomApiConfig {
        name: name.to_string(),
        ..CustomApiConfig::default()
    }
}

// ---- 기본값/직렬화 왕복 ----

#[test]
fn default_config_uses_eztrans_japanese_to_korean() {
    let config = TranslationConfig::default();
    assert_eq!(config.engine, "eztrans");
    assert_eq!(config.source_lang, "ja");
    assert_eq!(config.target_lang, "ko");
    assert_eq!(config.eztrans_process_count, 2);
    assert_eq!(config.deepl_strategy, "failover");
}

#[test]
fn unknown_engine_is_rejected_on_deserialize() {
    let result = toml::from_str::<TranslationConfig>("engine = \"bogus\"\n");
    assert!(result.is_err(), "알 수 없는 엔진 값은 거부되어야 합니다");
}

#[test]
fn unknown_language_code_is_rejected_on_deserialize() {
    let result = toml::from_str::<TranslationConfig>("source_lang = \"xx\"\n");
    assert!(result.is_err(), "알 수 없는 언어 코드는 거부되어야 합니다");

    let result = toml::from_str::<TranslationConfig>("target_lang = \"xx\"\n");
    assert!(result.is_err(), "알 수 없는 언어 코드는 거부되어야 합니다");
}

#[test]
fn legacy_eztrans_dll_path_key_is_read_as_dictionary_path() {
    let config =
        toml::from_str::<TranslationConfig>("eztrans_dll_path = \"C:/old/JisJK.flat.bin\"\n")
            .unwrap();
    assert_eq!(config.eztrans_dictionary_path, "C:/old/JisJK.flat.bin");
}

// ---- 구 eztrans_dat_path -> eztrans_ehnd_path 마이그레이션 ----

#[test]
fn legacy_dat_folder_migrates_to_sibling_ehnd_folder() {
    let mut config = TranslationConfig {
        eztrans_dat_path: Some("C:/eztrans_dll/Dat".to_string()),
        eztrans_ehnd_path: String::new(),
        ..TranslationConfig::default()
    };
    config.migrate_legacy_ehnd_path();
    let expected = std::path::Path::new("C:/eztrans_dll")
        .join("Ehnd")
        .to_string_lossy()
        .into_owned();
    assert_eq!(config.eztrans_ehnd_path, expected);
    assert_eq!(config.eztrans_dat_path, None);
}

#[test]
fn legacy_path_not_named_dat_is_kept_verbatim() {
    let mut config = TranslationConfig {
        eztrans_dat_path: Some("C:/eztrans_dll/CustomDict".to_string()),
        eztrans_ehnd_path: String::new(),
        ..TranslationConfig::default()
    };
    config.migrate_legacy_ehnd_path();
    assert_eq!(config.eztrans_ehnd_path, "C:/eztrans_dll/CustomDict");
}

#[test]
fn explicit_new_ehnd_path_is_never_overwritten_by_legacy_value() {
    let mut config = TranslationConfig {
        eztrans_dat_path: Some("C:/eztrans_dll/Dat".to_string()),
        eztrans_ehnd_path: "D:/user/Ehnd".to_string(),
        ..TranslationConfig::default()
    };
    config.migrate_legacy_ehnd_path();
    assert_eq!(config.eztrans_ehnd_path, "D:/user/Ehnd");
    assert_eq!(
        config.eztrans_dat_path, None,
        "새 키가 우선하더라도 소비된 구 키는 다시 저장되지 않아야 합니다"
    );
}

#[test]
fn migrate_legacy_ehnd_path_is_a_noop_without_a_legacy_value() {
    let mut config = TranslationConfig {
        eztrans_ehnd_path: "D:/user/Ehnd".to_string(),
        ..TranslationConfig::default()
    };
    config.migrate_legacy_ehnd_path();
    assert_eq!(config.eztrans_ehnd_path, "D:/user/Ehnd");
}

// ---- 구 단일 custom API -> custom_apis 마이그레이션 ----

#[test]
fn non_default_legacy_custom_api_migrates_into_the_named_list() {
    let mut config = TranslationConfig {
        custom: CustomApiConfig {
            name: "레거시".to_string(),
            url: "https://example.com/translate".to_string(),
            ..CustomApiConfig::default()
        },
        ..TranslationConfig::default()
    };
    config.migrate_legacy_custom_api();
    assert_eq!(config.custom_apis.len(), 1);
    assert_eq!(config.custom_apis[0].name, "레거시");
    assert_eq!(config.custom_api, "레거시");
    assert!(
        config.custom.is_default(),
        "이관 후 legacy 단일 필드는 기본값으로 비워져야 합니다"
    );
}

#[test]
fn default_legacy_custom_api_does_not_create_an_entry() {
    let mut config = TranslationConfig::default();
    config.migrate_legacy_custom_api();
    assert!(config.custom_apis.is_empty());
}

#[test]
fn existing_named_list_resets_the_legacy_field_and_keeps_selection() {
    let mut config = config_with_custom_apis(vec![named_api("A"), named_api("B")], "B");
    config.custom = CustomApiConfig {
        name: "레거시-잔재".to_string(),
        ..CustomApiConfig::default()
    };
    config.migrate_legacy_custom_api();
    assert!(config.custom.is_default());
    assert_eq!(
        config.custom_api, "B",
        "이미 선택된 이름은 유지되어야 합니다"
    );
}

#[test]
fn existing_named_list_without_a_selection_defaults_to_the_first_entry() {
    let mut config = config_with_custom_apis(vec![named_api("A"), named_api("B")], "");
    config.migrate_legacy_custom_api();
    assert_eq!(config.custom_api, "A");
}

// ---- active_custom_api 계열 ----

#[test]
fn active_custom_api_falls_back_to_legacy_single_config_when_list_is_empty() {
    let config = TranslationConfig {
        custom: named_api("단일"),
        ..TranslationConfig::default()
    };
    assert_eq!(config.active_custom_api().unwrap().name, "단일");
    assert_eq!(config.active_custom_api_index().unwrap(), 0);
}

#[test]
fn active_custom_api_uses_first_entry_when_selection_is_empty() {
    let config = config_with_custom_apis(vec![named_api("A"), named_api("B")], "");
    assert_eq!(config.active_custom_api().unwrap().name, "A");
    assert_eq!(config.active_custom_api_index().unwrap(), 0);
}

#[test]
fn active_custom_api_finds_the_selected_entry_by_name() {
    let config = config_with_custom_apis(vec![named_api("A"), named_api("B")], "B");
    assert_eq!(config.active_custom_api().unwrap().name, "B");
    assert_eq!(config.active_custom_api_index().unwrap(), 1);
}

#[test]
fn active_custom_api_reports_not_found_for_unknown_selection() {
    let config = config_with_custom_apis(vec![named_api("A")], "없음");
    assert_eq!(
        config.active_custom_api().unwrap_err(),
        CustomApiSelectionError::NotFound("없음".to_string())
    );
    assert_eq!(
        config.active_custom_api_index().unwrap_err(),
        CustomApiSelectionError::NotFound("없음".to_string())
    );
}

#[test]
fn active_custom_api_rejects_duplicate_names_before_lookup() {
    let config = config_with_custom_apis(vec![named_api("A"), named_api("A")], "A");
    assert_eq!(
        config.active_custom_api().unwrap_err(),
        CustomApiSelectionError::DuplicateName("A".to_string())
    );
}

#[test]
fn active_custom_api_rejects_blank_names_before_lookup() {
    let config = config_with_custom_apis(vec![named_api("  ")], "  ");
    assert_eq!(
        config.active_custom_api().unwrap_err(),
        CustomApiSelectionError::EmptyName
    );
}

// ---- select_custom_api ----

#[test]
fn select_custom_api_switches_to_a_known_entry() {
    let mut config = config_with_custom_apis(vec![named_api("A"), named_api("B")], "A");
    assert_eq!(config.select_custom_api("B"), Ok(true));
    assert_eq!(config.custom_api, "B");
}

#[test]
fn select_custom_api_reports_no_change_for_the_current_selection() {
    let mut config = config_with_custom_apis(vec![named_api("A"), named_api("B")], "A");
    assert_eq!(config.select_custom_api("A"), Ok(false));
}

#[test]
fn select_custom_api_rejects_unknown_names() {
    let mut config = config_with_custom_apis(vec![named_api("A")], "A");
    assert_eq!(
        config.select_custom_api("없음"),
        Err(CustomApiSelectionError::NotFound("없음".to_string()))
    );
}

#[test]
fn select_custom_api_with_empty_list_only_accepts_the_legacy_name() {
    let mut config = TranslationConfig {
        custom: named_api("단일"),
        ..TranslationConfig::default()
    };
    assert_eq!(config.select_custom_api("단일"), Ok(false));
    assert_eq!(
        config.select_custom_api("다른"),
        Err(CustomApiSelectionError::NotFound("다른".to_string()))
    );
}

// ---- deepl_strategy / deepl_effective_keys ----

#[test]
fn deepl_strategy_parses_round_robin_case_insensitively_and_aliased() {
    for value in ["round-robin", "ROUND-ROBIN", "roundrobin", "rr", "RR"] {
        let config = TranslationConfig {
            deepl_strategy: value.to_string(),
            ..TranslationConfig::default()
        };
        assert_eq!(
            config.deepl_strategy(),
            DeepLStrategy::RoundRobin,
            "{value}"
        );
    }
}

#[test]
fn deepl_strategy_defaults_to_failover_for_unknown_values() {
    let config = TranslationConfig {
        deepl_strategy: "이상한값".to_string(),
        ..TranslationConfig::default()
    };
    assert_eq!(config.deepl_strategy(), DeepLStrategy::Failover);
}

#[test]
fn deepl_effective_keys_prefers_multi_key_list_over_legacy_single_key() {
    let config = TranslationConfig {
        deepl_api_key: "legacy-key".to_string(),
        deepl_keys: vec!["k1".to_string(), String::new(), "k2".to_string()],
        ..TranslationConfig::default()
    };
    assert_eq!(config.deepl_effective_keys(), vec!["k1", "k2"]);
}

#[test]
fn deepl_effective_keys_falls_back_to_legacy_single_key() {
    let config = TranslationConfig {
        deepl_api_key: "legacy-key".to_string(),
        deepl_keys: Vec::new(),
        ..TranslationConfig::default()
    };
    assert_eq!(config.deepl_effective_keys(), vec!["legacy-key"]);
}

#[test]
fn deepl_effective_keys_is_empty_when_no_key_is_configured() {
    let config = TranslationConfig::default();
    assert!(config.deepl_effective_keys().is_empty());
}

#[test]
fn deepl_effective_keys_ignores_a_legacy_key_when_multi_keys_are_all_blank() {
    let config = TranslationConfig {
        deepl_api_key: "legacy-key".to_string(),
        deepl_keys: vec![String::new()],
        ..TranslationConfig::default()
    };
    assert_eq!(config.deepl_effective_keys(), vec!["legacy-key"]);
}

// ---- 엔진/언어 접근자 ----

#[test]
fn get_engine_round_trips_through_set_engine() {
    let mut config = TranslationConfig::default();
    config.set_engine(crate::translation::TranslationEngine::DeepL);
    assert_eq!(
        config.get_engine().unwrap(),
        crate::translation::TranslationEngine::DeepL
    );
}

#[test]
fn get_engine_reports_a_parse_error_for_a_corrupted_value() {
    let config = TranslationConfig {
        engine: "corrupted".to_string(),
        ..TranslationConfig::default()
    };
    assert!(config.get_engine().is_err());
}

#[test]
fn source_and_target_language_round_trip_through_setters() {
    let mut config = TranslationConfig::default();
    config.set_source_language(crate::translation::Language::Eng);
    config.set_target_language(crate::translation::Language::Fra);
    assert_eq!(config.source_lang, "en");
    assert_eq!(config.target_lang, "fr");
    assert_eq!(
        config.get_source_language().unwrap(),
        crate::translation::Language::Eng
    );
    assert_eq!(
        config.get_target_language().unwrap(),
        crate::translation::Language::Fra
    );
}

#[test]
fn get_language_reports_invalid_language_code() {
    let config = TranslationConfig {
        source_lang: "??".to_string(),
        ..TranslationConfig::default()
    };
    assert_eq!(
        config.get_source_language().unwrap_err(),
        InvalidLanguageCode("??".to_string())
    );
}

#[test]
fn source_lang_index_finds_the_position_within_supported_languages() {
    let config = TranslationConfig {
        source_lang: "ja".to_string(),
        ..TranslationConfig::default()
    };
    let index = config
        .source_lang_index(crate::translation::TranslationEngine::Google)
        .unwrap();
    assert_eq!(
        crate::translation::GOOGLE_SUPPORTED_LANGUAGES[index],
        crate::translation::Language::Jpn
    );
}

#[test]
fn source_lang_index_reports_an_unsupported_pair_by_name() {
    // EzTrans는 일본어 소스만 지원하므로 한국어는 지원 목록에 없다.
    let config = TranslationConfig {
        source_lang: "ko".to_string(),
        ..TranslationConfig::default()
    };
    let error = config
        .source_lang_index(crate::translation::TranslationEngine::EzTrans)
        .unwrap_err();
    assert!(error.contains("eztrans"), "{error}");
    assert!(error.contains("ko"), "{error}");
}

// ---- 번역 서버 URL 해시 ----

/// 기준 해시는 실행 파일에만 둔다 — 설정 파일에 쓰지 않고, 구 설정에 남아
/// 있는 줄은 읽지도 않는다(저장하면 사라진다).
#[test]
fn mys_translater_url_hash_never_appears_in_the_config_file() {
    let old_config = toml::from_str::<TranslationConfig>(
        "mys_translater_url = \"http://127.0.0.1:8080\"\n\
         mys_translater_url_hash = \"deadbeef\"\n",
    )
    .expect("구 키가 남아 있어도 로드는 성공해야 한다");
    assert_eq!(old_config.mys_translater_url, "http://127.0.0.1:8080");

    let saved = toml::to_string(&old_config).expect("직렬화");
    assert!(!saved.contains("mys_translater_url_hash"), "{saved}");
}
