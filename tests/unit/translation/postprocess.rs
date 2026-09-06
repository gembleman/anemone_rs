use super::*;

fn entry(source: &str, target: &str) -> EzTransPostprocessEntry {
    EzTransPostprocessEntry {
        source: source.into(),
        target: target.into(),
    }
}

#[test]
fn replaces_longest_match_without_cascading() {
    let dictionary = [
        entry("아네모네", "Anemone"),
        entry("아네모네 씨", "아네모네 님"),
        entry("Anemone", "잘못된 재치환"),
    ];

    assert_eq!(
        apply_eztrans_dictionary("아네모네 씨와 아네모네".into(), &dictionary),
        "아네모네 님와 Anemone"
    );
}

#[test]
fn ignores_empty_sources_and_allows_deletion() {
    let dictionary = [entry("", "무시"), entry("삭제", "")];
    assert_eq!(
        apply_eztrans_dictionary("문구 삭제".into(), &dictionary),
        "문구 "
    );
}

#[test]
fn empty_source_in_the_middle_does_not_shift_pattern_indexes() {
    // 빈 source 항목이 automaton 패턴에서 제외되므로, 패턴 인덱스와 사전
    // 인덱스 매핍(pattern_to_entry)이 어긋나면 엉뚱한 target이 들어간다.
    let dictionary = [entry("알파", "A"), entry("", "빈 항목"), entry("베타", "B")];
    assert_eq!(
        apply_eztrans_dictionary("알파 베타".into(), &dictionary),
        "A B"
    );
}

#[test]
fn prefers_longest_match_at_the_same_offset() {
    // 같은 시작 위치에서 겹치는 항목은 더 긴 쪽을 우선한다.
    let dictionary = [entry("テスト", "테스트"), entry("テスト文", "테스트문")];
    assert_eq!(
        apply_eztrans_dictionary("テスト文です".into(), &dictionary),
        "테스트문です"
    );
}

#[test]
fn later_offset_wins_over_longer_match() {
    // 최소 offset이 우선이다 — 더 긴 항목이 뒤에 있어도 먼저 나오는 항목을 쓴다.
    let dictionary = [entry("文", "문장"), entry("テスト文", "테스트문")];
    assert_eq!(
        apply_eztrans_dictionary("あテスト文".into(), &dictionary),
        "あ테스트문"
    );
}
