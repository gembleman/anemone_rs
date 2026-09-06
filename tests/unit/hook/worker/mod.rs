//! `HookWorker`/`SessionShared`/`stop_session` 등 워커 수명 관리 로직.
//!
//! `HookRequest::Attach`의 실제 세션 스레드(`session::run_session`)는 인젝션이
//! 성공해야 파이프까지 도달하므로, 여기서는 인젝션 없이 즉시 실패하는 pid
//! (`u32::MAX`)로도 `worker_thread`의 세션 교체/정리 로직을 실제로 왕복시킨다.
//! `worker_thread`를 직접 호출(스폰하지 않고)하면 채널을 미리 채운 뒤 sender를
//! 닫는 것만으로 결정론적으로 처리되므로 sleep 폴링이 필요 없다.

use std::sync::Mutex;
use std::sync::mpsc;

use windows_sys::Win32::Foundation::HWND;

use super::*;

fn null_hwnd() -> HWND {
    std::ptr::null_mut()
}

fn events_slot() -> EventSlot {
    Arc::new(Mutex::new(Vec::new()))
}

// --- SessionShared: 상태 전이 -----------------------------------------------

#[test]
fn a_fresh_session_shared_is_not_stopped_and_has_no_server() {
    let shared = SessionShared::new();
    assert!(!shared.is_stopped());
    assert!(shared.server_slot().is_none());
}

#[test]
fn server_slot_round_trips_a_stored_server() {
    let pid = 0x5EED_3000;
    let shared = SessionShared::new();
    let server = Arc::new(pipe_client::PipeServer::create(pid).expect("파이프 생성"));
    *shared.server_slot() = Some(Arc::clone(&server));
    assert!(shared.server_slot().is_some());
    assert_eq!(shared.server_slot().as_ref().unwrap().pid(), pid);
}

// --- push_event / drain_events ----------------------------------------------

#[test]
fn drain_events_preserves_arrival_order_and_empties_the_slot() {
    let events = events_slot();
    push_event(&events, 0, 0, HookEvent::EngineDetected("A".to_string()));
    push_event(&events, 0, 0, HookEvent::EngineDetected("B".to_string()));

    let worker = HookWorker {
        sender: Mutex::new(None),
        handle: Mutex::new(None),
        events: events.clone(),
    };
    let drained = worker.drain_events();
    assert_eq!(drained.len(), 2);
    match (&drained[0], &drained[1]) {
        (HookEvent::EngineDetected(a), HookEvent::EngineDetected(b)) => {
            assert_eq!(a, "A");
            assert_eq!(b, "B");
        }
        other => panic!("unexpected events: {other:?}"),
    }
    assert!(
        worker.drain_events().is_empty(),
        "drain 이후에는 슬롯이 비어 있어야 한다"
    );
}

// --- HookWorker: 공개 API 수명 관리 -----------------------------------------

#[test]
fn shutdown_without_any_request_does_not_hang() {
    let worker = HookWorker::spawn(null_hwnd(), 0x9000);
    worker.shutdown();
}

#[test]
fn request_after_shutdown_reports_unavailable() {
    let worker = HookWorker::spawn(null_hwnd(), 0x9000);
    worker.shutdown();
    assert!(worker.request(HookRequest::Detach).is_err());
}

#[test]
fn drain_events_is_empty_when_nothing_has_completed_yet() {
    let worker = HookWorker::spawn(null_hwnd(), 0x9000);
    assert!(worker.drain_events().is_empty());
    worker.shutdown();
}

/// Detach 요청이 세션 없이 도착해도 패닉하지 않는다 — 실제 스레드/채널을
/// 거쳐 `shutdown()`이 내장한 제한 시간 join으로 결정론적으로 끝난다.
#[test]
fn detaching_with_no_active_session_is_a_safe_no_op() {
    let worker = HookWorker::spawn(null_hwnd(), 0x9000);
    worker
        .request(HookRequest::Detach)
        .expect("살아있는 워커는 요청을 받아야 한다");
    worker.shutdown();
    assert!(worker.drain_events().is_empty());
}

// --- worker_thread: 세션 수명 왕복 (인젝션 없이) -----------------------------
//
// `worker_thread`를 직접(스레드를 스폰하지 않고) 호출한다. 요청을 채널에 먼저
// 다 채운 뒤 sender를 닫으면 `rx.recv()` 루프가 남은 메시지를 전부 처리하고
// 채널이 빈 뒤에 종료하므로, 실행 순서가 sleep 없이도 결정론적이다.

fn run_worker_thread_with(requests: Vec<HookRequest>) -> Vec<HookEvent> {
    let (tx, rx) = mpsc::channel::<HookRequest>();
    for request in requests {
        tx.send(request).expect("채널 전송");
    }
    drop(tx);
    let events = events_slot();
    HookWorker::worker_thread(rx, events.clone(), 0, 0);
    std::mem::take(&mut *events.lock().unwrap())
}

