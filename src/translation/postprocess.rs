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
        // 빈 패턴은 위에서 걸러냈으므로 automaton 구성은 항상 성공하지만,
        // 프로덕션 코드에 expect를 남기지 않기 위해 실패 시에도 패닉 대신
        // 후처리를 건너뛰도록(`None`) 처리한다.
        let automaton = match AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .build(&patterns)
        {
            Ok(automaton) => automaton,
            Err(error) => {
                tracing::warn!("EzTrans 후처리 automaton 구성 실패: {error}");
                return None;
            }
        };
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

/// 사전을 매번 다시 빌드해 한 줄만 치환하는 편의 함수. 실제 앱 경로는
/// [`EzTransPostprocessMatcher`]를 작업 준비 시 한 번만 빌드하므로, 이 함수는
/// 단위 테스트와 재빌드 비용을 재는 벤치(`benchmark/postprocess_bench.rs`)에서만 쓴다.
#[cfg(any(test, feature = "benchmark"))]
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
#[path = "../../tests/unit/translation/postprocess.rs"]
mod tests;
