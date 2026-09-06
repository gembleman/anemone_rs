use super::*;

#[test]
fn receive_timestamps_split_events_at_the_window_boundary() {
    let mut merger = TextMerger::new(100);
    let source = HookSource {
        address: 1,
        context: 2,
        subcontext: 0,
    };
    let first_at = Instant::now();
    let line = |text: &str| HookText {
        source,
        hook_name: "test".into(),
        text: text.into(),
        full_string: true,
    };

    assert!(merger.submit_at(line("첫째"), first_at).is_empty());
    let flushed = merger.submit_at(line("둘째"), first_at + Duration::from_millis(100));

    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].text, "첫째");
    assert_eq!(merger.drain_all()[0].text, "둘째");
}

#[test]
fn merger_collapses_same_source_within_window() {
    let mut merger = TextMerger::new(10);
    merger.submit(HookText {
        source: HookSource {
            address: 1,
            context: 2,
            subcontext: 0,
        },
        hook_name: "test".into(),
        text: "안녕".into(),
        full_string: true,
    });
    merger.submit(HookText {
        source: HookSource {
            address: 1,
            context: 2,
            subcontext: 0,
        },
        hook_name: "test".into(),
        text: "하세요".into(),
        full_string: true,
    });
    std::thread::sleep(std::time::Duration::from_millis(15));
    let flushed = merger.flush_expired();
    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].text, "안녕\n하세요");
    // flush 뒤에는 비어 있다.
    assert!(merger.drain_all().is_empty());
}

#[test]
fn merger_keeps_distinct_sources_apart() {
    let mut merger = TextMerger::new(5);
    merger.submit(HookText {
        source: HookSource {
            address: 1,
            context: 2,
            subcontext: 0,
        },
        hook_name: "first".into(),
        text: "a".into(),
        full_string: false,
    });
    merger.submit(HookText {
        source: HookSource {
            address: 3,
            context: 4,
            subcontext: 0,
        },
        hook_name: "second".into(),
        text: "b".into(),
        full_string: false,
    });
    std::thread::sleep(std::time::Duration::from_millis(8));
    let mut texts: Vec<String> = merger
        .flush_expired()
        .into_iter()
        .map(|event| event.text)
        .collect();
    texts.sort();
    assert_eq!(texts, ["a".to_string(), "b".to_string()]);
}

#[test]
fn merger_reassembles_character_fragments() {
    let mut merger = TextMerger::new(0);
    let source = HookSource {
        address: 1,
        context: 2,
        subcontext: 0,
    };
    for text in ["가", "나", "다"] {
        merger.submit(HookText {
            source,
            hook_name: "test".into(),
            text: text.into(),
            full_string: false,
        });
    }

    let flushed = merger.flush_expired();
    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].text, "가나다");
}

/// 뒤 스냅샷이 앞 스냅샷보다 짧아도 앞 문장을 지우지 않는다. 창 안에 온 것은
/// 모두 실제로 화면에 나온 문장이다.
#[test]
fn merger_appends_shrinking_snapshot() {
    let mut merger = TextMerger::new(0);
    let source = HookSource {
        address: 1,
        context: 2,
        subcontext: 0,
    };
    for text in ["abcdef", "abc"] {
        merger.submit(HookText {
            source,
            hook_name: "test".into(),
            text: text.into(),
            full_string: true,
        });
    }

    let flushed = merger.flush_expired();
    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].text, "abcdef\nabc");
}

#[test]
fn merger_appends_unrelated_shrinking_snapshot() {
    let mut merger = TextMerger::new(0);
    let source = HookSource {
        address: 1,
        context: 2,
        subcontext: 0,
    };
    for text in ["공격하기", "아이템"] {
        merger.submit(HookText {
            source,
            hook_name: "test".into(),
            text: text.into(),
            full_string: true,
        });
    }

    let flushed = merger.flush_expired();
    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].text, "공격하기\n아이템");
}

/// 한 글자만 낸 FULL_STRING 이벤트에는 줄바꿈을 붙이지 않는다. 원본 `Push`가
/// 두 글자 이상일 때만 붙인다.
#[test]
fn merger_does_not_break_single_char_full_string() {
    let mut merger = TextMerger::new(0);
    let source = HookSource {
        address: 1,
        context: 2,
        subcontext: 0,
    };
    for text in ["가", "나"] {
        merger.submit(HookText {
            source,
            hook_name: "test".into(),
            text: text.into(),
            full_string: true,
        });
    }

    let flushed = merger.flush_expired();
    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].text, "가나");
}

