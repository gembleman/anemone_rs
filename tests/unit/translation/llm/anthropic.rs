use super::*;

#[test]
fn serializes_cache_control_and_borrowed_messages() {
    let params = LlmCallParams {
        provider: super::super::LlmProvider::Anthropic,
        model: "model".into(),
        api_key: "key".into(),
        base_url: String::new(),
        system_prompt: String::new(),
        temperature: 0.5,
        top_p: 1.0,
        frequency_penalty: 0.0,
        presence_penalty: 0.0,
        max_tokens: 321,
        reasoning_effort: None,
        glossary: Vec::new(),
    };
    let value = serde_json::to_value(request_payload(&params, "system", "source")).unwrap();
    assert_eq!(value["system"][0]["type"], "text");
    assert_eq!(value["system"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(value["messages"][0]["content"], "source");
    assert_eq!(value["max_tokens"], 321);
    assert_eq!(value["temperature"], 0.5);
}

#[test]
fn omits_temperature_for_models_without_sampling_parameters() {
    for model in ["", "claude-opus-4-8", "claude-sonnet-5"] {
        let params = LlmCallParams {
            provider: super::super::LlmProvider::Anthropic,
            model: model.into(),
            api_key: "key".into(),
            base_url: String::new(),
            system_prompt: String::new(),
            temperature: 0.3,
            top_p: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            max_tokens: 321,
            reasoning_effort: None,
            glossary: Vec::new(),
        };

        let value = serde_json::to_value(request_payload(&params, "system", "source")).unwrap();
        assert!(
            value.get("temperature").is_none(),
            "model: {}",
            params.effective_model()
        );
    }
}

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
