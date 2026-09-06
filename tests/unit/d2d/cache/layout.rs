use super::*;

use crate::config::TextAlign;

fn style() -> TextRenderStyle {
    TextRenderStyle {
        font_size: 22,
        font_face: "Test Font".into(),
        font_style: 0,
        text_align: TextAlign::Left,
        color: 0xffff_ffff,
        outline1_size: 0,
        outline1_color: 0xff11_1111,
        outline2_size: 0,
        outline2_color: 0xff22_2222,
        shadow_enabled: false,
        shadow_color: 0xff33_3333,
        shadow_offset_x: 2,
        shadow_offset_y: 2,
    }
}

#[test]
fn paint_only_colors_do_not_invalidate_layout_key() {
    let base = style();
    let key = LayoutKeyRef::from_style("text", &base, 100.0, 50.0).to_owned();
    let mut changed = base;
    changed.color ^= u32::MAX;
    changed.outline1_color ^= u32::MAX;
    changed.shadow_color ^= u32::MAX;
    assert!(LayoutKeyRef::from_style("text", &changed, 100.0, 50.0).matches(&key));
}
