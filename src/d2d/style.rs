use std::sync::Arc;

use crate::config::TextAlign;

/// Font 이름을 공유해 cache 조회에서 재할당하지 않는 text render style.
#[derive(Clone, Debug)]
pub(crate) struct TextRenderStyle {
    pub font_size: i32,
    pub font_face: Arc<str>,
    pub font_style: u8, // 0: normal, 1: bold, 2: italic, 3: bold+italic
    pub text_align: TextAlign,
    pub color: u32, // ARGB
    pub outline1_size: i32,
    pub outline1_color: u32,
    pub outline2_size: i32,
    pub outline2_color: u32,
    pub shadow_enabled: bool,
    pub shadow_color: u32,
    pub shadow_offset_x: i32,
    pub shadow_offset_y: i32,
}
