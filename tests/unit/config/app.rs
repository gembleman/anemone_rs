use super::*;

#[test]
fn out_of_range_toml_is_normalized_at_deserialize_boundary() {
    let mut raw = Config {
        border_width: i32::MAX,
        text_margin_x: -1,
        text_margin_y: i32::MAX,
        name_margin: -10,
        shadow_offset_x: -20,
        shadow_offset_y: i32::MAX,
        ..Config::default()
    };

    for style in [
        &mut raw.name_style,
        &mut raw.original_style,
        &mut raw.translation_style,
    ] {
        style.size = i32::MIN;
        style.outline1_size = -1;
        style.outline2_size = i32::MAX;
        style.font_style = u8::MAX;
    }

    let toml = toml::to_string(&raw).expect("serialize test config");
    let normalized = Config::from_toml_str(&toml).expect("deserialize test config");

    assert_eq!(normalized.border_width, 10);
    assert_eq!(normalized.text_margin_x, 0);
    assert_eq!(normalized.text_margin_y, 300);
    assert_eq!(normalized.name_margin, 0);
    assert_eq!(normalized.shadow_offset_x, 0);
    assert_eq!(normalized.shadow_offset_y, 20);
    for style in [
        &normalized.name_style,
        &normalized.original_style,
        &normalized.translation_style,
    ] {
        assert_eq!(style.size, 6);
        assert_eq!(style.outline1_size, 0);
        assert_eq!(style.outline2_size, 20);
        assert_eq!(style.font_style, 3);
    }
}

#[test]
fn invalid_translation_values_are_rejected_instead_of_defaulted() {
    for mutate in [
        |config: &mut Config| config.translation.engine = "typo-engine".into(),
        |config: &mut Config| config.translation.source_lang = "not-a-language".into(),
        |config: &mut Config| config.translation.llm.provider = "typo-provider".into(),
    ] {
        let mut raw = Config::default();
        mutate(&mut raw);
        let text = toml::to_string(&raw).expect("serialize invalid test config");
        assert!(Config::from_toml_str(&text).is_err(), "config: {text}");
    }
}

#[test]
fn partial_config_uses_defaults_for_missing_top_level_fields() {
    let loaded = Config::from_toml_str("window_visible = false\n").unwrap();
    assert_eq!(loaded.schema_version, CURRENT_SCHEMA_VERSION);
    assert!(!loaded.window_visible);
    assert_eq!(
        loaded.clipboard_max_length,
        Config::default().clipboard_max_length
    );
    assert_eq!(
        loaded.translation.engine,
        Config::default().translation.engine
    );
}

