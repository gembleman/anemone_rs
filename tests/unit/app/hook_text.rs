use std::collections::VecDeque;

use super::state::{MAX_QUEUED_HOOK_TEXTS, QueuedHookText};
use super::{queue_hook_text, take_queued_hook_text};

fn sentence(text: &str) -> QueuedHookText {
    QueuedHookText {
        text: text.to_string(),
        full_string: true,
    }
}

fn fragment(text: &str) -> QueuedHookText {
    QueuedHookText {
        text: text.to_string(),
        full_string: false,
    }
}

/// 앞 문장 번역을 기다리는 동안 쌓인 문장은 하나도 버리지 않고 한 요청으로
/// 나간다. 앞 요청을 취소시키는 별도 요청으로 보내면 앞 문장이 사라진다.
#[test]
fn queued_sentences_leave_together_in_order() {
    let mut queue = VecDeque::new();
    assert_eq!(
        queue_hook_text(&mut queue, sentence("「お早う、我が花畑学園の生徒諸君！")),
        None
    );
    assert_eq!(
        queue_hook_text(&mut queue, sentence("ふふふ……みんな、")),
        None
    );

    assert_eq!(
        take_queued_hook_text(&mut queue).as_deref(),
        Some("「お早う、我が花畑学園の生徒諸君！\nふふふ……みんな、")
    );
    assert!(queue.is_empty());
    assert_eq!(take_queued_hook_text(&mut queue), None);
}

/// 글자 단위 훅에서 온 조각은 한 문장이 병합 창을 넘겨 끊긴 것이다. 줄을
/// 나누지 않고 그대로 이어야 원래 문장이 된다.
#[test]
fn queued_fragments_rejoin_without_a_line_break() {
    let mut queue = VecDeque::new();
    queue_hook_text(&mut queue, fragment("「お早う、我が花畑学園の生徒諸君！ふ"));
    queue_hook_text(&mut queue, fragment("ふふ…"));
    queue_hook_text(&mut queue, fragment("…みんな、"));

    assert_eq!(
        take_queued_hook_text(&mut queue).as_deref(),
        Some("「お早う、我が花畑学園の生徒諸君！ふふふ……みんな、")
    );
}

/// 번역이 문장 속도를 못 따라가도 대기열은 한계를 넘지 않는다.
#[test]
fn an_overfull_queue_drops_its_oldest_sentence() {
    let mut queue = VecDeque::new();
    for index in 0..MAX_QUEUED_HOOK_TEXTS {
        assert_eq!(
            queue_hook_text(&mut queue, sentence(&index.to_string())),
            None
        );
    }

    assert_eq!(
        queue_hook_text(&mut queue, sentence("overflow")),
        Some(sentence("0"))
    );
    assert_eq!(queue.len(), MAX_QUEUED_HOOK_TEXTS);
    let joined = take_queued_hook_text(&mut queue).expect("대기열이 비어 있지 않다");
    assert!(joined.starts_with("1\n2\n"));
    assert!(joined.ends_with("\noverflow"));
}
