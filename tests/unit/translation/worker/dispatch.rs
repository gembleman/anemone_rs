use super::state::{DispatchShared, RouteSlot};
use super::{
    CompletionNotifier, TargetId, TranslationDispatch, TranslationRequest, TranslationRequestError,
    TranslationResponse,
};
use crate::translation::{Language, PreparedJob};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::mpsc as std_mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Default)]
struct TestNotifier {
    notifications: AtomicUsize,
}

impl CompletionNotifier for TestNotifier {
    fn notify(&self, _target: TargetId, _request_id: u64) -> Result<(), String> {
        self.notifications.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn test_notifier() -> Arc<TestNotifier> {
    Arc::new(TestNotifier::default())
}

fn test_dispatch() -> TranslationDispatch {
    TranslationDispatch::spawn(
        test_notifier(),
        crate::translation::http_common::create_client(),
    )
}

fn spawn_llm_server(
    delay: Duration,
) -> (
    String,
    mpsc::Receiver<()>,
    mpsc::Receiver<()>,
    JoinHandle<()>,
) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request);
        let _ = started_tx.send(());
        thread::sleep(delay);
        let body = r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = finished_tx.send(());
    });
    (format!("http://{address}"), started_rx, finished_rx, handle)
}

fn llm_request_for_url(base_url: String) -> TranslationRequest {
    let config = crate::config::TranslationConfig {
        engine: "llm".into(),
        source_lang: "en".into(),
        target_lang: "ko".into(),
        llm: crate::config::LlmConfig {
            model: "test".into(),
            api_key: "test-key".into(),
            base_url,
            system_prompt: "translate".into(),
            temperature: 0.0,
            max_tokens: 32,
            glossary: Vec::new(),
            ..crate::config::LlmConfig::default()
        },
        ..crate::config::TranslationConfig::default()
    };
    TranslationRequest {
        id: 0,
        text: Arc::from("source"),
        job: PreparedJob::from_config(&config).unwrap(),
    }
}

#[test]
fn disconnected_worker_rejects_request_and_rolls_back_latest_id() {
    let (sender, receiver) = mpsc::channel();
    drop(receiver);
    let shared = Arc::new(DispatchShared::new(test_notifier()));
    let dispatch = TranslationDispatch {
        sender: Mutex::new(Some(sender)),
        shared: shared.clone(),
        worker: Mutex::new(None),
    };

    let result = dispatch.request(
        TargetId::new(0),
        TranslationRequest {
            id: 0,
            text: Arc::from("source"),
            job: PreparedJob::google(Language::Jpn, Language::Kor).unwrap(),
        },
    );

    assert_eq!(result, Err(TranslationRequestError::WorkerUnavailable));
    assert_eq!(
        shared
            .latest_atomic_lookup(TargetId::new(0))
            .expect("request allocated a latest-id slot")
            .load(Ordering::Acquire),
        0
    );
}

#[test]
fn unregister_invalidates_the_old_target_generation() {
    let shared = DispatchShared::new(test_notifier());
    let target = TargetId::new(42);
    let old_route = shared.route(target);
    old_route.set_latest(1);
    shared.unregister(target);
    let new_route = shared.route(target);
    new_route.set_latest(2);

    assert!(!Arc::ptr_eq(&old_route, &new_route));
    assert_eq!(old_route.latest_id.load(Ordering::Acquire), 0);
    assert_eq!(new_route.latest_id.load(Ordering::Acquire), 2);
}

#[test]
fn late_response_after_unregister_is_discarded_without_notification() {
    let notifier = test_notifier();
    let shared = DispatchShared::new(notifier.clone());
    let target = TargetId::new(77);
    let old_route = shared.route(target);
    old_route.set_latest(1);
    shared.unregister(target);

    TranslationDispatch::route_response(&shared, target, &old_route, 1, Ok("late".to_string()));

    assert!(shared.state.lock().unwrap().pending.is_empty());
    assert_eq!(notifier.notifications.load(Ordering::Relaxed), 0);
}

#[test]
fn superseding_a_request_wakes_its_cancellation_waiter() {
    let route = RouteSlot::new();
    let mut cancellation = route.set_latest(1);
    route.set_latest(2);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(TranslationDispatch::wait_until_superseded(
        &mut cancellation,
        1,
    ));
}

#[test]
fn slow_consumer_does_not_head_of_line_block_another_consumer() {
    let (slow_url, slow_started, _slow_finished, slow_server) =
        spawn_llm_server(Duration::from_millis(1_500));
    let (fast_url, _fast_started, fast_finished, fast_server) = spawn_llm_server(Duration::ZERO);
    let dispatch = test_dispatch();

    dispatch
        .request(TargetId::new(1), llm_request_for_url(slow_url))
        .unwrap();
    slow_started.recv_timeout(Duration::from_secs(1)).unwrap();
    dispatch
        .request(TargetId::new(2), llm_request_for_url(fast_url))
        .unwrap();

    // 직렬 워커라면 slow의 1.5초 응답 이후에야 fast 서버가 호출된다.
    fast_finished
        .recv_timeout(Duration::from_millis(700))
        .expect("fast consumer was head-of-line blocked by slow consumer");

    dispatch.shutdown();
    slow_server.join().unwrap();
    fast_server.join().unwrap();
}

#[test]
fn newer_request_cancels_stalled_http_and_starts_immediately() {
    let (base_url, accepted, cancelled, server) = spawn_two_request_server();
    let dispatch = test_dispatch();
    let target = TargetId::new(1);

    let first = dispatch
        .request(target, llm_request("first", &base_url))
        .unwrap();
    assert_eq!(
        accepted.recv_timeout(Duration::from_secs(2)).unwrap(),
        "first"
    );

    let second = dispatch
        .request(target, llm_request("second", &base_url))
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
    let dispatch = test_dispatch();

    dispatch
        .request(TargetId::new(11), llm_request("slow-window", &base_url))
        .unwrap();
    assert_eq!(
        accepted.recv_timeout(Duration::from_secs(2)).unwrap(),
        "slow-window"
    );
    let fast = dispatch
        .request(TargetId::new(12), llm_request("fast-window", &base_url))
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
    let dispatch = test_dispatch();
    dispatch
        .request(TargetId::new(21), llm_request("shutdown", &base_url))
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

fn llm_request(text: &str, base_url: &str) -> TranslationRequest {
    let config = crate::config::TranslationConfig {
        engine: "llm".into(),
        llm: crate::config::LlmConfig {
            api_key: "test-key".to_string(),
            base_url: base_url.to_string(),
            ..crate::config::LlmConfig::default()
        },
        ..crate::config::TranslationConfig::default()
    };
    TranslationRequest {
        id: 0,
        text: Arc::from(text),
        job: PreparedJob::from_config(&config).unwrap(),
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
        assert!(
            headers
                .lines()
                .next()
                .is_some_and(|line| line == "POST /responses HTTP/1.1"),
            "OpenAI request did not use the Responses API: {headers}"
        );
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
            return value["input"].as_str().unwrap().to_string();
        }
    }
}

fn write_success(stream: &mut TcpStream, translated: &str) {
    let body = serde_json::json!({
        "status": "completed",
        "output": [{
            "type": "message",
            "content": [{
                "type": "output_text",
                "text": translated
            }]
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
