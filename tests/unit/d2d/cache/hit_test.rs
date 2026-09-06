use super::*;

use crate::config::TextAlign;
use crate::d2d::cache::layout::LayoutKeyRef;

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
fn hit_test_key_matches_when_layout_uses_measure_max_height() {
    // 호출부 경로: layout 캐시 키는 항상 MEASURE_MAX_HEIGHT(1M) 박스이고,
    // hit-test 조회 키도 같은 값을 써야 한다. bbox.max_height(bitmap 상한)를
    // 조회 키에 넣으면 저장 키(1M)와 어긋나 매 paint miss가 되는 회귀를 고정한다.
    let base = style();
    let layout = LayoutKeyRef::from_style("text", &base, 100.0, 1_000_000.0).to_owned();
    let key = HitTestKeyRef::from_style("text", &base, 5.0, 7.0, 100.0, 1_000_000.0, 3.0)
        .to_owned_reusing_layout(&layout);

    assert!(
        HitTestKeyRef::from_style("text", &base, 5.0, 7.0, 100.0, 1_000_000.0, 3.0).matches(&key)
    );
    assert!(!HitTestKeyRef::from_style("text", &base, 5.0, 7.0, 100.0, 200.0, 3.0).matches(&key));
}
