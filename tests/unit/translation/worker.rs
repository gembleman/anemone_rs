use super::*;

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
