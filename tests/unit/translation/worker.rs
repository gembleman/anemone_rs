use super::*;

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

fn llm_request(base_url: String) -> TranslationRequest {
    TranslationRequest {
        id: 0,
        text: Arc::from("source"),
        engine: TranslationEngine::Llm,
        source_lang: Language::Eng,
        target_lang: Language::Kor,
        credentials: EngineCredentials::Llm(LlmCallParams {
            provider: LlmProvider::OpenAi,
            model: "test".into(),
            api_key: "test-key".into(),
            base_url,
            system_prompt: "translate".into(),
            temperature: 0.0,
            max_tokens: 32,
            glossary: Vec::new(),
        }),
    }
}

#[test]
fn disconnected_worker_rejects_request_and_rolls_back_latest_id() {
    let (sender, receiver) = mpsc::channel();
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
        text: Arc::from("가".repeat(TranslationEngine::Google.max_input_chars() + 1)),
        engine: TranslationEngine::Google,
        source_lang: Language::Kor,
        target_lang: Language::Eng,
        credentials: EngineCredentials::None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(TranslationDispatch::translate_async(
        &req,
        &reqwest::Client::new(),
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
fn unregister_invalidates_the_old_hwnd_generation() {
    let shared = DispatchShared::new();
    let old_route = shared.route(42);
    old_route.set_latest(1);
    shared.unregister(42);
    let new_route = shared.route(42);
    new_route.set_latest(2);

    assert!(!Arc::ptr_eq(&old_route, &new_route));
    assert_eq!(old_route.latest_id.load(Ordering::Acquire), 0);
    assert_eq!(new_route.latest_id.load(Ordering::Acquire), 2);
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
    let dispatch = TranslationDispatch::spawn();
    let mut slow_consumer = 0u8;
    let mut fast_consumer = 0u8;

    dispatch
        .request(
            HWND((&mut slow_consumer as *mut u8).cast()),
            llm_request(slow_url),
        )
        .unwrap();
    slow_started.recv_timeout(Duration::from_secs(1)).unwrap();
    dispatch
        .request(
            HWND((&mut fast_consumer as *mut u8).cast()),
            llm_request(fast_url),
        )
        .unwrap();

    // 직렬 워커라면 slow의 1.5초 응답 이후에야 fast 서버가 호출된다.
    fast_finished
        .recv_timeout(Duration::from_millis(700))
        .expect("fast consumer was head-of-line blocked by slow consumer");

    dispatch.shutdown();
    slow_server.join().unwrap();
    fast_server.join().unwrap();
}
