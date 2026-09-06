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
        top_p: 1.0,
        frequency_penalty: 0.0,
        presence_penalty: 0.0,
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
        top_p: 0.8,
        frequency_penalty: 0.4,
        presence_penalty: -0.2,
        max_tokens: 123,
        reasoning_effort: None,
        glossary: Vec::new(),
    };
    let value = serde_json::to_value(chat_request_payload(&params, "system", "source")).unwrap();
    assert_eq!(value["messages"][0]["content"], "system");
    assert_eq!(value["messages"][1]["content"], "source");
    assert_eq!(value["max_tokens"], 123);
    assert_eq!(value["temperature"], 0.25);
    assert!((value["top_p"].as_f64().unwrap() - 0.8).abs() < 1e-6);
    assert!((value["frequency_penalty"].as_f64().unwrap() - 0.4).abs() < 1e-6);
    assert!((value["presence_penalty"].as_f64().unwrap() + 0.2).abs() < 1e-6);
    assert!(value.get("max_output_tokens").is_none());
}

#[test]
fn omits_openrouter_sampling_options_for_grok() {
    let params = LlmCallParams {
        provider: LlmProvider::Grok,
        model: "model".into(),
        api_key: "key".into(),
        base_url: String::new(),
        system_prompt: String::new(),
        temperature: 0.25,
        top_p: 0.8,
        frequency_penalty: 0.4,
        presence_penalty: -0.2,
        max_tokens: 123,
        reasoning_effort: None,
        glossary: Vec::new(),
    };
    let value = serde_json::to_value(chat_request_payload(&params, "system", "source")).unwrap();
    assert!(value.get("top_p").is_none());
    assert!(value.get("frequency_penalty").is_none());
    assert!(value.get("presence_penalty").is_none());
}

#[tokio::test]
async fn rejects_openrouter_without_an_explicit_model() {
    let params = LlmCallParams {
        provider: LlmProvider::OpenRouter,
        model: "  ".into(),
        api_key: "key".into(),
        base_url: String::new(),
        system_prompt: String::new(),
        temperature: 0.25,
        top_p: 1.0,
        frequency_penalty: 0.0,
        presence_penalty: 0.0,
        max_tokens: 123,
        reasoning_effort: None,
        glossary: Vec::new(),
    };
    let result = translate_async_with_client(
        &crate::translation::http_common::create_client(),
        "source",
        Language::Eng,
        Language::Kor,
        &params,
    )
    .await;
    assert!(matches!(result, Err(TranslationError::MissingModel)));
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
            top_p: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
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
        top_p: 1.0,
        frequency_penalty: 0.0,
        presence_penalty: 0.0,
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
        top_p: 1.0,
        frequency_penalty: 0.0,
        presence_penalty: 0.0,
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

#[test]
fn flags_a_non_stop_finish_reason_when_no_text_or_error_is_present() {
    let json = r#"{
        "choices": [{
            "message": { "content": null },
            "finish_reason": "content_filter"
        }]
    }"#;
    match parse_chat_completion(json) {
        Err(TranslationError::Api {
            code: 0, message, ..
        }) => {
            assert!(message.contains("content_filter"), "message: {message}");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[test]
fn a_response_with_no_text_error_or_finish_reason_is_a_parse_error() {
    assert!(matches!(
        parse_chat_completion(r#"{"choices":[]}"#),
        Err(TranslationError::Parse(_))
    ));
}

#[test]
fn maps_a_numeric_error_code_to_api_error() {
    let json = r#"{"error":{"message":"거부됨","code":403}}"#;
    match parse_response(json) {
        Err(TranslationError::Api {
            code: 403, message, ..
        }) => {
            assert_eq!(message, "거부됨");
        }
        other => panic!("expected Api(403), got {other:?}"),
    }
}

#[test]
fn a_responses_output_without_text_is_a_parse_error() {
    let json = r#"{"status":"completed","output":[{"type":"reasoning","summary":[]}]}"#;
    assert!(matches!(
        parse_response(json),
        Err(TranslationError::Parse(_))
    ));
}

#[tokio::test]
async fn anthropic_and_gemini_are_rejected_by_the_openai_compatible_backend() {
    for provider in [LlmProvider::Anthropic, LlmProvider::Gemini] {
        let params = LlmCallParams {
            provider,
            model: "model".into(),
            api_key: "key".into(),
            base_url: String::new(),
            system_prompt: String::new(),
            temperature: 0.25,
            top_p: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            max_tokens: 123,
            reasoning_effort: None,
            glossary: Vec::new(),
        };
        let result = translate_async_with_client(
            &crate::translation::http_common::create_client(),
            "source",
            Language::Eng,
            Language::Kor,
            &params,
        )
        .await;
        assert!(
            matches!(result, Err(TranslationError::Engine(_))),
            "provider={provider:?}"
        );
    }
}

#[tokio::test]
async fn rejects_a_missing_api_key_before_any_network_call() {
    let params = LlmCallParams {
        provider: LlmProvider::OpenAi,
        model: "model".into(),
        api_key: String::new(),
        base_url: String::new(),
        system_prompt: String::new(),
        temperature: 0.25,
        top_p: 1.0,
        frequency_penalty: 0.0,
        presence_penalty: 0.0,
        max_tokens: 123,
        reasoning_effort: None,
        glossary: Vec::new(),
    };
    let result = translate_async_with_client(
        &crate::translation::http_common::create_client(),
        "source",
        Language::Eng,
        Language::Kor,
        &params,
    )
    .await;
    assert!(matches!(result, Err(TranslationError::MissingApiKey)));
}

#[tokio::test]
async fn grok_and_openrouter_post_chat_completions_and_bear_the_api_key() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0_u8; 2048];
        loop {
            let count = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..count]);
            if request.windows(4).any(|part| part == b"\r\n\r\n") {
                break;
            }
        }
        request_tx
            .send(String::from_utf8_lossy(&request).into_owned())
            .unwrap();
        let body = r#"{"choices":[{"message":{"content":"grok 번역"},"finish_reason":"stop"}]}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    let params = LlmCallParams {
        provider: LlmProvider::Grok,
        model: "grok-4.3".into(),
        api_key: "grok-key".into(),
        base_url: format!("http://{address}"),
        system_prompt: String::new(),
        temperature: 0.25,
        top_p: 1.0,
        frequency_penalty: 0.0,
        presence_penalty: 0.0,
        max_tokens: 64,
        reasoning_effort: None,
        glossary: Vec::new(),
    };
    let translated = translate_async_with_client(
        &crate::translation::http_common::create_client(),
        "hello",
        Language::Eng,
        Language::Kor,
        &params,
    )
    .await
    .unwrap();
    assert_eq!(translated, "grok 번역");

    let request = request_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    let request = request.to_ascii_lowercase();
    assert!(request.starts_with("post /chat/completions"));
    assert!(request.contains("authorization: bearer grok-key"));
    server.join().unwrap();
}
