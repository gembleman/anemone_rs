//! 실제 픽셀에 영향을 주는 outline/shadow 값만 남긴 정규형.

use super::super::style::TextRenderStyle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::d2d) struct EffectiveOutlineStyle {
    pub(in crate::d2d) outline1_size: i32,
    pub(in crate::d2d) outline1_color: u32,
    pub(in crate::d2d) outline2_size: i32,
    pub(in crate::d2d) outline2_color: u32,
    pub(in crate::d2d) outline_total: i32,
    pub(in crate::d2d) has_shadow: bool,
    pub(in crate::d2d) shadow_color: u32,
    pub(in crate::d2d) shadow_offset_x: i32,
    pub(in crate::d2d) shadow_offset_y: i32,
}

impl EffectiveOutlineStyle {
    pub(in crate::d2d) fn from_style(style: &TextRenderStyle) -> Self {
        let outline1_size = style.outline1_size.max(0);
        let outline2_size = style.outline2_size.max(0);
        let outline_total = outline1_size.saturating_add(outline2_size);
        let has_shadow =
            style.shadow_enabled && (style.shadow_offset_x != 0 || style.shadow_offset_y != 0);
        Self {
            outline1_size,
            outline1_color: if outline1_size > 0 {
                style.outline1_color
            } else {
                0
            },
            outline2_size,
            outline2_color: if outline2_size > 0 {
                style.outline2_color
            } else {
                0
            },
            outline_total,
            has_shadow,
            shadow_color: if has_shadow { style.shadow_color } else { 0 },
            shadow_offset_x: if has_shadow { style.shadow_offset_x } else { 0 },
            shadow_offset_y: if has_shadow { style.shadow_offset_y } else { 0 },
        }
    }
}
