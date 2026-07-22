use crate::config::EzTransPostprocessEntry;

/// EzTrans 번역 결과를 사전 항목으로 한 번만 치환한다.
///
/// 같은 위치에서 여러 원문이 겹치면 더 긴 항목을 우선하며, 치환해서 생긴 문자열은
/// 다시 사전 입력으로 사용하지 않는다. 빈 원문 항목은 무시한다.
pub(crate) fn apply_eztrans_dictionary(
    translated: String,
    dictionary: &[EzTransPostprocessEntry],
) -> String {
    if translated.is_empty() || dictionary.iter().all(|entry| entry.source.is_empty()) {
        return translated;
    }

    let mut output = String::with_capacity(translated.len());
    let mut cursor = 0;

    while cursor < translated.len() {
        let remaining = &translated[cursor..];
        let next = dictionary
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.source.is_empty())
            .filter_map(|(index, entry)| {
                remaining
                    .find(&entry.source)
                    .map(|offset| (offset, std::cmp::Reverse(entry.source.len()), index, entry))
            })
            .min_by_key(|(offset, source_len, index, _)| (*offset, *source_len, *index));

        let Some((offset, _, _, entry)) = next else {
            output.push_str(remaining);
            break;
        };
        output.push_str(&remaining[..offset]);
        output.push_str(&entry.target);
        cursor += offset + entry.source.len();
    }

    output
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
}
