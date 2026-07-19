use super::*;

#[test]
fn serializes_camel_case_generation_config_and_borrowed_parts() {
    let params = LlmCallParams {
        provider: super::super::LlmProvider::Gemini,
        model: "model".into(),
        api_key: "key".into(),
        base_url: String::new(),
        system_prompt: String::new(),
        temperature: 0.75,
        max_tokens: 456,
        reasoning_effort: None,
        glossary: Vec::new(),
    };
    let value = serde_json::to_value(request_payload(&params, "system", "source")).unwrap();
    assert_eq!(value["system_instruction"]["parts"][0]["text"], "system");
    assert_eq!(value["contents"][0]["parts"][0]["text"], "source");
    assert_eq!(value["generationConfig"]["maxOutputTokens"], 456);
}

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
