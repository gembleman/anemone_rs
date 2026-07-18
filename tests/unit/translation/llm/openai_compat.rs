use super::*;

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
    match parse_chat_completion(json) {
        Err(TranslationError::Api { code: 0, message }) => {
            assert!(message.contains("max_tokens"), "message: {message}");
        }
        other => panic!("expected Api error for length, got {other:?}"),
    }
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
        Err(TranslationError::Api { code: 0, message }) => {
            assert!(message.contains("거부"), "message: {message}");
        }
        other => panic!("expected Api error for refusal, got {other:?}"),
    }
}

#[test]
fn maps_error_object_to_api_error() {
    let json = r#"{ "error": { "message": "Invalid key", "code": "401" } }"#;
    match parse_chat_completion(json) {
        Err(TranslationError::Api { code: 401, message }) => {
            assert_eq!(message, "Invalid key");
        }
        other => panic!("expected Api(401), got {other:?}"),
    }
}
