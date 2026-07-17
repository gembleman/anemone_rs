use windows::Win32::Graphics::{Direct2D::Common::*, DirectWrite::*};

// ── ARGB 컬러 변환 헬퍼 ─────────────────────────────────

/// ARGB u32를 D2D1_COLOR_F로 변환
#[inline]
pub(super) fn argb_to_color_f(color: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        a: ((color >> 24) & 0xFF) as f32 / 255.0,
        r: ((color >> 16) & 0xFF) as f32 / 255.0,
        g: ((color >> 8) & 0xFF) as f32 / 255.0,
        b: (color & 0xFF) as f32 / 255.0,
    }
}

// ── font_style 비트 → DirectWrite 변환 ──────────────────

/// font_style 비트플래그(0: normal, 1: bold, 2: italic, 3: bold+italic)를
/// DirectWrite의 weight/style 쌍으로 변환
#[inline]
pub(super) fn font_style_to_dwrite(bits: u8) -> (DWRITE_FONT_WEIGHT, DWRITE_FONT_STYLE) {
    let weight = if bits & 1 != 0 {
        DWRITE_FONT_WEIGHT_BOLD
    } else {
        DWRITE_FONT_WEIGHT_NORMAL
    };
    let style = if bits & 2 != 0 {
        DWRITE_FONT_STYLE_ITALIC
    } else {
        DWRITE_FONT_STYLE_NORMAL
    };
    (weight, style)
}
