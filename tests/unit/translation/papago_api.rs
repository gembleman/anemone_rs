use super::*;
use crate::translation::TranslationEngine;

#[test]
fn parses_translated_text_from_a_successful_response() {
    let body = r#"{"message":{"result":{"translatedText":"안녕하세요"}}}"#;
    assert_eq!(parse_papago_response(body).unwrap(), "안녕하세요");
}

#[test]
fn maps_error_message_and_code_to_an_api_error() {
    let body = r#"{"errorCode":"N2MT05","errorMessage":"지원하지 않는 언어 조합입니다."}"#;
    match parse_papago_response(body) {
        Err(TranslationError::Api { code, message, .. }) => {
            // errorCode가 숫자로 파싱되지 않으면 0으로 대체된다.
            assert_eq!(code, 0);
            assert_eq!(message, "지원하지 않는 언어 조합입니다.");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[test]
fn a_response_without_result_or_error_fields_is_a_parse_error() {
    assert!(matches!(
        parse_papago_response(r#"{"unexpected":true}"#),
        Err(TranslationError::Parse(_))
    ));
}

#[test]
fn rejects_empty_text_before_any_network_call() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(translate_async_with_client(
        &crate::translation::http_common::create_client(),
        "",
        Language::Eng,
        Language::Kor,
        "id",
        "secret",
    ));
    assert!(matches!(result, Err(TranslationError::EmptyText)));
}

#[test]
fn rejects_language_pairs_papago_does_not_support() {
    // Por는 Papago 지원 언어 목록에 없다.
    assert!(!TranslationEngine::Papago.supports_pair(Language::Kor, Language::Por));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(translate_async_with_client(
        &crate::translation::http_common::create_client(),
        "hello",
        Language::Kor,
        Language::Por,
        "id",
        "secret",
    ));
    assert!(matches!(
        result,
        Err(TranslationError::UnsupportedLanguagePair)
    ));
}

#[test]
fn rejects_missing_credentials_before_any_network_call() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(translate_async_with_client(
        &crate::translation::http_common::create_client(),
        "hello",
        Language::Kor,
        Language::Eng,
        "",
        "secret",
    ));
    assert!(matches!(result, Err(TranslationError::MissingApiKey)));

    let result = runtime.block_on(translate_async_with_client(
        &crate::translation::http_common::create_client(),
        "hello",
        Language::Kor,
        Language::Eng,
        "id",
        "",
    ));
    assert!(matches!(result, Err(TranslationError::MissingApiKey)));
}

#[tokio::test]
async fn sends_ncloud_headers_and_form_body_to_the_configured_endpoint() {
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
        let body = r#"{"message":{"result":{"translatedText":"world"}}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    let translated = send_papago_request(
        &crate::translation::http_common::create_client(),
        &format!("http://{address}/nmt/v1/translation"),
        "안녕",
        "ko",
        "en",
        "test-client-id",
        "test-client-secret",
    )
    .await
    .unwrap();
    assert_eq!(translated, "world");

    let request = request_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
    assert!(request.contains("x-ncp-apigw-api-key-id: test-client-id"));
    assert!(request.contains("x-ncp-apigw-api-key: test-client-secret"));
    assert!(request.contains("source=ko"));
    assert!(request.contains("target=en"));
    server.join().unwrap();
}

#[tokio::test]
async fn surfaces_an_api_error_returned_by_the_endpoint() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
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
        let body = r#"{"errorCode":"429","errorMessage":"요청량 초과"}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    let result = send_papago_request(
        &crate::translation::http_common::create_client(),
        &format!("http://{address}/nmt/v1/translation"),
        "hi",
        "ko",
        "en",
        "id",
        "secret",
    )
    .await;
    match result {
        Err(TranslationError::Api { code, message, .. }) => {
            assert_eq!(code, 429);
            assert_eq!(message, "요청량 초과");
        }
        other => panic!("expected Api(429), got {other:?}"),
    }
    server.join().unwrap();
}
