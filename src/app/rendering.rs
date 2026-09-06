use std::sync::Arc;

use windows_sys::Win32::Foundation::RECT;

use crate::config::{Config, TextStyle, TextType};
use crate::d2d::MeasureSlot;
use crate::d2d::TextRenderStyle;

/// [`super::paint`] 렌더 패스가 그리는 한 줄짜리 텍스트 블록.
pub(super) struct RenderBlock {
    pub(super) text: String,
    pub(super) style: TextRenderStyle,
    pub(super) slot: MeasureSlot,
    pub(super) top: f32,
    pub(super) height: f32,
    pub(super) gap_after: f32,
}

/// 텍스트 유형별 measure 결과 슬롯. paint는 유형별로 최대 1블록만 측정한다.
fn measure_slot_for(text_type: TextType) -> MeasureSlot {
    match text_type {
        TextType::Name => MeasureSlot::Name,
        TextType::Original => MeasureSlot::Original,
        TextType::Translation => MeasureSlot::Translation,
    }
}

fn split_name(text: &str) -> Option<(&str, &str)> {
    text.split_once([':', '：'])
        .map(|(name, body)| (name.trim(), body.trim()))
        .filter(|(name, _)| !name.is_empty())
}

fn display_segments(config: &Config, original: &str, translated: &str) -> Vec<(TextType, String)> {
    let (original_name, original_body) = if config.separate_name {
        split_name(original).map_or((None, original), |(name, body)| (Some(name), body))
    } else {
        (None, original)
    };
    let (translated_name, translated_body) = if config.separate_name {
        split_name(translated).map_or((None, translated), |(name, body)| (Some(name), body))
    } else {
        (None, translated)
    };

    let mut segments = Vec::with_capacity(3);
    if config.show_name
        && let Some(name) = translated_name.or(original_name)
        && !name.is_empty()
    {
        segments.push((TextType::Name, name.to_string()));
    }
    if config.show_original && !original_body.is_empty() {
        segments.push((TextType::Original, original_body.to_string()));
    }
    if config.show_translation && !translated_body.is_empty() {
        segments.push((TextType::Translation, translated_body.to_string()));
    }
    segments
}

fn render_style(config: &Config, style: &TextStyle) -> TextRenderStyle {
    TextRenderStyle {
        font_size: style.size,
        font_face: Arc::from(style.font_face.as_str()),
        font_style: style.font_style,
        text_align: config.text_align,
        color: style.color_primary,
        outline1_size: style.outline1_size,
        outline1_color: style.color_outline1,
        outline2_size: style.outline2_size,
        outline2_color: style.color_outline2,
        shadow_enabled: style.shadow_enabled,
        shadow_color: style.color_shadow,
        shadow_offset_x: config.shadow_offset_x,
        shadow_offset_y: config.shadow_offset_y,
    }
}

pub(super) fn build_render_blocks(
    config: &Config,
    original: &str,
    translated: &str,
) -> Vec<RenderBlock> {
    let mut top = config.text_margin_y as f32;
    display_segments(config, original, translated)
        .into_iter()
        .map(|(text_type, text)| {
            let style = render_style(config, config.get_text_style(text_type));
            let line_count = text.lines().count().max(1) as f32;
            let height = line_count * (style.font_size.max(1) as f32 * 1.35);
            let block = RenderBlock {
                text,
                style,
                slot: measure_slot_for(text_type),
                top,
                height,
                gap_after: if text_type == TextType::Name {
                    config.name_margin as f32
                } else {
                    0.0
                },
            };
            top += height;
            top += block.gap_after;
            block
        })
        .collect()
}

pub(super) fn build_notice_render_blocks(config: &Config, text: &str) -> Vec<RenderBlock> {
    let style = render_style(config, &config.translation_style);
    let height = text.lines().count().max(1) as f32 * (style.font_size.max(1) as f32 * 1.35);
    vec![RenderBlock {
        text: text.to_string(),
        style,
        slot: MeasureSlot::Notice,
        top: config.text_margin_y as f32,
        height,
        gap_after: 0.0,
    }]
}

#[inline]
pub(super) fn pixels_to_dips(value: i32, dpi: u32) -> f32 {
    let dpi = if dpi == 0 { crate::dpi::BASE_DPI } else { dpi };
    value as f32 * crate::dpi::BASE_DPI as f32 / dpi as f32
}

#[inline]
fn dip_floor_to_pixels(value: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { crate::dpi::BASE_DPI } else { dpi };
    let scaled = value as i64 * dpi as i64;
    scaled.div_euclid(crate::dpi::BASE_DPI as i64) as i32
}

#[inline]
fn dip_ceil_to_pixels(value: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { crate::dpi::BASE_DPI } else { dpi };
    let scaled = value as i64 * dpi as i64;
    (-(-scaled).div_euclid(crate::dpi::BASE_DPI as i64)) as i32
}

pub(super) fn dip_rect_to_pixels(rect: &windows::Win32::Foundation::RECT, dpi: u32) -> RECT {
    RECT {
        left: dip_floor_to_pixels(rect.left, dpi),
        top: dip_floor_to_pixels(rect.top, dpi),
        right: dip_ceil_to_pixels(rect.right, dpi),
        bottom: dip_ceil_to_pixels(rect.bottom, dpi),
    }
}

pub(super) fn composition_retry_delay_ms(failures: u32) -> u32 {
    const INITIAL_MS: u32 = 250;
    const MAX_SHIFT: u32 = 5;
    INITIAL_MS.saturating_mul(1u32 << failures.min(MAX_SHIFT))
}

#[cfg(test)]
#[path = "../../tests/unit/app/rendering.rs"]
mod tests;
