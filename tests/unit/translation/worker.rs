use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc as std_mpsc;
use std::time::Instant;

#[test]
fn disconnected_worker_rejects_request_and_rolls_back_latest_id() {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    drop(receiver);
    let shared = Arc::new(DispatchShared::new());
    let dispatch = TranslationDispatch {
        sender: Mutex::new(Some(sender)),
        shared: shared.clone(),
        worker: Mutex::new(None),
    };

    let result = dispatch.request(
        HWND::default(),
        TranslationRequest {
            id: 0,
            text: Arc::from("source"),
            engine: TranslationEngine::Google,
            source_lang: Language::Jpn,
            target_lang: Language::Kor,
            credentials: EngineCredentials::None,
        },
    );

    assert_eq!(result, Err(TranslationRequestError::WorkerUnavailable));
    assert_eq!(
        shared
            .latest_atomic_lookup(0)
            .expect("request allocated a latest-id slot")
            .load(Ordering::Acquire),
        0
    );
}

#[test]
fn newer_request_cancels_stalled_http_and_starts_immediately() {
    let (base_url, accepted, cancelled, server) = spawn_two_request_server();
    let dispatch = TranslationDispatch::spawn();
    let hwnd = test_hwnd(1);

    let first = dispatch
        .request(hwnd, llm_request("first", &base_url))
        .unwrap();
    assert_eq!(
        accepted.recv_timeout(Duration::from_secs(2)).unwrap(),
        "first"
    );

    let second = dispatch
        .request(hwnd, llm_request("second", &base_url))
        .unwrap();
    assert!(second > first);
    assert_eq!(
        accepted.recv_timeout(Duration::from_secs(1)).unwrap(),
        "second"
    );
    assert!(cancelled.recv_timeout(Duration::from_secs(2)).unwrap());
    assert_eq!(
        wait_for_response(&dispatch, second)
            .result
            .expect("latest request succeeds"),
        "translated-second"
    );
    assert!(dispatch.take_response(first).is_none());

    dispatch.shutdown();
    server.join().unwrap();
}

#[test]
fn separate_windows_are_not_serially_blocked() {
    let (base_url, accepted, _cancelled, server) = spawn_two_request_server();
    let dispatch = TranslationDispatch::spawn();

    dispatch
        .request(test_hwnd(11), llm_request("slow-window", &base_url))
        .unwrap();
    assert_eq!(
        accepted.recv_timeout(Duration::from_secs(2)).unwrap(),
        "slow-window"
    );
    let fast = dispatch
        .request(test_hwnd(12), llm_request("fast-window", &base_url))
        .unwrap();
    assert_eq!(
        accepted.recv_timeout(Duration::from_secs(1)).unwrap(),
        "fast-window"
    );
    assert_eq!(
        wait_for_response(&dispatch, fast).result.unwrap(),
        "translated-second"
    );

    dispatch.shutdown();
    server.join().unwrap();
}

#[test]
fn shutdown_aborts_in_flight_http_and_joins_within_two_seconds() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let (accepted_tx, accepted_rx) = std_mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request_body(&mut stream);
        accepted_tx.send(()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut byte = [0u8; 1];
        matches!(stream.read(&mut byte), Ok(0))
    });
    let dispatch = TranslationDispatch::spawn();
    dispatch
        .request(test_hwnd(21), llm_request("shutdown", &base_url))
        .unwrap();
    accepted_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let started = Instant::now();
    dispatch.shutdown();

    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        server.join().unwrap(),
        "HTTP socket should close on shutdown"
    );
}

fn test_hwnd(value: usize) -> HWND {
    HWND(value as *mut std::ffi::c_void)
}

fn llm_request(text: &str, base_url: &str) -> TranslationRequest {
    let config = crate::config::LlmConfig {
        api_key: "test-key".to_string(),
        base_url: base_url.to_string(),
        ..crate::config::LlmConfig::default()
    };
    TranslationRequest {
        id: 0,
        text: Arc::from(text),
        engine: TranslationEngine::Llm,
        source_lang: Language::Jpn,
        target_lang: Language::Kor,
        credentials: EngineCredentials::Llm(config.to_call_params()),
    }
}

fn wait_for_response(dispatch: &TranslationDispatch, req_id: u64) -> TranslationResponse {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(response) = dispatch.take_response(req_id) {
            return response;
        }
        assert!(Instant::now() < deadline, "response {req_id} timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn spawn_two_request_server() -> (
    String,
    std_mpsc::Receiver<String>,
    std_mpsc::Receiver<bool>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let (accepted_tx, accepted_rx) = std_mpsc::channel();
    let (cancelled_tx, cancelled_rx) = std_mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        accepted_tx.send(read_request_body(&mut first)).unwrap();

        let (mut second, _) = listener.accept().unwrap();
        accepted_tx.send(read_request_body(&mut second)).unwrap();
        write_success(&mut second, "translated-second");

        first
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0u8; 1];
        let was_cancelled = matches!(first.read(&mut byte), Ok(0));
        let _ = cancelled_tx.send(was_cancelled);
    });
    (base_url, accepted_rx, cancelled_rx, server)
}

fn read_request_body(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        let count = stream.read(&mut chunk).unwrap();
        bytes.extend_from_slice(&chunk[..count]);
        let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let header_end = header_end + 4;
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if bytes.len() >= header_end + content_length {
            let body = String::from_utf8_lossy(&bytes[header_end..header_end + content_length]);
            let value: serde_json::Value = serde_json::from_str(&body).unwrap();
            return value["messages"][1]["content"]
                .as_str()
                .unwrap()
                .to_string();
        }
    }
}

fn write_success(stream: &mut TcpStream, translated: &str) {
    let body = serde_json::json!({
        "choices": [{
            "message": { "content": translated },
            "finish_reason": "stop"
        }]
    })
    .to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
}
