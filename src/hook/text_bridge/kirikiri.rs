//! KiriKiri 계열 엔진 전용 후처리 — 중복 화자 표기 축약 + 화자/대사 줄바꿈.

pub(crate) fn is_kirikiri_engine(engine_name: &str) -> bool {
    matches!(
        engine_name.to_ascii_lowercase().as_str(),
        "kirikiri" | "kirikiri1" | "kirikiri2" | "kirikiriz" | "krkrz64" | "krkr2wcs"
    )
}

pub(crate) fn postprocess_hook_text(engine_name: &str, text: &mut String) {
    if is_kirikiri_engine(engine_name) {
        collapse_duplicate_speaker_prefix(text);
        break_after_speaker_prefix(text);
    }
}

/// 문장 맨 앞 `【화자】` 표기가 끝나는 바이트 오프셋. 이름이 비면 화자로 보지 않는다.
fn speaker_prefix_end(text: &str) -> Option<usize> {
    let name_len = text.strip_prefix('【')?.find('】')?;
    if name_len == 0 {
        return None;
    }
    Some('【'.len_utf8() + name_len + '】'.len_utf8())
}

/// 문장 맨 앞에서 동일한 `【화자】` 표기가 연속되면 하나만 남긴다.
fn collapse_duplicate_speaker_prefix(text: &mut String) {
    let Some(prefix_end) = speaker_prefix_end(text) else {
        return;
    };

    let prefix = &text[..prefix_end];
    let mut remainder = &text[prefix_end..];
    while let Some(next) = remainder.strip_prefix(prefix) {
        remainder = next;
    }
    let remove_len = text.len() - (prefix.len() + remainder.len());
    if remove_len > 0 {
        text.drain(..remove_len);
    }
}

/// 문장 맨 앞 `【화자】` 뒤에서 줄을 바꿔 화자와 대사를 분리해 보여 준다.
///
/// 화자와 대사 사이의 기존 공백/개행은 개행 하나로 정규화하고, 화자 표기만
/// 있는 문장은 빈 줄이 생기지 않도록 그대로 둔다.
fn break_after_speaker_prefix(text: &mut String) {
    let Some(prefix_end) = speaker_prefix_end(text) else {
        return;
    };
    let remainder = &text[prefix_end..];
    let body_start = prefix_end + (remainder.len() - remainder.trim_start().len());
    if body_start == text.len() {
        return;
    }
    text.replace_range(prefix_end..body_start, "\n");
}

#[cfg(test)]
#[path = "../../../tests/unit/hook/text_bridge/kirikiri.rs"]
mod tests;
