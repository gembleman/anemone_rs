use super::*;

#[test]
fn serializes_responses_request_with_the_api_shape() {
    let params = LlmCallParams {
        provider: LlmProvider::OpenAi,
        model: "model".into(),
        api_key: "key".into(),
        base_url: String::new(),
        system_prompt: String::new(),
        temperature: 0.25,
        max_tokens: 123,
        reasoning_effort: None,
        glossary: Vec::new(),
    };
    let value =
        serde_json::to_value(responses_request_payload(&params, "system", "source")).unwrap();
    assert_eq!(value["model"], "model");
    assert_eq!(value["instructions"], "system");
    assert_eq!(value["input"], "source");
    assert_eq!(value["max_output_tokens"], 123);
    assert_eq!(value["store"], false);
    assert_eq!(value["temperature"], 0.25);
    assert!(value.get("messages").is_none());
    assert!(value.get("max_completion_tokens").is_none());
}

#[test]
fn serializes_chat_request_for_compatible_providers() {
    let params = LlmCallParams {
        provider: LlmProvider::OpenRouter,
        model: "model".into(),
        api_key: "key".into(),
        base_url: String::new(),
        system_prompt: String::new(),
        temperature: 0.25,
        max_tokens: 123,
        reasoning_effort: None,
        glossary: Vec::new(),
    };
    let value = serde_json::to_value(chat_request_payload(&params, "system", "source")).unwrap();
    assert_eq!(value["messages"][0]["content"], "system");
    assert_eq!(value["messages"][1]["content"], "source");
    assert_eq!(value["max_tokens"], 123);
    assert_eq!(value["temperature"], 0.25);
    assert!(value.get("max_output_tokens").is_none());
}

#[test]
fn omits_temperature_for_openai_reasoning_models() {
    for model in ["gpt-5.6", "o3", "o4-mini"] {
        let params = LlmCallParams {
            provider: LlmProvider::OpenAi,
            model: model.into(),
            api_key: "key".into(),
            base_url: String::new(),
            system_prompt: String::new(),
            temperature: 0.25,
            max_tokens: 123,
            reasoning_effort: None,
            glossary: Vec::new(),
        };
        let value =
            serde_json::to_value(responses_request_payload(&params, "system", "source")).unwrap();
        assert!(value.get("temperature").is_none(), "model: {model}");
    }
}

#[test]
fn serializes_reasoning_effort_only_for_openai_reasoning_models() {
    let mut params = LlmCallParams {
        provider: LlmProvider::OpenAi,
        model: "gpt-5.6".into(),
        api_key: "key".into(),
        base_url: String::new(),
        system_prompt: String::new(),
        temperature: 0.25,
        max_tokens: 123,
        reasoning_effort: Some(ReasoningEffort::Xhigh),
        glossary: Vec::new(),
    };

    let value =
        serde_json::to_value(responses_request_payload(&params, "system", "source")).unwrap();
    assert_eq!(value["reasoning"]["effort"], "xhigh");
    assert!(value.get("reasoning_effort").is_none());

    params.model = "gpt-4o".into();
    let value =
        serde_json::to_value(responses_request_payload(&params, "system", "source")).unwrap();
    assert!(value.get("reasoning").is_none());

    params.provider = LlmProvider::OpenRouter;
    params.model = "openai/gpt-5.6".into();
    let value = serde_json::to_value(chat_request_payload(&params, "system", "source")).unwrap();
    assert!(value.get("reasoning").is_none());
    assert!(value.get("reasoning_effort").is_none());
}

#[test]
fn preserves_none_reasoning_for_the_blank_nano_default() {
    let params = LlmCallParams {
        provider: LlmProvider::OpenAi,
        model: String::new(),
        api_key: "key".into(),
        base_url: String::new(),
        system_prompt: String::new(),
        temperature: 0.25,
        max_tokens: 123,
        reasoning_effort: None,
        glossary: Vec::new(),
    };

    let value =
        serde_json::to_value(responses_request_payload(&params, "system", "source")).unwrap();
    assert_eq!(value["model"], "gpt-5.4-nano");
    assert_eq!(value["reasoning"]["effort"], "none");
}

#[test]
fn parses_all_responses_output_text_items() {
    let json = r#"{
        "status": "completed",
        "output": [
            { "type": "reasoning", "summary": [] },
            {
                "type": "message",
                "content": [
                    { "type": "output_text", "text": "안" },
                    { "type": "output_text", "text": "녕" }
                ]
            },
            {
                "type": "message",
                "content": [
                    { "type": "output_text", "text": "하세요" }
                ]
            }
        ]
    }"#;
    assert_eq!(parse_response(json).unwrap(), "안녕하세요");
}

#[test]
fn flags_incomplete_responses_output() {
    let json = r#"{
        "status": "incomplete",
        "incomplete_details": { "reason": "max_output_tokens" },
        "output": [{
            "type": "message",
            "content": [{ "type": "output_text", "text": "절단된 부분" }]
        }]
    }"#;
    assert!(matches!(
        parse_response(json),
        Err(TranslationError::OutputTruncated {
            provider: "OpenAI API",
            ..
        })
    ));
}

#[test]
fn flags_responses_refusal() {
    let json = r#"{
        "status": "completed",
        "output": [{
            "type": "message",
            "content": [{ "type": "refusal", "refusal": "정책상 거부" }]
        }]
    }"#;
    match parse_response(json) {
        Err(TranslationError::Api {
            code: 0, message, ..
        }) => assert!(message.contains("거부"), "message: {message}"),
        other => panic!("expected Api error for refusal, got {other:?}"),
    }
}

#[test]
fn parses_plain_string_content() {
    let json = r#"{
        "choices": [{
            "message": { "role": "assistant", "content": "안녕하세요" },
            "finish_reason": "stop"
        }]
    }"#;
    assert_eq!(parse_chat_completion(json).unwrap(), "안녕하세요");
}

#[test]
fn parses_array_content_parts() {
    let json = r#"{
        "choices": [{
            "message": {
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "안" },
                    { "type": "text", "text": "녕" }
                ]
            },
            "finish_reason": "stop"
        }]
    }"#;
    assert_eq!(parse_chat_completion(json).unwrap(), "안녕");
}

#[test]
fn flags_length_truncation_as_api_error() {
    let json = r#"{
        "choices": [{
            "message": { "content": "절단된 부분" },
            "finish_reason": "length"
        }]
    }"#;
    assert!(matches!(
        parse_chat_completion(json),
        Err(TranslationError::OutputTruncated {
            provider: "OpenAI 호환 API",
            ..
        })
    ));
}

#[test]
fn flags_refusal_when_content_null() {
    let json = r#"{
        "choices": [{
            "message": { "content": null, "refusal": "정책상 거부" },
            "finish_reason": "stop"
        }]
    }"#;
    match parse_chat_completion(json) {
        Err(TranslationError::Api {
            code: 0, message, ..
        }) => {
            assert!(message.contains("거부"), "message: {message}");
        }
        other => panic!("expected Api error for refusal, got {other:?}"),
    }
}

#[test]
fn maps_error_object_to_api_error() {
    let json = r#"{ "error": { "message": "Invalid key", "code": "401" } }"#;
    match parse_chat_completion(json) {
        Err(TranslationError::Api {
            code: 401, message, ..
        }) => {
            assert_eq!(message, "Invalid key");
        }
        other => panic!("expected Api(401), got {other:?}"),
    }
}
