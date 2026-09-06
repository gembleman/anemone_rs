use super::*;
use crate::config::Config;
use std::str::FromStr;

#[test]
fn parse_and_format_roundtrip() {
    let cases = [
        "Ctrl+Shift+A",
        "Ctrl+Shift+Up",
        "Ctrl+Shift+Down",
        "Ctrl+Shift+C",
        "Alt+F4",
        "Win+Space",
        "Ctrl+Alt+Shift+Win+Tab",
    ];
    for case in cases {
        let spec = HotkeySpec::from_str(case).unwrap_or_else(|e| panic!("{case} 파싱 실패: {e}"));
        let formatted = spec.to_string();
        let reparsed = HotkeySpec::from_str(&formatted)
            .unwrap_or_else(|e| panic!("{formatted} 재파싱 실패: {e}"));
        assert_eq!(spec, reparsed, "왕복 변환 불일치: {case} -> {formatted}");
    }
}

#[test]
fn parse_is_case_insensitive_for_modifiers_and_key() {
    let a = HotkeySpec::from_str("ctrl+shift+a").unwrap();
    let b = HotkeySpec::from_str("CTRL+SHIFT+A").unwrap();
    assert_eq!(a, b);
}

#[test]
fn modifierless_hotkey_is_valid_and_roundtrips() {
    let spec = HotkeySpec::from_str("F8").unwrap();
    assert!(spec.is_valid());
    assert_eq!(spec.modifiers(), 0);
    assert_eq!(spec.to_string(), "F8");
}

#[test]
fn parse_rejects_empty_string() {
    assert_eq!(
        HotkeySpec::from_str("").unwrap_err(),
        HotkeyParseError::Empty
    );
    assert_eq!(
        HotkeySpec::from_str("   ").unwrap_err(),
        HotkeyParseError::Empty
    );
}

#[test]
fn parse_rejects_unknown_key() {
    assert_eq!(
        HotkeySpec::from_str("Ctrl+Shift+???").unwrap_err(),
        HotkeyParseError::UnknownKey("???".to_string())
    );
}

#[test]
fn parse_rejects_missing_key() {
    assert_eq!(
        HotkeySpec::from_str("Ctrl+Shift").unwrap_err(),
        HotkeyParseError::MissingKey
    );
}

#[test]
fn default_hotkeys_match_legacy_hardcoded_values() {
    let config = HotkeyConfig::default();
    assert_eq!(config.toggle_window.to_string(), "Ctrl+Shift+A");
    assert_eq!(config.text_size_up.to_string(), "Ctrl+Shift+Up");
    assert_eq!(config.text_size_down.to_string(), "Ctrl+Shift+Down");
    assert_eq!(config.clipboard_watch.to_string(), "Ctrl+Shift+C");
}

#[test]
fn default_hotkeys_have_no_conflicts() {
    assert_eq!(HotkeyConfig::default().find_conflict(), None);
}

#[test]
fn find_conflict_detects_duplicate_assignment() {
    let mut config = HotkeyConfig::default();
    config.text_size_up = config.toggle_window;
    let conflict = config.find_conflict();
    assert_eq!(
        conflict,
        Some((HotkeySlot::ToggleWindow, HotkeySlot::TextSizeUp))
    );
}

#[test]
fn toml_roundtrip_preserves_hotkeys() {
    let mut config = Config::default();
    config.hotkeys.toggle_window = HotkeySpec::from_str("Alt+F4").unwrap();

    let toml = toml::to_string(&config).expect("serialize config");
    assert!(
        toml.contains("Alt+F4"),
        "TOML에 사람이 읽을 수 있는 문자열이 없습니다:\n{toml}"
    );

    let loaded: Config = toml::from_str(&toml).expect("deserialize config");
    assert_eq!(loaded.hotkeys.toggle_window, config.hotkeys.toggle_window);
}

#[test]
fn missing_hotkeys_field_falls_back_to_default() {
    // 단축키 필드가 없는 기존 config.toml도 정상 로드되어야 한다(하위호환).
    let legacy_toml = toml::to_string(&Config::default()).unwrap();
    let without_hotkeys = remove_toml_section(&legacy_toml, "[hotkeys]");
    let loaded: Config = toml::from_str(&without_hotkeys).expect("legacy config without hotkeys");
    assert_eq!(loaded.hotkeys, HotkeyConfig::default());
}

/// 테스트 전용: 주어진 섹션 헤더부터 다음 섹션 헤더(또는 끝)까지 잘라낸다.
fn remove_toml_section(toml: &str, header: &str) -> String {
    let mut result = String::new();
    let mut in_section = false;
    for line in toml.lines() {
        if line.trim_start() == header {
            in_section = true;
            continue;
        }
        if in_section && line.starts_with('[') {
            in_section = false;
        }
        if !in_section {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

#[test]
fn every_supported_key_parses_and_displays() {
    for &(name, vk) in KEY_TABLE {
        let spec = HotkeySpec::from_str(&format!("Ctrl+{name}"))
            .unwrap_or_else(|error| panic!("{name} 파싱 실패: {error}"));
        assert_eq!(spec.vk, vk);
        assert_eq!(spec.to_string(), format!("Ctrl+{name}"));
    }
    assert!(!KEY_TABLE.is_empty(), "키 테이블이 비어 있습니다");
}
