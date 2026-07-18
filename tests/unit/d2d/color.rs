use super::*;

#[test]
fn argb_channels_are_converted_to_normalized_floats() {
    assert_eq!(argb_to_color_f(0), D2D1_COLOR_F::default());
    assert_eq!(
        argb_to_color_f(0xffff_ffff),
        D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }
    );
    let color = argb_to_color_f(0x8040_20ff);
    assert_eq!(color.a, 128.0 / 255.0);
    assert_eq!(color.r, 64.0 / 255.0);
    assert_eq!(color.g, 32.0 / 255.0);
    assert_eq!(color.b, 1.0);
}

#[test]
fn font_style_uses_only_bold_and_italic_bits() {
    assert_eq!(
        font_style_to_dwrite(0),
        (DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL)
    );
    assert_eq!(
        font_style_to_dwrite(1),
        (DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_STYLE_NORMAL)
    );
    assert_eq!(
        font_style_to_dwrite(2),
        (DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_ITALIC)
    );
    assert_eq!(
        font_style_to_dwrite(u8::MAX),
        (DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_STYLE_ITALIC)
    );
}

#[test]
fn text_alignment_maps_to_directwrite() {
    assert_eq!(
        text_align_to_dwrite(TextAlign::Left),
        DWRITE_TEXT_ALIGNMENT_LEADING
    );
    assert_eq!(
        text_align_to_dwrite(TextAlign::Center),
        DWRITE_TEXT_ALIGNMENT_CENTER
    );
    assert_eq!(
        text_align_to_dwrite(TextAlign::Right),
        DWRITE_TEXT_ALIGNMENT_TRAILING
    );
}
