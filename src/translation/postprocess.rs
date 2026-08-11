use aho_corasick::{AhoCorasick, MatchKind};

use crate::config::EzTransPostprocessEntry;

#[cfg(feature = "benchmark")]
pub use self::apply_eztrans_dictionary as benchmark_apply_eztrans_dictionary;

/// EzTrans 번역 결과를 사전 항목으로 한 번만 치환하는 matcher.
///
/// Aho-Corasick automaton(`leftmost-longest`)을 작업 준비 시 한 번만 빌드한다.
/// 사전 크기에 무관하게 선형 시간 스캔을 유지한다. (벤치: 사전 20,000개에서
/// 호출마다 빌드하면 줄당 27.6ms, 빌드 1회로 미리하면 수십 us)
pub struct EzTransPostprocessMatcher {
    dictionary: Vec<EzTransPostprocessEntry>,
    automaton: AhoCorasick,
    /// automaton 패턴 인덱스 → 사전 인덱스 (빈 source 항목은 패턴에서 제외됨).
    pattern_to_entry: Vec<usize>,
}

impl EzTransPostprocessMatcher {
    /// 사전에서 automaton을 빌드한다. 번역 대상이 없으면(전부 빈 source) `None`.
    pub fn new(dictionary: &[EzTransPostprocessEntry]) -> Option<Self> {
        let mut pattern_to_entry = Vec::new();
        let patterns = dictionary
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.source.is_empty())
            .map(|(index, entry)| {
                pattern_to_entry.push(index);
                entry.source.as_str()
            })
            .collect::<Vec<_>>();
        if patterns.is_empty() {
            return None;
        }
        let automaton = AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .build(&patterns)
            .expect("빈 패턴은 제외했으므로 automaton 구성은 항상 성공한다");
        Some(Self {
            dictionary: dictionary.to_vec(),
            automaton,
            pattern_to_entry,
        })
    }

    /// 번역 결과를 사전 항목으로 한 번만 치환한다. 같은 위치에서 여러 원문이
    /// 겹치면 더 긴 항목을 우선하며, 치환한 결과는 다시 사전 입력으로 쓰지
    /// 않는다 (`leftmost-longest`가 최소 offset → 최장 매치 → 사전 순서의
    /// 기존 의미론과 동일).
    pub fn apply(&self, translated: String) -> String {
        if translated.is_empty() {
            return translated;
        }

        let mut output = String::with_capacity(translated.len());
        let mut cursor = 0;

        while cursor < translated.len() {
            let remaining = &translated[cursor..];
            let Some(matched) = self.automaton.find(remaining) else {
                output.push_str(remaining);
                break;
            };
            let entry = &self.dictionary[self.pattern_to_entry[matched.pattern().as_usize()]];
            output.push_str(&remaining[..matched.start()]);
            output.push_str(&entry.target);
            cursor += matched.end();
        }

        output
    }
}

/// 벤치·호출부가 사전을 한 번만 주고 매치를 반복할 때 쓰는 편의 함수.
/// 실제 앱 경로는 [`EzTransPostprocessMatcher`]를 작업 준비 시 빌드한다.
pub fn apply_eztrans_dictionary(
    translated: String,
    dictionary: &[EzTransPostprocessEntry],
) -> String {
    match EzTransPostprocessMatcher::new(dictionary) {
        Some(matcher) => matcher.apply(translated),
        None => translated,
    }
}

#[cfg(test)]
mod tests {
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
        let dictionary = [
            entry("알파", "A"),
            entry("", "빈 항목"),
            entry("베타", "B"),
        ];
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
}
