use super::{TranslationDispatch, TranslationRequest};
use crate::translation::{Language, PreparedJob, TranslationError};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

// 번역 서버 dispatch 테스트는 요청 계약을 그대로 드러내므로 비공개
// 구현과 함께만 존재한다(`build.rs`의 `mys_private` 참고).
#[cfg(mys_private)]
#[path = "translate_mys.rs"]
mod mys;

#[test]
fn retry_after_overrides_exponential_backoff() {
    let error = TranslationError::Api {
        code: 429,
        message: "slow down".into(),
        retry_after: Some(Duration::from_secs(7)),
    };
    assert_eq!(
        TranslationDispatch::retry_delay(Some(&error), 1),
        Duration::from_secs(7)
    );
    assert_eq!(
        TranslationDispatch::retry_delay(None, 3),
        Duration::from_secs(2)
    );
}

#[test]
fn oversized_input_is_rejected_before_network_io() {
    let req = TranslationRequest {
        id: 1,
        text: Arc::from(
            "가".repeat(crate::translation::TranslationEngine::Google.max_input_chars() + 1),
        ),
        job: PreparedJob::google(Language::Kor, Language::Eng).unwrap(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(TranslationDispatch::translate_async(
        &req,
        &crate::translation::http_common::create_client(),
    ));
    assert!(matches!(
        result,
        Err(TranslationError::InputTooLong {
            engine: "google",
            ..
        })
    ));
}

#[test]
fn deepl_should_fallback_only_on_quota_or_auth_style_codes() {
    let fallback = |code: u16| {
        TranslationDispatch::deepl_should_fallback(&TranslationError::Api {
            code,
            message: "x".into(),
            retry_after: None,
        })
    };
    assert!(fallback(429));
    assert!(fallback(456));
    assert!(fallback(403));
    assert!(!fallback(500));
    assert!(!fallback(400));
    assert!(!TranslationDispatch::deepl_should_fallback(
        &TranslationError::Network("boom".into())
    ));
    // RateLimited도 Api와 같은 코드 판정을 공유한다 (같은 매치 갈래).
    assert!(TranslationDispatch::deepl_should_fallback(
        &TranslationError::RateLimited {
            code: 429,
            message: "x".into(),
            retry_after: None,
        }
    ));
    assert!(!TranslationDispatch::deepl_should_fallback(
        &TranslationError::RateLimited {
            code: 500,
            message: "x".into(),
            retry_after: None,
        }
    ));
}

#[test]
fn deepl_multi_key_rejects_an_empty_key_list_without_any_network_call() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(TranslationDispatch::translate_deepl_multi_key(
        &crate::translation::http_common::create_client(),
        "text",
        Language::Eng,
        Language::Kor,
        &[],
        crate::translation::DeepLStrategy::Failover,
    ));
    assert!(matches!(result, Err(TranslationError::MissingApiKey)));
}

#[test]
fn custom_engine_dispatch_posts_the_template_and_extracts_the_configured_path() {
    let (base_url, server) = spawn_json_stub_server(r#"{"translatedText":"커스텀 번역"}"#);
    let config = crate::config::TranslationConfig {
        engine: "custom".into(),
        source_lang: "en".into(),
        target_lang: "ko".into(),
        custom: crate::config::CustomApiConfig {
            url: base_url,
            ..crate::config::CustomApiConfig::default()
        },
        ..crate::config::TranslationConfig::default()
    };
    let req = TranslationRequest {
        id: 1,
        text: Arc::from("hello"),
        job: PreparedJob::from_config(&config).unwrap(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(TranslationDispatch::translate_async(
        &req,
        &crate::translation::http_common::create_client(),
    ));
    assert_eq!(result.unwrap(), "커스텀 번역");
    server.join().unwrap();
}

#[test]
fn llm_anthropic_engine_dispatch_reaches_the_anthropic_backend() {
    let (base_url, server) = spawn_json_stub_server(
        r#"{"content":[{"type":"text","text":"클로드 번역"}],"stop_reason":"end_turn"}"#,
    );
    let config = crate::config::TranslationConfig {
        engine: "llm".into(),
        source_lang: "en".into(),
        target_lang: "ko".into(),
        llm: crate::config::LlmConfig {
            provider: "anthropic".into(),
            model: "claude-haiku-4-5".into(),
            api_key: "test-key".into(),
            base_url,
            max_tokens: 32,
            ..crate::config::LlmConfig::default()
        },
        ..crate::config::TranslationConfig::default()
    };
    let req = TranslationRequest {
        id: 1,
        text: Arc::from("hello"),
        job: PreparedJob::from_config(&config).unwrap(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(TranslationDispatch::translate_async(
        &req,
        &crate::translation::http_common::create_client(),
    ));
    assert_eq!(result.unwrap(), "클로드 번역");
    server.join().unwrap();
}

#[test]
fn llm_gemini_engine_dispatch_reaches_the_gemini_backend() {
    let (base_url, server) = spawn_json_stub_server(
        r#"{"candidates":[{"content":{"parts":[{"text":"제미니 번역"}]},"finishReason":"STOP"}]}"#,
    );
    let config = crate::config::TranslationConfig {
        engine: "llm".into(),
        source_lang: "en".into(),
        target_lang: "ko".into(),
        llm: crate::config::LlmConfig {
            provider: "gemini".into(),
            model: "gemini-2.5-flash".into(),
            api_key: "test-key".into(),
            base_url,
            max_tokens: 32,
            ..crate::config::LlmConfig::default()
        },
        ..crate::config::TranslationConfig::default()
    };
    let req = TranslationRequest {
        id: 1,
        text: Arc::from("hello"),
        job: PreparedJob::from_config(&config).unwrap(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(TranslationDispatch::translate_async(
        &req,
        &crate::translation::http_common::create_client(),
    ));
    assert_eq!(result.unwrap(), "제미니 번역");
    server.join().unwrap();
}

/// 단일 요청/단일 JSON 응답만 필요한 테스트를 위한 최소 스텁 서버.
fn spawn_json_stub_server(body: &'static str) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_http_headers(&mut stream);
        write_raw_response(&mut stream, "200 OK", body);
    });
    (base_url, server)
}

fn read_http_headers(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let count = stream.read(&mut chunk).unwrap();
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            return String::from_utf8_lossy(&bytes[..end + 4]).into_owned();
        }
    }
}

fn write_raw_response(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
}
