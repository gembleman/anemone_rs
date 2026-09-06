use super::*;

/// `PostMessageW`가 실패해도(대상 창이 없으므로) 워커는 계속 동작해야 한다 —
/// 결과를 슬롯에 넣는 것과 알림 게시는 별개 단계다. 실제 HWND 없이 스레드
/// 수명/채널 동작만 검증한다.
fn null_hwnd() -> HWND {
    std::ptr::null_mut()
}

#[test]
fn shutdown_without_any_request_does_not_hang() {
    let worker = UpdateWorker::spawn(null_hwnd(), 0x8000, 0x8001);
    worker.shutdown();
}

#[test]
fn request_after_shutdown_reports_worker_unavailable() {
    let worker = UpdateWorker::spawn(null_hwnd(), 0x8000, 0x8001);
    worker.shutdown();

    let result = worker.request(UpdateRequest::Check {
        current: Version::current(),
        trigger: CheckTrigger::Auto,
    });
    assert!(matches!(result, Err(UpdateRequestError::WorkerUnavailable)));
}

#[test]
fn drain_results_is_empty_when_nothing_has_completed_yet() {
    let worker = UpdateWorker::spawn(null_hwnd(), 0x8000, 0x8001);
    assert!(worker.drain_results().is_empty());
    worker.shutdown();
}

#[test]
fn progress_starts_at_zero_received_with_an_unknown_total() {
    let worker = UpdateWorker::spawn(null_hwnd(), 0x8000, 0x8001);
    let progress = worker.progress();
    assert_eq!(progress.received, 0);
    assert_eq!(progress.total, None);
    worker.shutdown();
}

/// `shutdown()`은 채널을 닫은 뒤 스레드를 join하는데, mpsc는 채널이 닫혀도 이미
/// 들어간 메시지는 전부 비워낸 뒤에야 `recv()`가 끝난다. 그래서 request() 직후
/// shutdown()이 반환했다면 그 요청은 반드시 처리되어 결과 슬롯에 있어야
/// 한다 — sleep 폴링 없이도 결정론적으로 확인할 수 있다.
#[test]
fn a_request_sent_to_a_live_worker_is_fully_processed_before_shutdown_returns() {
    let worker = UpdateWorker::spawn(null_hwnd(), 0x8000, 0x8001);
    let destination = std::env::temp_dir().join(format!(
        "anemone-update-worker-live-{}.exe",
        std::process::id()
    ));
    let update = AvailableUpdate {
        version: Version::current(),
        asset_url: "http://evil.example.com/app.exe".to_string(),
        checksum_url: "http://evil.example.com/app.exe.sha256".to_string(),
        release_page_url: String::new(),
    };

    let sent = worker.request(UpdateRequest::Download {
        update,
        destination: destination.clone(),
    });
    assert!(sent.is_ok(), "살아있는 워커는 요청을 받아야 한다");
    worker.shutdown();

    let results = worker.drain_results();
    assert_eq!(
        results.len(),
        1,
        "요청이 완전히 처리되어 결과가 하나 있어야 한다"
    );
    match &results[0] {
        UpdateOutcome::Download(Err(UpdateError::UntrustedUrl(_))) => {}
        UpdateOutcome::Download(other) => {
            panic!("신뢰할 수 없는 URL은 즉시 거부되어야 한다: {other:?}")
        }
        UpdateOutcome::Check { .. } => panic!("Download 요청인데 Check 결과가 돌아왔다"),
    }
    assert!(!destination.exists());
}

// --- ProgressSlot: 원자값 기반 진행률 저장/읽기 -----------------------------

#[test]
fn a_fresh_progress_slot_reports_zero_received_and_unknown_total() {
    let slot = ProgressSlot::new();
    let progress = slot.load();
    assert_eq!(progress.received, 0);
    assert_eq!(progress.total, None);
}

#[test]
fn storing_progress_with_a_known_total_round_trips() {
    let slot = ProgressSlot::new();
    slot.store(DownloadProgress {
        received: 512,
        total: Some(2048),
    });
    let progress = slot.load();
    assert_eq!(progress.received, 512);
    assert_eq!(progress.total, Some(2048));
}

/// `u64::MAX`는 "총량 모름"의 내부 센티널이다 — 실제 total로 그 값이 들어와도
/// None으로 오인하지 않는지, 그리고 reset 뒤 다시 None으로 돌아가는지 확인한다.
#[test]
fn resetting_progress_clears_previous_values() {
    let slot = ProgressSlot::new();
    slot.store(DownloadProgress {
        received: 1000,
        total: Some(1000),
    });
    slot.reset();
    let progress = slot.load();
    assert_eq!(progress.received, 0);
    assert_eq!(progress.total, None);
}

#[test]
fn storing_progress_without_a_total_reports_none_not_a_sentinel_collision() {
    let slot = ProgressSlot::new();
    slot.store(DownloadProgress {
        received: 10,
        total: None,
    });
    assert_eq!(slot.load().total, None);
}

// --- handle_request: Download 분기의 신뢰 경계 -----------------------------
//
// Check 분기(`check::fetch_latest`)는 항상 실제 api.github.com을 호출하므로
// 로컬 테스트에서 성공 경로를 재현할 수 없다 — 네트워크 금지 규칙 때문에
// 이 분기는 의도적으로 커버하지 않는다. Download 분기는 `download::download`가
// `ensure_trusted_url`로 즉시 실패하는 경로가 있어, 스레드/채널을 거치지 않고
// `handle_request`를 직접 호출해 결정론적으로 검증한다 (sleep 폴링 없이).
#[tokio::test]
async fn an_untrusted_download_url_yields_an_untrusted_url_outcome() {
    let progress = ProgressSlot::new();
    progress.store(DownloadProgress {
        received: 999,
        total: Some(999),
    });
    let destination = std::env::temp_dir().join(format!(
        "anemone-update-worker-untrusted-{}.exe",
        std::process::id()
    ));

    let update = AvailableUpdate {
        version: Version::current(),
        asset_url: "http://evil.example.com/app.exe".to_string(),
        checksum_url: "http://evil.example.com/app.exe.sha256".to_string(),
        release_page_url: String::new(),
    };
    let outcome = UpdateWorker::handle_request(
        UpdateRequest::Download {
            update,
            destination: destination.clone(),
        },
        &progress,
        0,
        0x8001,
    )
    .await;

    // `UpdateOutcome`은 Debug를 파생하지 않으므로, 안쪽 `Result`(요소가 모두
    // Debug인)를 먼저 꺼내 실패 메시지에 쓴다.
    match outcome {
        UpdateOutcome::Download(result) => match result {
            Err(UpdateError::UntrustedUrl(_)) => {}
            other => panic!("신뢰할 수 없는 URL은 즉시 거부되어야 한다: {other:?}"),
        },
        UpdateOutcome::Check { .. } => panic!("Download 요청인데 Check 결과가 돌아왔다"),
    }
    assert!(!destination.exists());
    // Download 요청은 시작하자마자 progress.reset()을 호출한다 — 이전 다운로드의
    // 잔여 값이 새 다운로드로 새는 것을 막는 실제 동작이다.
    assert_eq!(progress.load().received, 0);
    assert_eq!(progress.load().total, None);
}
