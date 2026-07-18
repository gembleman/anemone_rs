use super::{
    CustomApiCallParams, build_extra_headers, extract_translation, substitute_json_strings,
    translate_async_with_client,
};

#[test]
fn substitutes_placeholders_only_in_json_string_values() {
    let mut value = serde_json::json!({
        "text": "before {text} after",
        "languages": ["{source}", "{target}"],
        "nested": { "key": "{api_key}" },
        "number": 1
    });
    substitute_json_strings(
        &mut value,
        &[
            ("{text}", "hello \"world\""),
            ("{source}", "ja"),
            ("{target}", "ko"),
            ("{api_key}", "secret"),
        ],
    );
    assert_eq!(value["text"], "before hello \"world\" after");
    assert_eq!(value["languages"], serde_json::json!(["ja", "ko"]));
    assert_eq!(value["nested"]["key"], "secret");
    assert_eq!(value["number"], 1);
}

#[test]
fn extracts_json_pointer_and_dot_paths_with_array_indexes() {
    let body = r#"{"data":{"translations":[{"text":"번역"}]}}"#;
    assert_eq!(
        extract_translation(body, "/data/translations/0/text").unwrap(),
        "번역"
    );
    assert_eq!(
        extract_translation(body, "data.translations.0.text").unwrap(),
        "번역"
    );
}

#[test]
fn empty_response_path_returns_plain_body() {
    assert_eq!(
        extract_translation("plain result", "").unwrap(),
        "plain result"
    );
}

#[test]
fn validates_url_template_and_header() {
    let valid = CustomApiCallParams {
        url: "https://example.com/translate".into(),
        api_key: "secret".into(),
        auth_header: "X-API-Key".into(),
        auth_scheme: String::new(),
        headers: r#"{"X-Tenant":"demo"}"#.into(),
        request_template: r#"{"q":"{text}"}"#.into(),
        response_path: "translated".into(),
    };
    assert!(valid.validate().is_ok());

    let mut invalid = valid.clone();
    invalid.url = "file:///tmp/translate".into();
    assert!(invalid.validate().is_err());
    invalid.url = valid.url;
    invalid.request_template = "not-json".into();
    assert!(invalid.validate().is_err());
}

#[test]
fn builds_templated_extra_headers() {
    let headers = build_extra_headers(
        r#"{"X-API-Key":"{api_key}","X-Language":"{source}-{target}"}"#,
        &[
            ("{api_key}", "secret"),
            ("{source}", "ja"),
            ("{target}", "ko"),
        ],
    )
    .unwrap();
    assert_eq!(headers["X-API-Key"], "secret");
    assert_eq!(headers["X-Language"], "ja-ko");
    assert!(build_extra_headers(r#"{"X-Test":1}"#, &[]).is_err());
}

#[tokio::test]
async fn posts_rendered_json_and_extracts_the_configured_response() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
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
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap();
            if request.len() >= body_start + length {
                break;
            }
        }
        request_tx.send(request).unwrap();
        let body = r#"{"data":{"translated":"안녕"}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    let params = CustomApiCallParams {
        url: format!("http://{address}/translate"),
        api_key: "secret".into(),
        auth_header: "X-API-Key".into(),
        auth_scheme: String::new(),
        headers: r#"{"X-Language":"{source}-{target}"}"#.into(),
        request_template: r#"{"q":"{text}","from":"{source}","to":"{target}"}"#.into(),
        response_path: "data.translated".into(),
    };
    let translated = translate_async_with_client(
        &reqwest::Client::new(),
        "hello \"world\"",
        crate::translation::Language::Eng,
        crate::translation::Language::Kor,
        &params,
    )
    .await
    .unwrap();
    assert_eq!(translated, "안녕");

    let request = request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let body_start = request
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap()
        + 4;
    let headers = String::from_utf8_lossy(&request[..body_start]).to_ascii_lowercase();
    assert!(headers.contains("x-api-key: secret"));
    assert!(headers.contains("x-language: en-ko"));
    let body: serde_json::Value = serde_json::from_slice(&request[body_start..]).unwrap();
    assert_eq!(body["q"], "hello \"world\"");
    assert_eq!(body["from"], "en");
    assert_eq!(body["to"], "ko");
    server.join().unwrap();
}
