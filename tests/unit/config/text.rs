use super::*;

#[test]
fn default_text_style_uses_readable_white_on_dark_outline() {
    let style = TextStyle::default();
    assert_eq!(style.font_face, "맑은 고딕");
    assert_eq!(style.size, 22);
    assert!(style.shadow_enabled);
    assert_eq!(style.color_primary, 0xFFFF_FFFF);
}

#[test]
fn get_color_reads_the_field_matching_the_color_type() {
    let style = TextStyle {
        color_primary: 1,
        color_outline1: 2,
        color_outline2: 3,
        color_shadow: 4,
        ..TextStyle::default()
    };
    assert_eq!(style.get_color(ColorType::Primary), 1);
    assert_eq!(style.get_color(ColorType::Outline1), 2);
    assert_eq!(style.get_color(ColorType::Outline2), 3);
    assert_eq!(style.get_color(ColorType::Shadow), 4);
}

#[test]
fn set_color_writes_only_the_targeted_field() {
    let mut style = TextStyle::default();
    style.set_color(ColorType::Outline2, 0x1234_5678);
    assert_eq!(style.color_outline2, 0x1234_5678);
    assert_eq!(style.color_primary, TextStyle::default().color_primary);
}

#[test]
fn set_size_writes_the_field_matching_the_color_type() {
    let mut style = TextStyle::default();
    style.set_size(ColorType::Primary, 30);
    style.set_size(ColorType::Outline1, 5);
    style.set_size(ColorType::Outline2, 7);
    assert_eq!(style.size, 30);
    assert_eq!(style.outline1_size, 5);
    assert_eq!(style.outline2_size, 7);
}

#[test]
fn set_size_ignores_shadow_since_it_has_no_dedicated_size_field() {
    let mut style = TextStyle::default();
    let before = style.clone();
    style.set_size(ColorType::Shadow, 99);
    assert_eq!(style.size, before.size);
    assert_eq!(style.outline1_size, before.outline1_size);
    assert_eq!(style.outline2_size, before.outline2_size);
}

#[test]
fn text_style_serializes_and_round_trips_through_json() {
    let mut style = TextStyle::default();
    style.set_color(ColorType::Shadow, 0x8000_0000);
    let json = serde_json::to_string(&style).unwrap();
    let reloaded: TextStyle = serde_json::from_str(&json).unwrap();
    assert_eq!(reloaded.color_shadow, 0x8000_0000);
    assert_eq!(reloaded.font_face, style.font_face);
}
