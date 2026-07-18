use super::*;

#[test]
fn rejects_partial_text_when_max_tokens_reached() {
    let json = r#"{
        "candidates": [{
            "content": {"parts": [{"text":"잘린 번역"}]},
            "finishReason": "MAX_TOKENS"
        }]
    }"#;
    assert!(matches!(
        parse_generate_content_response(json),
        Err(TranslationError::OutputTruncated {
            provider: "Gemini",
            ..
        })
    ));
}

#[test]
fn accepts_naturally_completed_text() {
    let json = r#"{
        "candidates": [{
            "content": {"parts": [{"text":"완료된 번역"}]},
            "finishReason": "STOP"
        }]
    }"#;
    assert_eq!(
        parse_generate_content_response(json).unwrap(),
        "완료된 번역"
    );
}
