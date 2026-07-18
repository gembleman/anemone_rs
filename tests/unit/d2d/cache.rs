use super::*;

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
fn alignment_is_part_of_layout_and_bitmap_keys() {
    let left = style();
    let layout_key = LayoutKeyRef::from_style("text", &left, 100.0, 50.0).to_owned();
    let bitmap_key = OutlineBitmapKeyRef::from_style("text", &left, 100.0, 50.0).to_owned();

    let mut center = left.clone();
    center.text_align = TextAlign::Center;
    assert!(!LayoutKeyRef::from_style("text", &center, 100.0, 50.0).matches(&layout_key));
    assert!(!OutlineBitmapKeyRef::from_style("text", &center, 100.0, 50.0).matches(&bitmap_key));
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

#[test]
fn inactive_effect_values_do_not_invalidate_bitmap_key() {
    let base = style();
    let key = OutlineBitmapKeyRef::from_style("text", &base, 100.0, 50.0).to_owned();

    let mut changed = base.clone();
    changed.outline1_color ^= u32::MAX;
    changed.outline2_color ^= u32::MAX;
    changed.shadow_color ^= u32::MAX;
    changed.shadow_offset_x = 0;
    changed.shadow_offset_y = 0;
    assert!(OutlineBitmapKeyRef::from_style("text", &changed, 100.0, 50.0).matches(&key));
}

#[test]
fn active_effect_values_invalidate_bitmap_key() {
    let mut base = style();
    base.outline1_size = 2;
    base.shadow_enabled = true;
    let key = OutlineBitmapKeyRef::from_style("text", &base, 100.0, 50.0).to_owned();

    let mut changed = base.clone();
    changed.outline1_color ^= u32::MAX;
    assert!(!OutlineBitmapKeyRef::from_style("text", &changed, 100.0, 50.0).matches(&key));

    let mut changed = base;
    changed.shadow_color ^= u32::MAX;
    assert!(!OutlineBitmapKeyRef::from_style("text", &changed, 100.0, 50.0).matches(&key));
}

#[test]
fn derived_keys_share_layout_strings() {
    let base = style();
    let layout = LayoutKeyRef::from_style("text", &base, 100.0, 50.0).to_owned();
    let bitmap = OutlineBitmapKeyRef::from_style("text", &base, 100.0, 50.0)
        .to_owned_reusing_layout(&layout);
    assert!(std::sync::Arc::ptr_eq(&layout.text, &bitmap.text));
    assert!(std::sync::Arc::ptr_eq(&layout.font_face, &bitmap.font_face));
}

#[test]
fn hit_test_key_tracks_position_and_inflate() {
    let base = style();
    let layout = LayoutKeyRef::from_style("text", &base, 100.0, 50.0).to_owned();
    let key = HitTestKeyRef::from_style("text", &base, 5.0, 7.0, 100.0, 50.0, 3.0)
        .to_owned_reusing_layout(&layout);

    assert!(HitTestKeyRef::from_style("text", &base, 5.0, 7.0, 100.0, 50.0, 3.0).matches(&key));
    assert!(!HitTestKeyRef::from_style("text", &base, 6.0, 7.0, 100.0, 50.0, 3.0).matches(&key));
    assert!(!HitTestKeyRef::from_style("text", &base, 5.0, 7.0, 100.0, 50.0, 4.0).matches(&key));
}

#[test]
fn miss_tracker_warms_up_enters_overload_and_recovers() {
    let mut tracker = MissTracker::new();
    for _ in 0..7 {
        tracker.record(true);
        assert!(!tracker.is_overloaded(), "warmup must fill the full window");
    }
    tracker.record(false);
    assert!(
        tracker.is_overloaded(),
        "seven misses in eight samples overload"
    );

    for _ in 0..8 {
        tracker.record(false);
    }
    assert!(
        !tracker.is_overloaded(),
        "stable hits must leave overload mode"
    );
}
