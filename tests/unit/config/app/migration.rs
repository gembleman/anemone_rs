//! 구 키·구 스키마 설정을 새 형식으로 옮기는 마이그레이션 테스트.
use super::*;

#[test]
fn legacy_eztrans_dictionary_key_deserializes_to_the_new_field() {
    let loaded = Config::from_toml_str(
        r#"
[translation]
engine = "eztrans"
eztrans_dll_path = "legacy.dll"
eztrans_dat_path = "Dat"
"#,
    )
    .expect("legacy EzTrans path key should remain readable");

    assert_eq!(loaded.translation.eztrans_dictionary_path, "legacy.dll");
    assert_eq!(loaded.translation.eztrans_ehnd_path, "Ehnd");
    let serialized = toml::to_string(&loaded).expect("serialize migrated config");
    assert!(serialized.contains("eztrans_dictionary_path"));
    assert!(serialized.contains("eztrans_ehnd_path"));
    assert!(!serialized.contains("eztrans_dll_path"));
    assert!(!serialized.contains("eztrans_dat_path"));
}

#[test]
fn explicit_eztrans_ehnd_key_is_never_rewritten_even_if_named_dat() {
    // 새 키로 쓴 값은 폴더 이름과 무관하게 사용자 값이다. 구 키 치환 로직이
    // 역직렬화기에 살아 있던 시절에는 매 로드/저장마다 몰래 재작성됐다.
    let loaded = Config::from_toml_str(
        r#"
[translation]
engine = "eztrans"
eztrans_ehnd_path = 'C:\custom\tools\Dat'
"#,
    )
    .expect("explicit ehnd key should load as-is");

    assert_eq!(loaded.translation.eztrans_ehnd_path, r"C:\custom\tools\Dat");
}

#[test]
fn explicit_eztrans_ehnd_key_wins_over_the_legacy_dat_key() {
    let loaded = Config::from_toml_str(
        r#"
[translation]
engine = "eztrans"
eztrans_ehnd_path = 'C:\custom\Ehnd'
eztrans_dat_path = 'C:\old\Dat'
"#,
    )
    .expect("both keys should be readable");

    assert_eq!(loaded.translation.eztrans_ehnd_path, r"C:\custom\Ehnd");
}

#[test]
fn named_custom_api_list_round_trips_and_selects_by_name() {
    let text = r#"
[translation]
engine = "custom"
custom_api = "backup"

[[translation.custom_apis]]
name = "primary"
url = "https://primary.example/translate"

[[translation.custom_apis]]
name = "backup"
url = "https://backup.example/translate"
request_template = '{"q":"{text}"}'
response_path = "result.text"
"#;

    let loaded = Config::from_toml_str(text).unwrap();
    assert_eq!(loaded.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(loaded.translation.custom_apis.len(), 2);
    assert_eq!(
        loaded.translation.active_custom_api().unwrap().url,
        "https://backup.example/translate"
    );

    let serialized = toml::to_string_pretty(&loaded).unwrap();
    assert!(serialized.contains("[[translation.custom_apis]]"));
    assert!(!serialized.contains("[translation.custom]\n"));
}

#[test]
fn legacy_single_custom_api_is_migrated_to_the_named_list() {
    let text = r#"
[translation]
engine = "custom"

[translation.custom]
url = "https://legacy.example/translate"
"#;

    let loaded = Config::from_toml_str(text).unwrap();
    assert_eq!(loaded.translation.custom_api, "Custom");
    assert_eq!(loaded.translation.custom_apis.len(), 1);
    assert_eq!(
        loaded.translation.active_custom_api().unwrap().url,
        "https://legacy.example/translate"
    );
}

#[test]
fn stale_bundled_eztrans_paths_follow_the_config_location() {
    let root = unique_test_dir("relocate-eztrans");
    let path = root.join("config.toml");
    let bundled = root.join("eztrans_dll");
    std::fs::create_dir_all(bundled.join("Ehnd")).unwrap();
    std::fs::write(bundled.join("JisJK.flat.bin"), b"test").unwrap();

    let mut config = Config::default();
    let stale = root.join("old-worktree").join("eztrans_dll");
    config.translation.eztrans_dictionary_path =
        stale.join("JisJK.flat.bin").to_string_lossy().into_owned();
    config.translation.eztrans_ehnd_path = stale.join("Ehnd").to_string_lossy().into_owned();
    config.save_to_file(&path).unwrap();

    let loaded = Config::load_or_default_from(&path);

    assert_eq!(
        loaded.translation.eztrans_dictionary_path,
        bundled.join("JisJK.flat.bin").to_string_lossy()
    );
    assert_eq!(
        loaded.translation.eztrans_ehnd_path,
        bundled.join("Ehnd").to_string_lossy()
    );
    let persisted = Config::load_from_file(&path).unwrap();
    assert_eq!(
        persisted.translation.eztrans_dictionary_path,
        loaded.translation.eztrans_dictionary_path
    );
    assert_eq!(
        persisted.translation.eztrans_ehnd_path,
        loaded.translation.eztrans_ehnd_path
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_custom_eztrans_paths_are_not_rewritten() {
    let root = unique_test_dir("keep-custom-eztrans");
    let path = root.join("config.toml");
    let bundled = root.join("eztrans_dll");
    std::fs::create_dir_all(bundled.join("Ehnd")).unwrap();
    std::fs::write(bundled.join("JisJK.flat.bin"), b"test").unwrap();

    let mut config = Config::default();
    config.translation.eztrans_dictionary_path = root
        .join("custom")
        .join("engine.dll")
        .to_string_lossy()
        .into_owned();
    config.translation.eztrans_ehnd_path = root
        .join("custom")
        .join("data")
        .to_string_lossy()
        .into_owned();
    config.save_to_file(&path).unwrap();

    let loaded = Config::load_or_default_from(&path);

    assert_eq!(
        loaded.translation.eztrans_dictionary_path,
        config.translation.eztrans_dictionary_path
    );
    assert_eq!(
        loaded.translation.eztrans_ehnd_path,
        config.translation.eztrans_ehnd_path
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn relative_bundled_eztrans_paths_are_preserved_when_loading() {
    let root = unique_test_dir("keep-relative-eztrans");
    let path = root.join("config.toml");
    let bundled = root.join("eztrans_dll");
    std::fs::create_dir_all(bundled.join("Ehnd")).unwrap();
    std::fs::write(bundled.join("JisJK.flat.bin"), b"test").unwrap();

    let mut config = Config::default();
    config.translation.eztrans_dictionary_path = r"eztrans_dll\JisJK.flat.bin".into();
    config.translation.eztrans_ehnd_path = r"eztrans_dll\Ehnd".into();
    config.save_to_file(&path).unwrap();

    let loaded = Config::load_or_default_from(&path);

    assert_eq!(
        loaded.translation.eztrans_dictionary_path,
        r"eztrans_dll\JisJK.flat.bin"
    );
    assert_eq!(loaded.translation.eztrans_ehnd_path, r"eztrans_dll\Ehnd");
    let persisted = Config::load_from_file(&path).unwrap();
    assert_eq!(
        persisted.translation.eztrans_dictionary_path,
        r"eztrans_dll\JisJK.flat.bin"
    );
    assert_eq!(persisted.translation.eztrans_ehnd_path, r"eztrans_dll\Ehnd");
    std::fs::remove_dir_all(root).unwrap();
}

fn unique_test_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "anemone-config-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
