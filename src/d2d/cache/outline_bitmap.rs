//! 색상, 두께, 그림자까지 포함한 outline/shadow bitmap 캐시 키와 항목.

use std::sync::Arc;

use crate::config::TextAlign;

use super::super::style::TextRenderStyle;
use super::effective_style::EffectiveOutlineStyle;
use super::layout::{LayoutKeyRef, TextLayoutKey};
use windows::Win32::Graphics::{Direct2D::*, DirectWrite::*};

/// 색상, 두께, 그림자까지 포함한 outline/shadow bitmap 캐시 키.
#[derive(Clone, Eq, PartialEq, Hash)]
pub(in crate::d2d) struct OutlineBitmapKey {
    pub(in crate::d2d) text: Arc<str>,
    pub(in crate::d2d) font_face: Arc<str>,
    pub(in crate::d2d) font_size: i32,
    pub(in crate::d2d) font_style: u8,
    pub(in crate::d2d) text_align: TextAlign,
    pub(in crate::d2d) max_width_bits: u32,
    pub(in crate::d2d) max_height_bits: u32,
    pub(in crate::d2d) outline1_size: i32,
    pub(in crate::d2d) outline1_color: u32,
    pub(in crate::d2d) outline2_size: i32,
    pub(in crate::d2d) outline2_color: u32,
    pub(in crate::d2d) shadow_enabled: bool,
    pub(in crate::d2d) shadow_color: u32,
    pub(in crate::d2d) shadow_offset_x: i32,
    pub(in crate::d2d) shadow_offset_y: i32,
}

/// 문자열 할당 없이 bitmap 캐시를 조회하는 뷰.
pub(in crate::d2d) struct OutlineBitmapKeyRef<'a> {
    pub(in crate::d2d) text: &'a str,
    pub(in crate::d2d) font_face: &'a str,
    pub(in crate::d2d) font_size: i32,
    pub(in crate::d2d) font_style: u8,
    pub(in crate::d2d) text_align: TextAlign,
    pub(in crate::d2d) max_width_bits: u32,
    pub(in crate::d2d) max_height_bits: u32,
    pub(in crate::d2d) outline1_size: i32,
    pub(in crate::d2d) outline1_color: u32,
    pub(in crate::d2d) outline2_size: i32,
    pub(in crate::d2d) outline2_color: u32,
    pub(in crate::d2d) shadow_enabled: bool,
    pub(in crate::d2d) shadow_color: u32,
    pub(in crate::d2d) shadow_offset_x: i32,
    pub(in crate::d2d) shadow_offset_y: i32,
}

impl<'a> OutlineBitmapKeyRef<'a> {
    pub(in crate::d2d) fn from_style(
        text: &'a str,
        style: &'a TextRenderStyle,
        max_width: f32,
        max_height: f32,
    ) -> Self {
        let effects = EffectiveOutlineStyle::from_style(style);
        Self {
            text,
            font_face: &style.font_face,
            font_size: style.font_size,
            font_style: style.font_style,
            text_align: style.text_align,
            max_width_bits: max_width.to_bits(),
            max_height_bits: max_height.to_bits(),
            outline1_size: effects.outline1_size,
            outline1_color: effects.outline1_color,
            outline2_size: effects.outline2_size,
            outline2_color: effects.outline2_color,
            shadow_enabled: effects.has_shadow,
            shadow_color: effects.shadow_color,
            shadow_offset_x: effects.shadow_offset_x,
            shadow_offset_y: effects.shadow_offset_y,
        }
    }

    pub(in crate::d2d) fn matches(&self, key: &OutlineBitmapKey) -> bool {
        self.font_size == key.font_size
            && self.font_style == key.font_style
            && self.text_align == key.text_align
            && self.max_width_bits == key.max_width_bits
            && self.max_height_bits == key.max_height_bits
            && self.outline1_size == key.outline1_size
            && self.outline1_color == key.outline1_color
            && self.outline2_size == key.outline2_size
            && self.outline2_color == key.outline2_color
            && self.shadow_enabled == key.shadow_enabled
            && self.shadow_color == key.shadow_color
            && self.shadow_offset_x == key.shadow_offset_x
            && self.shadow_offset_y == key.shadow_offset_y
            && self.text == &*key.text
            && self.font_face == &*key.font_face
    }

    #[cfg(test)]
    pub(in crate::d2d) fn to_owned(&self) -> OutlineBitmapKey {
        OutlineBitmapKey {
            text: Arc::from(self.text),
            font_face: Arc::from(self.font_face),
            font_size: self.font_size,
            font_style: self.font_style,
            text_align: self.text_align,
            max_width_bits: self.max_width_bits,
            max_height_bits: self.max_height_bits,
            outline1_size: self.outline1_size,
            outline1_color: self.outline1_color,
            outline2_size: self.outline2_size,
            outline2_color: self.outline2_color,
            shadow_enabled: self.shadow_enabled,
            shadow_color: self.shadow_color,
            shadow_offset_x: self.shadow_offset_x,
            shadow_offset_y: self.shadow_offset_y,
        }
    }

    /// 이미 생성된 layout key의 문자열 소유권을 공유해 중복 할당을 피한다.
    pub(in crate::d2d) fn to_owned_reusing_layout(
        &self,
        layout_key: &TextLayoutKey,
    ) -> OutlineBitmapKey {
        debug_assert!(
            LayoutKeyRef {
                text: self.text,
                font_face: self.font_face,
                font_size: self.font_size,
                font_style: self.font_style,
                text_align: self.text_align,
                max_width_bits: self.max_width_bits,
                max_height_bits: layout_key.max_height_bits,
            }
            .matches_ignoring_max_height(layout_key)
        );
        OutlineBitmapKey {
            text: Arc::clone(&layout_key.text),
            font_face: Arc::clone(&layout_key.font_face),
            font_size: self.font_size,
            font_style: self.font_style,
            text_align: self.text_align,
            max_width_bits: self.max_width_bits,
            max_height_bits: self.max_height_bits,
            outline1_size: self.outline1_size,
            outline1_color: self.outline1_color,
            outline2_size: self.outline2_size,
            outline2_color: self.outline2_color,
            shadow_enabled: self.shadow_enabled,
            shadow_color: self.shadow_color,
            shadow_offset_x: self.shadow_offset_x,
            shadow_offset_y: self.shadow_offset_y,
        }
    }
}

/// 음수 방향 효과까지 담도록 layout 원점 앞에 padding을 둔 중간 bitmap.
pub(in crate::d2d) struct OutlineBitmap {
    pub(in crate::d2d) key: OutlineBitmapKey,
    pub(in crate::d2d) bitmap: ID2D1Bitmap,
    /// 비트맵을 만든 layout — bitmap 키가 layout 키를 포함하므로 hit 시
    /// 재검증 없이 이 항목에서 바로 쓸 수 있다.
    pub(in crate::d2d) layout: IDWriteTextLayout,
    /// 비트맵 가로/세로 크기 (DIP). DrawBitmap 의 dest 사각형 크기.
    pub(in crate::d2d) width: f32,
    pub(in crate::d2d) height: f32,
    /// 비트맵 안에서 layout 원점이 떨어지는 위치 (DIP).
    pub(in crate::d2d) pad_left: f32,
    pub(in crate::d2d) pad_top: f32,
}

#[cfg(test)]
#[path = "../../../tests/unit/d2d/cache/outline_bitmap.rs"]
mod tests;
