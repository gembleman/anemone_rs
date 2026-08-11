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
fn measure_key_ignores_max_height_and_paint_only_colors() {
    let base = style();
    let key = MeasureKeyRef::from_style("text", &base, 100.0).to_owned();

    // measure는 항상 같은 높이 상한을 쓰므로 max_height는 키 변별력이 없다.
    assert!(MeasureKeyRef::from_style("text", &base, 100.0).matches(&key));
    // 색상·외곽선·그림자는 layout metrics 높이에 영향이 없다.
    let mut changed = base.clone();
    changed.color ^= u32::MAX;
    changed.outline1_color ^= u32::MAX;
    changed.shadow_color ^= u32::MAX;
    changed.outline1_size = 3;
    changed.shadow_enabled = true;
    assert!(MeasureKeyRef::from_style("text", &changed, 100.0).matches(&key));
}

#[test]
fn measure_key_tracks_text_font_and_width() {
    let base = style();
    let key = MeasureKeyRef::from_style("text", &base, 100.0).to_owned();

    let mut changed = base.clone();
    changed.font_size = 30;
    assert!(!MeasureKeyRef::from_style("text", &changed, 100.0).matches(&key));

    let mut changed = base.clone();
    changed.font_style = 1;
    assert!(!MeasureKeyRef::from_style("text", &changed, 100.0).matches(&key));

    let mut changed = base;
    changed.text_align = TextAlign::Center;
    assert!(!MeasureKeyRef::from_style("text", &changed, 100.0).matches(&key));
    assert!(!MeasureKeyRef::from_style("other", &changed, 100.0).matches(&key));
    assert!(!MeasureKeyRef::from_style("text", &changed, 80.0).matches(&key));
}

#[test]
fn measure_cache_warms_up_and_hits_multi_block_sequence() {
    let base = style();
    let mut cache = MeasureCache::new();
    let block_a = MeasureKeyRef::from_style("이름", &base, 100.0).to_owned();
    let block_b = MeasureKeyRef::from_style("대사", &base, 100.0).to_owned();

    // 2블록 paint: 첫 paint는 전부 miss (워밍업)
    cache.insert(block_a.clone(), 30.0);
    cache.insert(block_b.clone(), 20.0);
    assert!(cache
        .get(&MeasureKeyRef::from_style("이름", &base, 100.0))
        .is_some());

    // 두 번째 paint: 같은 순서 호출이면 두 블록 모두 hit
    assert_eq!(
        cache.get(&MeasureKeyRef::from_style("이름", &base, 100.0)),
        Some(30.0)
    );
    assert_eq!(
        cache.get(&MeasureKeyRef::from_style("대사", &base, 100.0)),
        Some(20.0)
    );

    // hit는 순환 인덱스를 움직이지 않으므로 세 번째 paint도 hit 유지
    assert_eq!(
        cache.get(&MeasureKeyRef::from_style("이름", &base, 100.0)),
        Some(30.0)
    );
    assert_eq!(
        cache.get(&MeasureKeyRef::from_style("대사", &base, 100.0)),
        Some(20.0)
    );
}

#[test]
fn measure_cache_fifo_evicts_oldest_slot_on_fourth_key() {
    let base = style();
    let mut cache = MeasureCache::new();
    let keys: Vec<MeasureKey> = (0..4)
        .map(|i| {
            MeasureKeyRef::from_style(&format!("block{i}"), &base, 100.0)
                .to_owned()
        })
        .collect();

    for key in &keys[..3] {
        cache.insert(key.clone(), 10.0);
    }
    // 3슬롯이 다 찬 뒤 4번째 키는 가장 오래된 슬롯(첫 키)을 덮어쓴다.
    cache.insert(keys[3].clone(), 40.0);

    assert_eq!(cache.get(&MeasureKeyRef::from_style("block0", &base, 100.0)), None);
    assert_eq!(
        cache.get(&MeasureKeyRef::from_style("block1", &base, 100.0)),
        Some(10.0)
    );
    assert_eq!(
        cache.get(&MeasureKeyRef::from_style("block2", &base, 100.0)),
        Some(10.0)
    );
    assert_eq!(
        cache.get(&MeasureKeyRef::from_style("block3", &base, 100.0)),
        Some(40.0)
    );

    // 블록 1~3 순서 호출(2블록/3블록 paint와 유사)은 전부 hit한다.
    for i in 1..4 {
        assert!(cache
            .get(&MeasureKeyRef::from_style(&format!("block{i}"), &base, 100.0))
            .is_some());
    }
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
