use super::*;

fn channels(argb: u32) -> (u8, u8, u8, u8) {
    (
        ((argb >> 24) & 0xFF) as u8,
        ((argb >> 16) & 0xFF) as u8,
        ((argb >> 8) & 0xFF) as u8,
        (argb & 0xFF) as u8,
    )
}

#[test]
fn test_color_result() {
    let color = ColorResult { argb: 0x80FF8040 };
    assert_eq!(channels(color.argb), (0x80, 0xFF, 0x80, 0x40));
}

#[test]
fn test_colorref_conversion() {
    let result = ColorResult::from_colorref(0x402080, 0xC0); // BGR -> RGB
    assert_eq!(channels(result.argb), (0xC0, 0x80, 0x20, 0x40));
}
