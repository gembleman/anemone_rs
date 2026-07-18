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
