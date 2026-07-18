use super::*;

#[test]
fn bool_commands_toggle_without_ui_identifiers() {
    let mut config = Config::default();
    let before = config.show_original;

    let result = SettingsEditor::apply(
        &mut config,
        SettingsChange::Toggle(BoolSetting::ShowOriginal),
    );

    assert_eq!(config.show_original, !before);
    assert!(result.save_required);
    assert!(result.preview_refresh_required);
}

#[test]
fn numeric_commands_clamp_and_suppress_identical_changes() {
    let mut config = Config::default();
    let changed = SettingsEditor::apply(
        &mut config,
        SettingsChange::Numeric {
            setting: NumericSetting::BorderWidth,
            value: 99,
        },
    );
    assert_eq!(config.border_width, 10);
    assert!(changed.changed);

    let unchanged = SettingsEditor::apply(
        &mut config,
        SettingsChange::Numeric {
            setting: NumericSetting::BorderWidth,
            value: 11,
        },
    );
    assert_eq!(unchanged, SettingsChangeResult::default());
}

#[test]
fn colors_and_fonts_are_applied_as_pure_config_changes() {
    let mut config = Config::default();
    SettingsEditor::apply(
        &mut config,
        SettingsChange::TextColor {
            text_type: TextType::Name,
            color_type: ColorType::Primary,
            argb: 0xff12_3456,
        },
    );
    SettingsEditor::apply(
        &mut config,
        SettingsChange::Font {
            text_type: TextType::Name,
            face_name: "Test Font".into(),
            style_bits: 3,
        },
    );

    assert_eq!(config.name_style.color_primary, 0xff12_3456);
    assert_eq!(config.name_style.font_face, "Test Font");
    assert_eq!(config.name_style.font_style, 3);
}
