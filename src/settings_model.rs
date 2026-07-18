//! Settings Dialog가 사용하는 Win32 비의존 설정 변경 명령.

use crate::config::{ColorType, Config, TextAlign, TextType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolSetting {
    BackgroundVisible,
    TextShadow(TextType),
    BorderVisible,
    ShowOriginal,
    ShowTranslation,
    ShowName,
    SeparateName,
    WindowTopmost,
    MagneticMinimize,
    ClipboardWatch,
    ClickThrough,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericSetting {
    BackgroundAlpha,
    TextSize(ColorType),
    ShadowOffsetX,
    ShadowOffsetY,
    TextMarginX,
    TextMarginY,
    NameMargin,
    BorderWidth,
}

pub enum SettingsChange {
    Toggle(BoolSetting),
    TextAlignment(TextAlign),
    Numeric {
        setting: NumericSetting,
        value: i32,
    },
    BackgroundColor(u32),
    BorderColor(u32),
    TextColor {
        text_type: TextType,
        color_type: ColorType,
        argb: u32,
    },
    Font {
        text_type: TextType,
        face_name: String,
        style_bits: u8,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SettingsChangeResult {
    pub changed: bool,
    pub save_required: bool,
    pub preview_refresh_required: bool,
}

pub struct SettingsEditor;

impl SettingsEditor {
    pub fn apply(config: &mut Config, change: SettingsChange) -> SettingsChangeResult {
        let changed = match change {
            SettingsChange::Toggle(setting) => toggle(config, setting),
            SettingsChange::TextAlignment(value) => set_if_changed(&mut config.text_align, value),
            SettingsChange::Numeric { setting, value } => set_numeric(config, setting, value),
            SettingsChange::BackgroundColor(argb) => {
                set_if_changed(&mut config.background_color, argb)
            }
            SettingsChange::BorderColor(argb) => set_if_changed(&mut config.border_color, argb),
            SettingsChange::TextColor {
                text_type,
                color_type,
                argb,
            } => {
                let target = config.get_text_style_mut(text_type);
                let current = target.get_color(color_type);
                if current == argb {
                    false
                } else {
                    target.set_color(color_type, argb);
                    true
                }
            }
            SettingsChange::Font {
                text_type,
                face_name,
                style_bits,
            } => {
                let style = config.get_text_style_mut(text_type);
                let mut changed = set_if_changed(&mut style.font_face, face_name);
                changed |= set_if_changed(&mut style.font_style, style_bits);
                changed
            }
        };
        SettingsChangeResult {
            changed,
            save_required: changed,
            preview_refresh_required: changed,
        }
    }
}

fn toggle(config: &mut Config, setting: BoolSetting) -> bool {
    let target = match setting {
        BoolSetting::BackgroundVisible => &mut config.background_visible,
        BoolSetting::TextShadow(text_type) => {
            &mut config.get_text_style_mut(text_type).shadow_enabled
        }
        BoolSetting::BorderVisible => &mut config.border_visible,
        BoolSetting::ShowOriginal => &mut config.show_original,
        BoolSetting::ShowTranslation => &mut config.show_translation,
        BoolSetting::ShowName => &mut config.show_name,
        BoolSetting::SeparateName => &mut config.separate_name,
        BoolSetting::WindowTopmost => &mut config.window_topmost,
        BoolSetting::MagneticMinimize => &mut config.magnetic_minimize,
        BoolSetting::ClipboardWatch => &mut config.clipboard_watch,
        BoolSetting::ClickThrough => &mut config.click_through,
    };
    *target = !*target;
    true
}

fn set_numeric(config: &mut Config, setting: NumericSetting, value: i32) -> bool {
    match setting {
        NumericSetting::BackgroundAlpha => {
            let alpha = value.clamp(0, 255) as u32;
            let argb = (alpha << 24) | (config.background_color & 0x00ff_ffff);
            set_if_changed(&mut config.background_color, argb)
        }
        NumericSetting::TextSize(color_type) => {
            let value = match color_type {
                ColorType::Primary => value.clamp(6, 100),
                ColorType::Outline1 | ColorType::Outline2 => value.clamp(0, 20),
                ColorType::Shadow => return false,
            };
            let changed = [TextType::Name, TextType::Original, TextType::Translation]
                .into_iter()
                .any(|text_type| config.get_text_style(text_type).get_size(color_type) != value);
            if changed {
                config.set_all_text_size(color_type, value);
            }
            changed
        }
        NumericSetting::ShadowOffsetX => {
            set_if_changed(&mut config.shadow_offset_x, value.clamp(0, 20))
        }
        NumericSetting::ShadowOffsetY => {
            set_if_changed(&mut config.shadow_offset_y, value.clamp(0, 20))
        }
        NumericSetting::TextMarginX => {
            set_if_changed(&mut config.text_margin_x, value.clamp(0, 300))
        }
        NumericSetting::TextMarginY => {
            set_if_changed(&mut config.text_margin_y, value.clamp(0, 300))
        }
        NumericSetting::NameMargin => set_if_changed(&mut config.name_margin, value.clamp(0, 300)),
        NumericSetting::BorderWidth => set_if_changed(&mut config.border_width, value.clamp(0, 10)),
    }
}

trait TextStyleSize {
    fn get_size(&self, color_type: ColorType) -> i32;
}

impl TextStyleSize for crate::config::TextStyle {
    fn get_size(&self, color_type: ColorType) -> i32 {
        match color_type {
            ColorType::Primary => self.size,
            ColorType::Outline1 => self.outline1_size,
            ColorType::Outline2 => self.outline2_size,
            ColorType::Shadow => 0,
        }
    }
}

fn set_if_changed<T: PartialEq>(target: &mut T, value: T) -> bool {
    if *target == value {
        return false;
    }
    *target = value;
    true
}

#[cfg(test)]
#[path = "../tests/unit/settings_model.rs"]
mod tests;
