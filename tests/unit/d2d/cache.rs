use super::*;

use crate::d2d::MeasureSlot;

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
    cache.insert(MeasureSlot::Name, block_a.clone(), 30.0);
    cache.insert(MeasureSlot::Translation, block_b.clone(), 20.0);
    assert!(cache
        .get(MeasureSlot::Name, &MeasureKeyRef::from_style("이름", &base, 100.0))
        .is_some());

    // 두 번째 paint: 같은 슬롯 순서 호출이면 두 블록 모두 hit
    assert_eq!(
        cache.get(MeasureSlot::Name, &MeasureKeyRef::from_style("이름", &base, 100.0)),
        Some(30.0)
    );
    assert_eq!(
        cache
            .get(MeasureSlot::Translation, &MeasureKeyRef::from_style("대사", &base, 100.0)),
        Some(20.0)
    );

    // hit는 아무 상태도 바꾸지 않으므로 세 번째 paint도 hit 유지
    assert_eq!(
        cache.get(MeasureSlot::Name, &MeasureKeyRef::from_style("이름", &base, 100.0)),
        Some(30.0)
    );
    assert_eq!(
        cache
            .get(MeasureSlot::Translation, &MeasureKeyRef::from_style("대사", &base, 100.0)),
        Some(20.0)
    );
}

#[test]
fn measure_cache_slots_are_independent_by_text_type() {
    let base = style();
    let mut cache = MeasureCache::new();
    cache.insert(
        MeasureSlot::Name,
        MeasureKeyRef::from_style("이름", &base, 100.0).to_owned(),
        30.0,
    );
    cache.insert(
        MeasureSlot::Translation,
        MeasureKeyRef::from_style("대사", &base, 100.0).to_owned(),
        20.0,
    );

    // 같은 슬롯·같은 키: hit.
    assert_eq!(
        cache.get(MeasureSlot::Name, &MeasureKeyRef::from_style("이름", &base, 100.0)),
        Some(30.0)
    );
    assert_eq!(
        cache
            .get(MeasureSlot::Translation, &MeasureKeyRef::from_style("대사", &base, 100.0)),
        Some(20.0)
    );

    // Translation 슬롯을 다른 텍스트로 덮어써도 Name 슬롯은 그대로 hit
    // (FIFO라면 순환 퇴거로 밀려났을 케이스).
    cache.insert(
        MeasureSlot::Translation,
        MeasureKeyRef::from_style("새 대사", &base, 100.0).to_owned(),
        25.0,
    );
    assert_eq!(
        cache.get(MeasureSlot::Name, &MeasureKeyRef::from_style("이름", &base, 100.0)),
        Some(30.0)
    );
    assert_eq!(
        cache
            .get(MeasureSlot::Translation, &MeasureKeyRef::from_style("새 대사", &base, 100.0)),
        Some(25.0)
    );
}

#[test]
fn measure_cache_same_slot_replaces_on_key_change() {
    let base = style();
    let mut cache = MeasureCache::new();
    cache.insert(
        MeasureSlot::Name,
        MeasureKeyRef::from_style("이름", &base, 100.0).to_owned(),
        30.0,
    );
    cache.insert(
        MeasureSlot::Translation,
        MeasureKeyRef::from_style("대사", &base, 100.0).to_owned(),
        20.0,
    );

    // 같은 슬롯(Name)에 다른 텍스트가 오면 옛 키는 miss, 새 키는 hit.
    cache.insert(
        MeasureSlot::Name,
        MeasureKeyRef::from_style("다른 이름", &base, 100.0).to_owned(),
        35.0,
    );
    assert_eq!(
        cache.get(MeasureSlot::Name, &MeasureKeyRef::from_style("이름", &base, 100.0)),
        None
    );
    assert_eq!(
        cache
            .get(MeasureSlot::Name, &MeasureKeyRef::from_style("다른 이름", &base, 100.0)),
        Some(35.0)
    );
    // 다른 슬롯(Translation)은 불변.
    assert_eq!(
        cache
            .get(MeasureSlot::Translation, &MeasureKeyRef::from_style("대사", &base, 100.0)),
        Some(20.0)
    );
}

