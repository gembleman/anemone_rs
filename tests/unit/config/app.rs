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
        repeat_text_mode: u8::MAX,
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
    assert_eq!(normalized.repeat_text_mode, 4);
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
fn llm_limits_are_normalized_at_every_config_boundary() {
    let mut raw = Config::default();
    raw.translation.llm.max_tokens = u32::MAX;
    raw.translation.llm.debounce_ms = u32::MAX;
    raw.translation.llm.temperature = f32::INFINITY;

    let toml = toml::to_string(&raw).unwrap();
    let normalized = Config::from_toml_str(&toml).unwrap();

    assert_eq!(normalized.translation.llm.max_tokens, 32_000);
    assert_eq!(normalized.translation.llm.debounce_ms, 10_000);
    assert_eq!(normalized.translation.llm.temperature, 0.3);
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
