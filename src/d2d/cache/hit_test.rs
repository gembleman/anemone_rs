//! 줄별 hit-test 결과의 캐시 키와 항목.

use super::super::style::TextRenderStyle;
use super::layout::{LayoutKeyRef, TextLayoutKey};
use windows::Win32::Foundation::RECT;

/// 줄별 hit-test 결과의 layout 및 위치 입력.
pub(in crate::d2d) struct HitTestKey {
    pub(in crate::d2d) layout: TextLayoutKey,
    origin_x_bits: u32,
    origin_y_bits: u32,
    inflate_bits: u32,
}

pub(in crate::d2d) struct HitTestKeyRef<'a> {
    layout: LayoutKeyRef<'a>,
    origin_x_bits: u32,
    origin_y_bits: u32,
    inflate_bits: u32,
}

impl<'a> HitTestKeyRef<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::d2d) fn from_style(
        text: &'a str,
        style: &'a TextRenderStyle,
        origin_x: f32,
        origin_y: f32,
        max_width: f32,
        max_height: f32,
        inflate: f32,
    ) -> Self {
        Self {
            layout: LayoutKeyRef::from_style(text, style, max_width, max_height),
            origin_x_bits: origin_x.to_bits(),
            origin_y_bits: origin_y.to_bits(),
            inflate_bits: inflate.to_bits(),
        }
    }

    pub(in crate::d2d) fn matches(&self, key: &HitTestKey) -> bool {
        self.layout.matches(&key.layout)
            && self.origin_x_bits == key.origin_x_bits
            && self.origin_y_bits == key.origin_y_bits
            && self.inflate_bits == key.inflate_bits
    }

    pub(in crate::d2d) fn to_owned_reusing_layout(&self, layout: &TextLayoutKey) -> HitTestKey {
        debug_assert!(self.layout.matches(layout));
        HitTestKey {
            layout: layout.clone(),
            origin_x_bits: self.origin_x_bits,
            origin_y_bits: self.origin_y_bits,
            inflate_bits: self.inflate_bits,
        }
    }
}

pub(in crate::d2d) struct HitTestCache {
    pub(in crate::d2d) key: HitTestKey,
    pub(in crate::d2d) rects: Vec<RECT>,
}

#[cfg(test)]
#[path = "../../../tests/unit/d2d/cache/hit_test.rs"]
mod tests;
