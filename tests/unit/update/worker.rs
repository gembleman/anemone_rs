use super::*;

/// `PostMessageW`가 실패해도(대상 창이 없으므로) 워커는 계속 동작해야 한다 —
/// 결과를 슬롯에 넣는 것과 알림 게시는 별개 단계다. 실제 HWND 없이 스레드
/// 수명/채널 동작만 검증한다.
fn null_hwnd() -> HWND {
    HWND(std::ptr::null_mut())
}

#[test]
fn shutdown_without_any_request_does_not_hang() {
    let worker = UpdateWorker::spawn(null_hwnd(), 0x8000);
    worker.shutdown();
}

#[test]
fn request_after_shutdown_reports_worker_unavailable() {
    let worker = UpdateWorker::spawn(null_hwnd(), 0x8000);
    worker.shutdown();

    let result = worker.request(UpdateRequest::Check {
        current: Version::current(),
    });
    assert!(matches!(result, Err(UpdateRequestError::WorkerUnavailable)));
}

#[test]
fn drain_results_is_empty_when_nothing_has_completed_yet() {
    let worker = UpdateWorker::spawn(null_hwnd(), 0x8000);
    assert!(worker.drain_results().is_empty());
    worker.shutdown();
}
