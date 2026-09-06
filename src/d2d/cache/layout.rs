//! Text layout 캐시 키와 항목.

use std::sync::Arc;

use crate::config::TextAlign;

use super::super::style::TextRenderStyle;
use windows::Win32::Graphics::{Direct2D::*, DirectWrite::*};

/// Layout 형태를 결정하는 값만 담은 캐시 키. 부동소수점은 bit 단위로 비교한다.
#[derive(Clone, Eq, PartialEq, Hash)]
pub(in crate::d2d) struct TextLayoutKey {
    pub(in crate::d2d) text: Arc<str>,
    pub(in crate::d2d) font_face: Arc<str>,
    pub(in crate::d2d) font_size: i32,
    pub(in crate::d2d) font_style: u8,
    pub(in crate::d2d) text_align: TextAlign,
    pub(in crate::d2d) max_width_bits: u32,
    pub(in crate::d2d) max_height_bits: u32,
}

/// Hit 조회에는 문자열을 빌리지 않고 miss일 때만 소유 키로 바꾸는 뷰.
pub(in crate::d2d) struct LayoutKeyRef<'a> {
    pub(in crate::d2d) text: &'a str,
    pub(in crate::d2d) font_face: &'a str,
    pub(in crate::d2d) font_size: i32,
    pub(in crate::d2d) font_style: u8,
    pub(in crate::d2d) text_align: TextAlign,
    pub(in crate::d2d) max_width_bits: u32,
    pub(in crate::d2d) max_height_bits: u32,
}

impl<'a> LayoutKeyRef<'a> {
    pub(in crate::d2d) fn from_style(
        text: &'a str,
        style: &'a TextRenderStyle,
        max_width: f32,
        max_height: f32,
    ) -> Self {
        Self {
            text,
            font_face: &style.font_face,
            font_size: style.font_size,
            font_style: style.font_style,
            text_align: style.text_align,
            max_width_bits: max_width.to_bits(),
            max_height_bits: max_height.to_bits(),
        }
    }

    pub(in crate::d2d) fn matches(&self, key: &TextLayoutKey) -> bool {
        self.font_size == key.font_size
            && self.font_style == key.font_style
            && self.text_align == key.text_align
            && self.max_width_bits == key.max_width_bits
            && self.max_height_bits == key.max_height_bits
            && self.text == &*key.text
            && self.font_face == &*key.font_face
    }

    /// `max_height_bits`를 제외하고 일치하는지 검사한다.
    ///
    /// bitmap 키는 `max_height`를 bitmap 높이 상한으로 쓰는 반면, layout은
    /// measure와 공유하는 `MEASURE_MAX_HEIGHT` 박스로 만들어져 의도적으로
    /// 어긋난다 — `OutlineBitmapKeyRef::to_owned_reusing_layout`의 문자열 재사용
    /// 검증에서만 쓰인다 (hit-test 키는 조회/저장 모두 1M이라 `matches`로 충분).
    pub(in crate::d2d) fn matches_ignoring_max_height(&self, key: &TextLayoutKey) -> bool {
        self.font_size == key.font_size
            && self.font_style == key.font_style
            && self.text_align == key.text_align
            && self.max_width_bits == key.max_width_bits
            && self.text == &*key.text
            && self.font_face == &*key.font_face
    }

    pub(in crate::d2d) fn to_owned(&self) -> TextLayoutKey {
        TextLayoutKey {
            text: Arc::from(self.text),
            font_face: Arc::from(self.font_face),
            font_size: self.font_size,
            font_style: self.font_style,
            text_align: self.text_align,
            max_width_bits: self.max_width_bits,
            max_height_bits: self.max_height_bits,
        }
    }
}

/// Outline geometry를 필요할 때 만드는 layout 캐시 항목.
pub(in crate::d2d) struct TextLayoutCache {
    pub(in crate::d2d) key: TextLayoutKey,
    pub(in crate::d2d) layout: IDWriteTextLayout,
    pub(in crate::d2d) outline: Option<ID2D1PathGeometry>,
}

#[cfg(test)]
#[path = "../../../tests/unit/d2d/cache/layout.rs"]
mod tests;