#[test]
fn merger_keeps_repeated_fragments() {
    let mut merger = TextMerger::new(0);
    let source = HookSource {
        address: 1,
        context: 2,
        subcontext: 0,
    };
    for _ in 0..2 {
        merger.submit(HookText {
            source,
            hook_name: "test".into(),
            text: "!".into(),
            full_string: false,
        });
    }

    let flushed = merger.flush_expired();
    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].text, "!!");
}

#[test]
fn merger_keeps_short_following_fragment() {
    let mut merger = TextMerger::new(0);
    let source = HookSource {
        address: 1,
        context: 2,
        subcontext: 0,
    };
    for text in ["Hello", "x"] {
        merger.submit(HookText {
            source,
            hook_name: "test".into(),
            text: text.into(),
            full_string: false,
        });
    }

    let flushed = merger.flush_expired();
    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].text, "Hellox");
}

#[test]
fn merger_preserves_whitespace_fragments() {
    let mut merger = TextMerger::new(0);
    let source = HookSource {
        address: 1,
        context: 2,
        subcontext: 0,
    };
    for text in ["Hello", " ", "world"] {
        merger.submit(HookText {
            source,
            hook_name: "test".into(),
            text: text.into(),
            full_string: false,
        });
    }

    let flushed = merger.flush_expired();
    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].text, "Hello world");
}

#[test]
fn merger_trims_only_after_fragments_are_assembled() {
    let mut merger = TextMerger::new(0);
    let source = HookSource {
        address: 1,
        context: 2,
        subcontext: 0,
    };
    for text in [" Hello", " ", "world "] {
        merger.submit(HookText {
            source,
            hook_name: "test".into(),
            text: text.into(),
            full_string: false,
        });
    }

    let flushed = merger.flush_expired();
    assert_eq!(flushed[0].text, "Hello world");
}

/// 쉬지 않고 조각을 뱉는 출처는 창이 지나지 않는다. 그런 출처가 끝없이
/// 쌓이지 않게 길이로도 끊는다.
#[test]
fn an_overlong_buffer_does_not_wait_for_the_window() {
    let mut merger = TextMerger::new(60_000);
    let source = HookSource {
        address: 1,
        context: 2,
        subcontext: 0,
    };
    for _ in 0..=MAX_BUFFER_CHARS {
        merger.submit(HookText {
            source,
            hook_name: "test".into(),
            text: "가".into(),
            full_string: false,
        });
    }

    let flushed = merger.flush_expired();
    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].text.chars().count(), MAX_BUFFER_CHARS + 1);
}

/// 문장을 통째로 내는 훅도 창을 기다린다. 원본 `TextThread::Flush`는 훅 종류를
/// 보지 않고 `lastPushTime` 하나로만 방출을 결정한다.
#[test]
fn a_full_string_event_waits_for_the_window() {
    let mut merger = TextMerger::new(60_000);
    let source = HookSource {
        address: 1,
        context: 2,
        subcontext: 0,
    };
    merger.submit(HookText {
        source,
        hook_name: "Krkr2wcs".into(),
        text: "「お早う、我が花畑学園の生徒諸君！".into(),
        full_string: true,
    });

    assert!(merger.flush_expired().is_empty());

    // 창 안에 도착한 다음 문장은 줄바꿈으로 이어져 한 번에 나간다.
    merger.submit(HookText {
        source,
        hook_name: "Krkr2wcs".into(),
        text: "今日も良い天気だな！」".into(),
        full_string: true,
    });
    assert!(merger.flush_expired().is_empty());

    let flushed = merger.drain_all();
    assert_eq!(flushed.len(), 1);
    assert_eq!(
        flushed[0].text,
        "「お早う、我が花畑学園の生徒諸君！\n今日も良い天気だな！」"
    );
}

/// EmbedSiglus도 원본 TextThread처럼 마지막 조각 뒤의 시간 창으로만 끊는다.
/// 한번 방출된 버퍼는 제거되므로 다음 문장에 다시 붙지 않는다.
#[test]
fn embed_siglus_uses_the_same_time_window_without_accumulating() {
    let mut merger = TextMerger::new(10);
    let source = HookSource {
        address: 1,
        context: 2,
        subcontext: 0,
    };
    let line = |text: &str| HookText {
        source,
        hook_name: "EmbedSiglus".into(),
        text: text.into(),
        full_string: false,
    };

    merger.submit(line("첫 문장"));
    std::thread::sleep(std::time::Duration::from_millis(15));
    let first = merger.flush_expired();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].text, "첫 문장");

    merger.submit(line("둘째 문장"));
    std::thread::sleep(std::time::Duration::from_millis(15));
    let second = merger.flush_expired();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].text, "둘째 문장");
}
