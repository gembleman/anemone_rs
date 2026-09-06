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

#[test]
fn flags_a_stop_reason_that_is_neither_end_turn_nor_stop_sequence() {
    let json = r#"{
        "content": [{"type":"text","text":"일부 번역"}],
        "stop_reason": "refusal"
    }"#;
    match parse_messages_response(json) {
        Err(TranslationError::Api {
            code: 0, message, ..
        }) => {
            assert!(message.contains("refusal"), "message: {message}");
        }
        other => panic!("expected Api error for a non-terminal stop_reason, got {other:?}"),
    }
}

#[test]
fn maps_known_anthropic_error_types_to_http_style_codes() {
    for (kind, expected_code) in [
        ("authentication_error", 401),
        ("permission_error", 401),
        ("rate_limit_error", 429),
        ("overloaded_error", 529),
        ("api_error", 500),
        ("invalid_request_error", 0),
    ] {
        let json = format!(r#"{{"error":{{"type":"{kind}","message":"에러: {kind}"}}}}"#);
        match parse_messages_response(&json) {
            Err(TranslationError::Api { code, message, .. }) => {
                assert_eq!(code, expected_code, "kind={kind}");
                assert_eq!(message, format!("에러: {kind}"));
            }
            other => panic!("expected Api error for {kind}, got {other:?}"),
        }
    }
}

#[test]
fn a_response_without_content_or_error_is_a_parse_error() {
    assert!(matches!(
        parse_messages_response(r#"{"stop_reason":"end_turn"}"#),
        Err(TranslationError::Parse(_))
    ));
}

#[tokio::test]
async fn rejects_an_empty_source_text_before_any_network_call() {
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
        max_tokens: 32,
        reasoning_effort: None,
        glossary: Vec::new(),
    };
    let result = translate_async_with_client(
        &crate::translation::http_common::create_client(),
        "",
        crate::translation::Language::Eng,
        crate::translation::Language::Kor,
        &params,
    )
    .await;
    assert!(matches!(result, Err(TranslationError::EmptyText)));
}

#[tokio::test]
async fn rejects_a_missing_api_key_before_any_network_call() {
    let params = LlmCallParams {
        provider: super::super::LlmProvider::Anthropic,
        model: "model".into(),
        api_key: String::new(),
        base_url: String::new(),
        system_prompt: String::new(),
        temperature: 0.5,
        top_p: 1.0,
        frequency_penalty: 0.0,
        presence_penalty: 0.0,
        max_tokens: 32,
        reasoning_effort: None,
        glossary: Vec::new(),
    };
    let result = translate_async_with_client(
        &crate::translation::http_common::create_client(),
        "source",
        crate::translation::Language::Eng,
        crate::translation::Language::Kor,
        &params,
    )
    .await;
    assert!(matches!(result, Err(TranslationError::MissingApiKey)));
}

#[tokio::test]
async fn sends_the_x_api_key_header_and_parses_the_content_block() {
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
            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let body_start = header_end + 4;
            let headers = String::from_utf8_lossy(&request[..body_start]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if request.len() >= body_start + content_length {
                break;
            }
        }
        request_tx.send(request).unwrap();
        let body = r#"{"content":[{"type":"text","text":"번역됨"}],"stop_reason":"end_turn"}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    let params = LlmCallParams {
        provider: super::super::LlmProvider::Anthropic,
        model: "claude-haiku-4-5".into(),
        api_key: "test-anthropic-key".into(),
        base_url: format!("http://{address}"),
        system_prompt: String::new(),
        temperature: 0.5,
        top_p: 1.0,
        frequency_penalty: 0.0,
        presence_penalty: 0.0,
        max_tokens: 32,
        reasoning_effort: None,
        glossary: Vec::new(),
    };
    let translated = translate_async_with_client(
        &crate::translation::http_common::create_client(),
        "hello",
        crate::translation::Language::Eng,
        crate::translation::Language::Kor,
        &params,
    )
    .await
    .unwrap();
    assert_eq!(translated, "번역됨");

    let request = request_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
    assert!(request.contains("x-api-key: test-anthropic-key"));
    assert!(request.contains("anthropic-version: 2023-06-01"));
    server.join().unwrap();
}
