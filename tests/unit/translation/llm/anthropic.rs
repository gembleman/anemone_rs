use super::*;

#[test]
fn rejects_partial_text_when_max_tokens_reached() {
    let json = r#"{
        "content": [{"type":"text","text":"잘린 번역"}],
        "stop_reason": "max_tokens"
    }"#;
    assert!(matches!(
        parse_messages_response(json),
        Err(TranslationError::OutputTruncated {
            provider: "Anthropic",
            ..
        })
    ));
}

#[test]
fn accepts_naturally_completed_text() {
    let json = r#"{
        "content": [{"type":"text","text":"완료된 번역"}],
        "stop_reason": "end_turn"
    }"#;
    assert_eq!(parse_messages_response(json).unwrap(), "완료된 번역");
}