#[test]
fn measure_cache_notice_slot_does_not_evict_translation() {
    let base = style();
    let mut cache = MeasureCache::new();
    cache.insert(
        MeasureSlot::Translation,
        MeasureKeyRef::from_style("대사", &base, 100.0).to_owned(),
        20.0,
    );

    // notice 슬롯을 사용해도 Translation 슬롯은 건드리지 않는다.
    cache.insert(
        MeasureSlot::Notice,
        MeasureKeyRef::from_style("업데이트 확인 중...", &base, 100.0).to_owned(),
        15.0,
    );
    assert_eq!(
        cache
            .get(MeasureSlot::Translation, &MeasureKeyRef::from_style("대사", &base, 100.0)),
        Some(20.0)
    );
    assert_eq!(
        cache
            .get(MeasureSlot::Notice, &MeasureKeyRef::from_style("업데이트 확인 중...", &base, 100.0)),
        Some(15.0)
    );
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
    let slot = MeasureSlot::Translation;
    let mut tracker = MissTracker::new();
    for _ in 0..7 {
        tracker.record(slot, true);
        assert!(!tracker.is_overloaded(slot), "warmup must fill the full window");
    }
    tracker.record(slot, false);
    assert!(
        tracker.is_overloaded(slot),
        "seven misses in eight samples overload"
    );

    for _ in 0..8 {
        tracker.record(slot, false);
    }
    assert!(
        !tracker.is_overloaded(slot),
        "stable hits must leave overload mode"
    );
}

#[test]
fn miss_tracker_overload_state_is_independent_per_slot() {
    let mut tracker = MissTracker::new();
    // Translation 슬롯만 miss로 채운다.
    for _ in 0..8 {
        tracker.record(MeasureSlot::Translation, true);
    }
    assert!(tracker.is_overloaded(MeasureSlot::Translation));
    // Name 슬롯은 hit만 기록 — 전역 ring이었다면 같이 overload됐을 케이스.
    for _ in 0..8 {
        tracker.record(MeasureSlot::Name, false);
    }
    assert!(!tracker.is_overloaded(MeasureSlot::Name));
    // 안정 Name의 hit가 Translation의 ring을 희석하지도 않는다.
    assert!(tracker.is_overloaded(MeasureSlot::Translation));
}

#[test]
fn unused_slot_tracker_keeps_cache_within_grace_window() {
    let mut tracker = UnusedSlotTracker::new();
    let mut used = [true; MeasureSlot::COUNT];

    // 연속 미사용이 GRACE 미만이면 폐기되지 않는다.
    for _ in 0..(UnusedSlotTracker::GRACE - 1) {
        used[MeasureSlot::Name as usize] = false;
        let to_prune = tracker.record(used);
        assert!(
            !to_prune[MeasureSlot::Name as usize],
            "unused span within grace must keep the cache"
        );
        used[MeasureSlot::Name as usize] = true;
    }
    let to_prune = tracker.record(used);
    assert!(!to_prune[MeasureSlot::Name as usize]);
}

#[test]
fn unused_slot_tracker_survives_indefinite_alternation() {
    let mut tracker = UnusedSlotTracker::new();
    // 대사(Name 있음) ↔ 지문(Name 없음)을 GRACE의 몇 배로 반복해도
    // 카운터가 매번 리셋되므로 폐기되지 않는다 (8132aab 회귀 고정).
    for _ in 0..(UnusedSlotTracker::GRACE as usize * 4) {
        let mut narration = [true; MeasureSlot::COUNT];
        narration[MeasureSlot::Name as usize] = false;
        assert!(!tracker.record(narration)[MeasureSlot::Name as usize]);
        // 다음 대사 줄에서 Name이 돌아오며 카운터가 리셋된다.
        assert!(!tracker.record([true; MeasureSlot::COUNT])[MeasureSlot::Name as usize]);
    }
}

