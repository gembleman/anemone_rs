use super::*;

use crate::config::TextAlign;
use crate::d2d::MeasureSlot;
use crate::d2d::cache::outline_bitmap::OutlineBitmapKeyRef;
use crate::d2d::style::TextRenderStyle;

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
fn miss_tracker_warms_up_enters_overload_and_recovers() {
    let slot = MeasureSlot::Translation;
    let mut tracker = MissTracker::new();
    for _ in 0..7 {
        tracker.record(slot, true);
        assert!(
            !tracker.is_overloaded(slot),
            "warmup must fill the full window"
        );
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

    // 연속 GRACE paint 미사용 → 폐기 대상 보고.
    for paint in 0..UnusedSlotTracker::GRACE {
        let mut used = used;
        used[MeasureSlot::Translation as usize] = false;
        let to_prune = tracker.record(used);
        if paint < UnusedSlotTracker::GRACE - 1 {
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
    for paint in 0..UnusedSlotTracker::GRACE {
        let mut used = used;
        used[MeasureSlot::Notice as usize] = false;
        let to_prune = tracker.record(used);
        assert!(!to_prune[MeasureSlot::Name as usize]);
        if paint == UnusedSlotTracker::GRACE - 1 {
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
    assert!(
        OutlineBitmapKeyRef::from_style("이름", &base, 100.0, 50.0)
            .matches(tracker.last_key(MeasureSlot::Name).unwrap())
    );

    tracker.set_last_key(MeasureSlot::Translation, Some(trans_key.clone()));
    assert!(
        OutlineBitmapKeyRef::from_style("대사", &base, 100.0, 50.0)
            .matches(tracker.last_key(MeasureSlot::Translation).unwrap())
    );
    // Name 슬롯은 Translation 갱신의 영향을 받지 않는다.
    assert!(
        OutlineBitmapKeyRef::from_style("이름", &base, 100.0, 50.0)
            .matches(tracker.last_key(MeasureSlot::Name).unwrap())
    );

    tracker.set_last_key(MeasureSlot::Name, None);
    assert!(tracker.last_key(MeasureSlot::Name).is_none());
    assert!(tracker.last_key(MeasureSlot::Translation).is_some());
}
