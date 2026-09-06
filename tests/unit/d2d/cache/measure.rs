use super::*;

use crate::config::TextAlign;
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
    assert!(
        cache
            .get(
                MeasureSlot::Name,
                &MeasureKeyRef::from_style("이름", &base, 100.0)
            )
            .is_some()
    );

    // 두 번째 paint: 같은 슬롯 순서 호출이면 두 블록 모두 hit
    assert_eq!(
        cache.get(
            MeasureSlot::Name,
            &MeasureKeyRef::from_style("이름", &base, 100.0)
        ),
        Some(30.0)
    );
    assert_eq!(
        cache.get(
            MeasureSlot::Translation,
            &MeasureKeyRef::from_style("대사", &base, 100.0)
        ),
        Some(20.0)
    );

    // hit는 아무 상태도 바꾸지 않으므로 세 번째 paint도 hit 유지
    assert_eq!(
        cache.get(
            MeasureSlot::Name,
            &MeasureKeyRef::from_style("이름", &base, 100.0)
        ),
        Some(30.0)
    );
    assert_eq!(
        cache.get(
            MeasureSlot::Translation,
            &MeasureKeyRef::from_style("대사", &base, 100.0)
        ),
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
        cache.get(
            MeasureSlot::Name,
            &MeasureKeyRef::from_style("이름", &base, 100.0)
        ),
        Some(30.0)
    );
    assert_eq!(
        cache.get(
            MeasureSlot::Translation,
            &MeasureKeyRef::from_style("대사", &base, 100.0)
        ),
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
        cache.get(
            MeasureSlot::Name,
            &MeasureKeyRef::from_style("이름", &base, 100.0)
        ),
        Some(30.0)
    );
    assert_eq!(
        cache.get(
            MeasureSlot::Translation,
            &MeasureKeyRef::from_style("새 대사", &base, 100.0)
        ),
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
        cache.get(
            MeasureSlot::Name,
            &MeasureKeyRef::from_style("이름", &base, 100.0)
        ),
        None
    );
    assert_eq!(
        cache.get(
            MeasureSlot::Name,
            &MeasureKeyRef::from_style("다른 이름", &base, 100.0)
        ),
        Some(35.0)
    );
    // 다른 슬롯(Translation)은 불변.
    assert_eq!(
        cache.get(
            MeasureSlot::Translation,
            &MeasureKeyRef::from_style("대사", &base, 100.0)
        ),
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
        cache.get(
            MeasureSlot::Translation,
            &MeasureKeyRef::from_style("대사", &base, 100.0)
        ),
        Some(20.0)
    );
    assert_eq!(
        cache.get(
            MeasureSlot::Notice,
            &MeasureKeyRef::from_style("업데이트 확인 중...", &base, 100.0)
        ),
        Some(15.0)
    );
}
