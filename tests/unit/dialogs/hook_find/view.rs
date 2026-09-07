use super::*;

#[test]
fn preview_flattens_and_truncates_text() {
    assert_eq!(one_line_preview("ab\ncd", 10), "ab cd");
    assert_eq!(one_line_preview("abcdef", 4), "abcd…");
}

#[test]
fn history_trim_keeps_latest_unicode_characters() {
    let mut text = "가나다라마바사".to_string();
    trim_history(&mut text, 3);
    assert_eq!(text, "마바사");
}

#[test]
fn stream_update_keeps_history_across_view_recreation() {
    let source = HookSource {
        address: 1,
        context: 2,
        subcontext: 3,
    };
    let mut streams = Vec::new();

    assert_eq!(
        update_stream(
            &mut streams,
            HookText {
                source,
                hook_name: "first".into(),
                text: "한 줄".into(),
                full_string: false,
            },
        ),
        (0, StreamLabel::Added)
    );
    assert_eq!(
        update_stream(
            &mut streams,
            HookText {
                source,
                hook_name: "updated".into(),
                text: "다음\n줄".into(),
                full_string: false,
            },
        ),
        // 이름이 바뀌었으니 목록 항목도 갈아 끼워야 한다.
        (0, StreamLabel::Changed)
    );
    assert_eq!(streams[0].hook_name, "updated");
    assert_eq!(streams[0].history, "한 줄\r\n다음\r\n줄");
}

/// 저장된 후크 코드를 자동 설치하면 같은 ThreadParam의 이름이 `UserUI`로
/// 덮인다. 그때만 항목을 갈아 끼우고, 이름이 그대로면 건드리지 않는다.
#[test]
fn only_a_renamed_stream_asks_for_a_new_label() {
    let source = HookSource {
        address: 0x2ee_cf0,
        context: 0,
        subcontext: 0,
    };
    let line = |name: &str| HookText {
        source,
        hook_name: name.into(),
        text: "대사".into(),
        full_string: true,
    };
    let mut streams = Vec::new();

    assert_eq!(
        update_stream(&mut streams, line("EmbedSiglus")),
        (0, StreamLabel::Added)
    );
    assert_eq!(
        update_stream(&mut streams, line("EmbedSiglus")),
        (0, StreamLabel::Unchanged)
    );
    assert_eq!(
        update_stream(&mut streams, line("UserUI")),
        (0, StreamLabel::Changed)
    );
    assert_eq!(stream_label(&streams[0]), "UserUI [2EECF0:0:0]");
}

/// 스트림은 나타난 순서대로 뒤에만 붙는다. 목록 control의 항목 인덱스가
/// `streams`의 인덱스와 계속 맞아야 선택이 엉뚱한 스레드를 가리키지 않는다.
#[test]
fn new_streams_are_appended_in_arrival_order() {
    let line = |address: u64, name: &str| HookText {
        source: HookSource {
            address,
            context: 0,
            subcontext: 0,
        },
        hook_name: name.into(),
        text: "대사".into(),
        full_string: false,
    };
    let mut streams = Vec::new();

    assert_eq!(
        update_stream(&mut streams, line(1, "SiglusEngine3")),
        (0, StreamLabel::Added)
    );
    assert_eq!(
        update_stream(&mut streams, line(2, "EmbedSiglus")),
        (1, StreamLabel::Added)
    );
    assert_eq!(
        update_stream(&mut streams, line(3, "UserUI")),
        (2, StreamLabel::Added)
    );
    // 이미 있는 스레드는 자리를 옮기지 않는다.
    assert_eq!(
        update_stream(&mut streams, line(1, "SiglusEngine3")),
        (0, StreamLabel::Unchanged)
    );

    assert_eq!(
        streams.iter().map(|s| s.source.address).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(stream_label(&streams[1]), "EmbedSiglus [2:0:0]");
}

#[test]
fn indexed_streams_are_bounded() {
    let mut streams = Vec::new();
    let mut indices = std::collections::HashMap::new();
    for address in 0..MAX_STREAMS as u64 {
        let text = HookText {
            source: HookSource {
                address,
                context: 0,
                subcontext: 0,
            },
            hook_name: "test".into(),
            text: "x".into(),
            full_string: false,
        };
        assert!(update_stream_indexed(&mut streams, &mut indices, text).is_some());
    }
    assert!(
        update_stream_indexed(
            &mut streams,
            &mut indices,
            HookText {
                source: HookSource {
                    address: MAX_STREAMS as u64,
                    context: 0,
                    subcontext: 0,
                },
                hook_name: "overflow".into(),
                text: "x".into(),
                full_string: false,
            },
        )
        .is_none()
    );
    assert_eq!(streams.len(), MAX_STREAMS);
}

#[test]
fn append_fit_accounts_for_history_separator() {
    assert!(append_fits(MAX_HISTORY_CHARS - 3, "x"));
    assert!(!append_fits(MAX_HISTORY_CHARS - 2, "x"));
}