#[test]
fn unused_slot_tracker_prunes_after_grace_period_and_restarts() {
    let mut tracker = UnusedSlotTracker::new();
    let used = [true; MeasureSlot::COUNT];

    // 연속 GRACE 프레임 미사용 → 폐기 대상 보고.
    for frame in 0..UnusedSlotTracker::GRACE {
        let mut used = used;
        used[MeasureSlot::Translation as usize] = false;
        let to_prune = tracker.record(used);
        if frame < UnusedSlotTracker::GRACE - 1 {
            assert!(!to_prune[MeasureSlot::Translation as usize]);
        } else {
            assert!(to_prune[MeasureSlot::Translation as usize]);
        }
    }
    // 폐기 보고 후 카운터가 리셋되어 다음 유예 주기가 시작된다.
    let mut used = used;
    used[MeasureSlot::Translation as usize] = false;
    assert!(!tracker.record(used)[MeasureSlot::Translation as usize]);
}

#[test]
fn unused_slot_tracker_slots_are_independent() {
    let mut tracker = UnusedSlotTracker::new();
    let used = [true; MeasureSlot::COUNT];

    // Notice만 장기 미사용(show notice off 후) — 유예 주기 동안 Name은
    // 계속 사용되어 폐기 대상이 되지 않는다.
    for frame in 0..UnusedSlotTracker::GRACE {
        let mut used = used;
        used[MeasureSlot::Notice as usize] = false;
        let to_prune = tracker.record(used);
        assert!(!to_prune[MeasureSlot::Name as usize]);
        if frame == UnusedSlotTracker::GRACE - 1 {
            assert!(to_prune[MeasureSlot::Notice as usize]);
        } else {
            assert!(!to_prune[MeasureSlot::Notice as usize]);
        }
    }
}

#[test]
fn miss_tracker_reset_clears_ring_filled_and_last_key() {
    let base = style();
    let mut tracker = MissTracker::new();
    let slot = MeasureSlot::Translation;

    for _ in 0..8 {
        tracker.record(slot, true);
    }
    tracker.set_last_key(
        slot,
        Some(OutlineBitmapKeyRef::from_style("대사", &base, 100.0, 50.0).to_owned()),
    );
    assert!(tracker.is_overloaded(slot));

    tracker.reset(slot);
    assert!(!tracker.is_overloaded(slot), "ring and filled must reset");
    assert!(tracker.last_key(slot).is_none(), "last_key must reset");
    // 다른 슬롯은 영향받지 않는다.
    for _ in 0..8 {
        tracker.record(MeasureSlot::Name, true);
    }
    assert!(tracker.is_overloaded(MeasureSlot::Name));
    tracker.reset(slot);
    assert!(tracker.is_overloaded(MeasureSlot::Name));
}

#[test]
fn miss_tracker_last_key_is_independent_per_slot() {
    let base = style();
    let mut tracker = MissTracker::new();

    let name_key = OutlineBitmapKeyRef::from_style("이름", &base, 100.0, 50.0).to_owned();
    let trans_key = OutlineBitmapKeyRef::from_style("대사", &base, 100.0, 50.0).to_owned();

    tracker.set_last_key(MeasureSlot::Name, Some(name_key.clone()));
    // 다른 슬롯은 아직 비어 있다 — 슬롯 간에 키가 섞이지 않는다.
    assert!(tracker.last_key(MeasureSlot::Translation).is_none());
    assert!(OutlineBitmapKeyRef::from_style("이름", &base, 100.0, 50.0)
        .matches(tracker.last_key(MeasureSlot::Name).unwrap()));

    tracker.set_last_key(MeasureSlot::Translation, Some(trans_key.clone()));
    assert!(OutlineBitmapKeyRef::from_style("대사", &base, 100.0, 50.0)
        .matches(tracker.last_key(MeasureSlot::Translation).unwrap()));
    // Name 슬롯은 Translation 갱신의 영향을 받지 않는다.
    assert!(OutlineBitmapKeyRef::from_style("이름", &base, 100.0, 50.0)
        .matches(tracker.last_key(MeasureSlot::Name).unwrap()));

    tracker.set_last_key(MeasureSlot::Name, None);
    assert!(tracker.last_key(MeasureSlot::Name).is_none());
    assert!(tracker.last_key(MeasureSlot::Translation).is_some());
}