#[test]
fn future_schema_version_is_rejected() {
    let error = Config::from_toml_str("schema_version = 999\n").unwrap_err();
    assert!(matches!(
        error,
        ConfigDecodeError::UnsupportedSchema { found: 999, .. }
    ));
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
fn llm_limits_are_normalized_at_every_config_boundary() {
    let mut raw = Config::default();
    raw.translation.llm.max_tokens = u32::MAX;
    raw.translation.llm.debounce_ms = u32::MAX;
    raw.translation.llm.temperature = f32::INFINITY;
    raw.translation.llm.top_p = 3.0;
    raw.translation.llm.frequency_penalty = -3.0;
    raw.translation.llm.presence_penalty = f32::INFINITY;

    let toml = toml::to_string(&raw).unwrap();
    let normalized = Config::from_toml_str(&toml).unwrap();

    assert_eq!(normalized.translation.llm.max_tokens, 32_000);
    assert_eq!(normalized.translation.llm.debounce_ms, 10_000);
    assert_eq!(normalized.translation.llm.temperature, 0.3);
    assert_eq!(normalized.translation.llm.top_p, 1.0);
    assert_eq!(normalized.translation.llm.frequency_penalty, -2.0);
    assert_eq!(normalized.translation.llm.presence_penalty, 0.0);
}

#[test]
fn openrouter_sampling_options_are_loaded_and_saved_without_ui() {
    let text = r#"
[translation.llm]
provider = "openrouter"
model = "anthropic/claude-sonnet-5"
top_p = 0.8
frequency_penalty = 0.4
presence_penalty = -0.2
"#;

    let loaded = Config::from_toml_str(text).unwrap();
    assert_eq!(loaded.translation.llm.top_p, 0.8);
    assert_eq!(loaded.translation.llm.frequency_penalty, 0.4);
    assert_eq!(loaded.translation.llm.presence_penalty, -0.2);
    let params = loaded.translation.llm.to_call_params().unwrap();
    assert_eq!(params.top_p, 0.8);
    assert_eq!(params.frequency_penalty, 0.4);
    assert_eq!(params.presence_penalty, -0.2);

    let serialized = toml::to_string_pretty(&loaded).unwrap();
    let reloaded = Config::from_toml_str(&serialized).unwrap();
    assert_eq!(reloaded.translation.llm.top_p, 0.8);
    assert_eq!(reloaded.translation.llm.frequency_penalty, 0.4);
    assert_eq!(reloaded.translation.llm.presence_penalty, -0.2);
}

#[test]
fn llm_base_url_is_loaded_from_and_saved_to_toml() {
    let text = r#"
[translation.llm]
base_url = "http://127.0.0.1:1234/v1"
"#;

    let loaded = Config::from_toml_str(text).unwrap();
    assert_eq!(loaded.translation.llm.base_url, "http://127.0.0.1:1234/v1");

    let serialized = toml::to_string_pretty(&loaded).unwrap();
    let reloaded = Config::from_toml_str(&serialized).unwrap();
    assert_eq!(
        reloaded.translation.llm.base_url,
        "http://127.0.0.1:1234/v1"
    );
}

#[test]
fn arbitrary_llm_model_is_loaded_from_and_saved_to_toml() {
    let text = r#"
[translation.llm]
provider = "openai"
model = "future-or-private-model-id"
"#;

    let loaded = Config::from_toml_str(text).unwrap();
    assert_eq!(loaded.translation.llm.model, "future-or-private-model-id");

    let serialized = toml::to_string_pretty(&loaded).unwrap();
    let reloaded = Config::from_toml_str(&serialized).unwrap();
    assert_eq!(reloaded.translation.llm.model, "future-or-private-model-id");
}

#[test]
fn llm_reasoning_effort_is_optional_and_round_trips() {
    use crate::translation::llm::ReasoningEffort;

    let default_config = Config::from_toml_str("[translation.llm]\n").unwrap();
    assert_eq!(default_config.translation.llm.reasoning_effort, None);

    let configured = Config::from_toml_str(
        "[translation.llm]\nprovider = \"openai\"\nreasoning_effort = \"xhigh\"\n",
    )
    .unwrap();
    assert_eq!(
        configured.translation.llm.reasoning_effort,
        Some(ReasoningEffort::Xhigh)
    );

    let serialized = toml::to_string_pretty(&configured).unwrap();
    let reloaded = Config::from_toml_str(&serialized).unwrap();
    assert_eq!(
        reloaded.translation.llm.reasoning_effort,
        Some(ReasoningEffort::Xhigh)
    );
}

#[test]
fn eztrans_process_count_defaults_and_is_clamped() {
    let partial = Config::from_toml_str("[translation]\nengine = \"eztrans\"\n").unwrap();
    assert_eq!(partial.translation.eztrans_process_count, 2);

    let mut raw = Config::default();
    raw.translation.eztrans_process_count = u32::MAX;
    let normalized = Config::from_toml_str(&toml::to_string(&raw).unwrap()).unwrap();
    assert_eq!(normalized.translation.eztrans_process_count, 16);

    raw.translation.eztrans_process_count = 0;
    let normalized = Config::from_toml_str(&toml::to_string(&raw).unwrap()).unwrap();
    assert_eq!(normalized.translation.eztrans_process_count, 1);
}

#[test]
fn stale_bundled_eztrans_paths_follow_the_config_location() {
    let root = unique_test_dir("relocate-eztrans");
    let path = root.join("config.toml");
    let bundled = root.join("eztrans_dll");
    std::fs::create_dir_all(bundled.join("Dat")).unwrap();
    std::fs::write(bundled.join("J2KEngine.dll"), b"test").unwrap();

    let mut config = Config::default();
    let stale = root.join("old-worktree").join("eztrans_dll");
    config.translation.eztrans_dll_path =
        stale.join("J2KEngine.dll").to_string_lossy().into_owned();
    config.translation.eztrans_dat_path = stale.join("Dat").to_string_lossy().into_owned();
    config.save_to_file(&path).unwrap();

    let loaded = Config::load_or_default_from(&path);

    assert_eq!(
        loaded.translation.eztrans_dll_path,
        bundled.join("J2KEngine.dll").to_string_lossy()
    );
    assert_eq!(
        loaded.translation.eztrans_dat_path,
        bundled.join("Dat").to_string_lossy()
    );
    let persisted = Config::load_from_file(&path).unwrap();
    assert_eq!(
        persisted.translation.eztrans_dll_path,
        loaded.translation.eztrans_dll_path
    );
    assert_eq!(
        persisted.translation.eztrans_dat_path,
        loaded.translation.eztrans_dat_path
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_custom_eztrans_paths_are_not_rewritten() {
    let root = unique_test_dir("keep-custom-eztrans");
    let path = root.join("config.toml");
    let bundled = root.join("eztrans_dll");
    std::fs::create_dir_all(bundled.join("Dat")).unwrap();
    std::fs::write(bundled.join("J2KEngine.dll"), b"test").unwrap();

    let mut config = Config::default();
    config.translation.eztrans_dll_path = root
        .join("custom")
        .join("engine.dll")
        .to_string_lossy()
        .into_owned();
    config.translation.eztrans_dat_path = root
        .join("custom")
        .join("data")
        .to_string_lossy()
        .into_owned();
    config.save_to_file(&path).unwrap();

    let loaded = Config::load_or_default_from(&path);

    assert_eq!(
        loaded.translation.eztrans_dll_path,
        config.translation.eztrans_dll_path
    );
    assert_eq!(
        loaded.translation.eztrans_dat_path,
        config.translation.eztrans_dat_path
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn relative_bundled_eztrans_paths_are_preserved_when_loading() {
    let root = unique_test_dir("keep-relative-eztrans");
    let path = root.join("config.toml");
    let bundled = root.join("eztrans_dll");
    std::fs::create_dir_all(bundled.join("Dat")).unwrap();
    std::fs::write(bundled.join("J2KEngine.dll"), b"test").unwrap();

    let mut config = Config::default();
    config.translation.eztrans_dll_path = r"eztrans_dll\J2KEngine.dll".into();
    config.translation.eztrans_dat_path = r"eztrans_dll\Dat".into();
    config.save_to_file(&path).unwrap();

    let loaded = Config::load_or_default_from(&path);

    assert_eq!(
        loaded.translation.eztrans_dll_path,
        r"eztrans_dll\J2KEngine.dll"
    );
    assert_eq!(loaded.translation.eztrans_dat_path, r"eztrans_dll\Dat");
    let persisted = Config::load_from_file(&path).unwrap();
    assert_eq!(
        persisted.translation.eztrans_dll_path,
        r"eztrans_dll\J2KEngine.dll"
    );
    assert_eq!(persisted.translation.eztrans_dat_path, r"eztrans_dll\Dat");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn save_is_atomic_and_preserves_last_known_good_generation() {
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
        Config::load_from_file(&path.with_extension("toml.last-good"))
            .unwrap()
            .translation
            .llm
            .model,
        "first"
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