/// u32::MAX는 유효한 pid일 수 없으므로 `inject::detect_arch`가 즉시 실패해
/// 실제 인젝션은 시도되지 않는다. Attach 실패 뒤에도 세션 슬롯은 (이미 끝난
/// 스레드 핸들과 함께) 남아 있다가, 다음 Detach가 이를 정리하고
/// `Detached{by_user:true}`를 보고해야 한다.
#[test]
fn a_failed_attach_followed_by_detach_reports_attach_failed_then_detached() {
    let events = run_worker_thread_with(vec![
        HookRequest::Attach {
            pid: u32::MAX,
            process_name: "ghost.exe".to_string(),
        },
        HookRequest::Detach,
    ]);

    assert_eq!(events.len(), 2, "이벤트: {events:?}");
    match &events[0] {
        HookEvent::AttachFailed { pid, .. } => assert_eq!(*pid, u32::MAX),
        other => panic!("expected AttachFailed, got {other:?}"),
    }
    match &events[1] {
        HookEvent::Detached { pid, by_user } => {
            assert_eq!(*pid, u32::MAX);
            assert!(*by_user);
        }
        other => panic!("expected Detached, got {other:?}"),
    }
}

/// 단일 게임 정책: 두 번째 Attach는 첫 번째 세션을 먼저 정리하고
/// `Detached{by_user:true}`를 보고한 뒤에야 새 세션을 시작한다.
#[test]
fn a_second_attach_replaces_the_first_session() {
    let events = run_worker_thread_with(vec![
        HookRequest::Attach {
            pid: u32::MAX,
            process_name: "ghost-a.exe".to_string(),
        },
        HookRequest::Attach {
            pid: std::process::id(),
            process_name: "ghost-b.exe".to_string(),
        },
        HookRequest::Detach,
    ]);

    assert_eq!(events.len(), 4, "이벤트: {events:?}");
    assert!(matches!(
        &events[0],
        HookEvent::AttachFailed { pid, .. } if *pid == u32::MAX
    ));
    assert!(matches!(
        &events[1],
        HookEvent::Detached { pid, by_user: true } if *pid == u32::MAX
    ));
    assert!(matches!(
        &events[2],
        HookEvent::AttachFailed { pid, .. } if *pid == std::process::id()
    ));
    assert!(matches!(
        &events[3],
        HookEvent::Detached { pid, by_user: true } if *pid == std::process::id()
    ));
}

/// NewHook/RemoveHook/FindHook 명령은 활성 세션이 없으면 조용히 무시된다.
#[test]
fn hook_commands_with_no_active_session_do_not_panic() {
    let events = run_worker_thread_with(vec![
        HookRequest::RemoveHook(0x1234),
        HookRequest::FindHook(Box::new(pipe_client::build_general_search_param())),
    ]);
    assert!(events.is_empty());
}

// --- stop_session / send_command: 실제 파이프 연동 --------------------------

#[test]
fn stop_session_with_no_connected_server_returns_promptly() {
    let pid = 0x5EED_3001;
    let server = Arc::new(pipe_client::PipeServer::create(pid).expect("파이프 생성"));
    let shared = Arc::new(SessionShared::new());
    *shared.server_slot() = Some(server);
    // connected는 기본값 false다 — cancel_pending 경로를 타야 한다.

    let started = std::time::Instant::now();
    stop_session(&shared);
    // cancel_pending은 접속 대기 중인 파이프에도 안전한 즉시 취소이므로,
    // shutdown()의 500ms 명령 타임아웃보다 훨씬 빨리 끝나야 한다(회귀 감시용
    // 여유 상한 — server.rs의 기존 타임아웃 테스트와 같은 관례).
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

/// `connected == true`인 세션을 멈추면 `PipeServer::shutdown()`이 실제로 DETACH
/// 명령을 HOST_PIPE로 보낸다 — DLL이 이 프레임을 받아야 접속 하나만 닫고 다음
/// attach를 위해 이벤트 대기로 돌아간다(`session.rs`의 shutdown 문서 참고).
#[test]
fn stop_session_on_a_connected_server_sends_a_detach_command() {
    use std::io::Read;

    let pid = 0x5EED_3002;
    let server = Arc::new(pipe_client::PipeServer::create(pid).expect("파이프 생성"));

    let client = std::thread::spawn(move || {
        use std::fs::OpenOptions;
        use std::time::Duration;

        let mut hook_writer = None;
        let mut host_reader = None;
        while hook_writer.is_none() || host_reader.is_none() {
            if hook_writer.is_none() {
                hook_writer = OpenOptions::new()
                    .write(true)
                    .open(format!("{}{pid}", lunahook_rs::protocol::HOOK_PIPE))
                    .ok();
            }
            if host_reader.is_none() {
                host_reader = OpenOptions::new()
                    .read(true)
                    .open(format!("{}{pid}", lunahook_rs::protocol::HOST_PIPE))
                    .ok();
            }
            if hook_writer.is_none() || host_reader.is_none() {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let _hook_writer = hook_writer.unwrap();
        let mut host_reader = host_reader.unwrap();

        let mut frame = [0u8; 8];
        host_reader
            .read_exact(&mut frame)
            .expect("DETACH 프레임을 읽어야 한다");
        frame
    });

    server.wait_connect().expect("접속은 성공해야 한다");
    let shared = Arc::new(SessionShared::new());
    *shared.server_slot() = Some(Arc::clone(&server));
    shared.connected.store(true, Ordering::Release);

    stop_session(&shared);

    let frame = client.join().expect("클라이언트 스레드");
    let id = u32::from_le_bytes(frame[0..4].try_into().unwrap());
    let payload_size = u32::from_le_bytes(frame[4..8].try_into().unwrap());
    assert_eq!(id, lunahook_rs::protocol::rpc_id::DETACH);
    assert_eq!(payload_size, 0);
}

#[test]
fn send_command_with_no_active_session_is_a_safe_no_op() {
    let session: Option<super::ActiveSession> = None;
    send_command(&session, vec![1, 2, 3]);
}
