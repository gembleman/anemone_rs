//! 설정 파일 저장/로드 시 원자성, 손상 파일 처리에 관한 테스트.
use super::*;

#[cfg(mys_private)]
#[path = "persistence_mys.rs"]
mod mys;

#[test]
fn save_replaces_the_previous_generation_without_leaving_extra_files() {
    let root = unique_test_dir("save");
    let path = root.join("config.toml");
    let mut first = Config::default();
    first.translation.llm.model = "first".to_string();
    first.save_to_file(&path).unwrap();
    let mut second = first.clone();
    second.translation.llm.model = "second".to_string();
    second.save_to_file(&path).unwrap();

    assert_eq!(
        Config::load_from_file(&path).unwrap().translation.llm.model,
        "second"
    );
    assert_eq!(
        std::fs::read_dir(&root).unwrap().count(),
        1,
        "설정 파일 말고 다른 파일이 남으면 안 된다"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn credentials_are_saved_in_the_plaintext_config_without_a_secret_store() {
    let root = unique_test_dir("plaintext-credentials");
    let path = root.join("config.toml");
    let mut config = Config::default();
    config.translation.llm.api_key = "test-credential-value".to_string();

    config.save_to_file(&path).unwrap();

    let persisted = std::fs::read_to_string(&path).unwrap();
    assert!(persisted.contains("test-credential-value"));
    assert_eq!(
        Config::load_from_file(&path)
            .unwrap()
            .translation
            .llm
            .api_key,
        "test-credential-value"
    );
    assert!(!root.join("secrets.dat").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_config_is_quarantined_without_overwrite() {
    let root = unique_test_dir("corrupt");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("config.toml");
    std::fs::write(&path, "not = [valid").unwrap();

    let loaded = Config::load_or_default_from(&path);

    assert!(!path.exists());
    assert_eq!(
        loaded.translation.engine,
        Config::default().translation.engine
    );
    let quarantines: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(quarantines.len(), 1);
    assert_eq!(
        std::fs::read_to_string(&quarantines[0]).unwrap(),
        "not = [valid"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn io_error_does_not_quarantine_or_rename_the_source() {
    let root = unique_test_dir("io-error");
    let path = root.join("config.toml");
    std::fs::create_dir_all(&path).unwrap();

    assert!(matches!(
        Config::load_from_file(&path),
        Err(ConfigLoadError::Io(_))
    ));
    let _ = Config::load_or_default_from(&path);

    assert!(path.is_dir());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
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

/// 로그 subscriber는 설정을 온전히 읽기 전에 세우므로 이 값만 따로 꺼낸다.
/// 파일이 없거나 깨졌으면 꺼짐이고, 파일을 새로 만들지 않는다.
#[test]
fn the_hook_debug_log_flag_is_read_without_loading_the_whole_config() {
    let root = unique_test_dir("peek-hook-debug");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("config.toml");

    assert!(!Config::peek_hook_debug_log_from(&path));
    assert!(!path.exists(), "없는 설정 파일을 만들지 않는다");

    std::fs::write(&path, "[hook]\ndebug_log = true\n").unwrap();
    assert!(Config::peek_hook_debug_log_from(&path));

    std::fs::write(&path, "[hook]\ndebug_log = false\n").unwrap();
    assert!(!Config::peek_hook_debug_log_from(&path));

    // 다른 키가 깨져 있어도, 표를 읽을 수만 있으면 이 값은 살린다.
    std::fs::write(&path, "[hook]\ndebug_log = true\nmerge_window_ms = \"?\"\n").unwrap();
    assert!(Config::peek_hook_debug_log_from(&path));

    std::fs::write(&path, "이건 toml이 아니다 [[[").unwrap();
    assert!(!Config::peek_hook_debug_log_from(&path));

    std::fs::remove_dir_all(root).unwrap();
}
