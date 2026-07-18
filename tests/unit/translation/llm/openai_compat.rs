use super::*;

#[test]
fn serializes_borrowed_request_with_the_api_shape() {
    let params = LlmCallParams {
        provider: LlmProvider::OpenAi,
        model: "model".into(),
        api_key: "key".into(),
        base_url: String::new(),
        system_prompt: String::new(),
        temperature: 0.25,
        max_tokens: 123,
        glossary: Vec::new(),
    };
    let value = serde_json::to_value(request_payload(&params, "system", "source")).unwrap();
    assert_eq!(value["model"], "model");
    assert_eq!(value["messages"][0]["content"], "system");
    assert_eq!(value["messages"][1]["content"], "source");
    assert_eq!(value["max_tokens"], 123);
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
