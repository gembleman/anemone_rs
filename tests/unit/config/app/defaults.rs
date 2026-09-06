//! 기본값 적용, 범위 정규화, 스키마 검증에 관한 테스트.
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
fn legacy_config_without_update_setting_keeps_auto_check_enabled() {
    let loaded = Config::from_toml_str("window_visible = false\n").unwrap();
    assert!(loaded.update_check_enabled);
}

/// 방어 옵션은 기본값이 켜짐이므로, 키가 없는 기존 설정도 켜진 상태로 로드된다.
#[test]
fn legacy_config_without_source_language_guard_enables_it() {
    let loaded = Config::from_toml_str("window_visible = false\n").unwrap();
    assert!(loaded.clipboard_source_language_guard);

    let disabled =
        Config::from_toml_str("clipboard_source_language_guard = false\n").expect("valid config");
    assert!(!disabled.clipboard_source_language_guard);
}

#[test]
fn update_check_setting_round_trips_when_disabled() {
    let config = Config {
        update_check_enabled: false,
        ..Default::default()
    };
    let serialized = toml::to_string(&config).expect("serialize update setting");
    let loaded = Config::from_toml_str(&serialized).expect("deserialize update setting");
    assert!(!loaded.update_check_enabled);
}

#[test]
fn overlay_window_placement_is_optional_and_round_trips() {
    let legacy = Config::from_toml_str("window_visible = true\n").unwrap();
    assert_eq!(legacy.window_x, None);
    assert_eq!(legacy.window_y, None);
    assert_eq!(legacy.window_width, None);
    assert_eq!(legacy.window_height, None);
    // 배치를 저장한 적이 없으면 키 자체를 남기지 않는다.
    let legacy_toml = toml::to_string_pretty(&legacy).unwrap();
    assert!(!legacy_toml.contains("window_x"));
    assert!(!legacy_toml.contains("window_width"));

    let config = Config {
        window_x: Some(-1920),
        window_y: Some(37),
        window_width: Some(720),
        window_height: Some(360),
        ..Config::default()
    };
    let serialized = toml::to_string_pretty(&config).unwrap();
    let reloaded = Config::from_toml_str(&serialized).unwrap();
    assert_eq!(reloaded.window_x, Some(-1920));
    assert_eq!(reloaded.window_y, Some(37));
    assert_eq!(reloaded.window_width, Some(720));
    assert_eq!(reloaded.window_height, Some(360));
}

#[test]
fn future_schema_version_is_rejected() {
    let error = Config::from_toml_str("schema_version = 999\n").unwrap_err();
    assert!(matches!(
        error,
        ConfigDecodeError::UnsupportedSchema { found: 999, .. }
    ));
}
