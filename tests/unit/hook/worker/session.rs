//! `session::run_session` 전체는 실제 DLL 인젝션이 성공해야 파이프 핸드셰이크
//! 단계까지 도달한다. 이 파일은 그 대신
//!
//! 1. 인젝션 없이도 자연스럽게 도달하는 실패 경로(`attach_and_handshake`의
//!    비트니스 판별/DLL 존재 확인 단계)를 실제 `run_session` 호출로 검증하고,
//! 2. 알림 읽기 루프(`run_notification_loop`)와 알림 분기(`handle_notification`/
//!    `handle_text_notification`)는 실제 named pipe를 세워 프로토콜 왕복으로
//!    검증한다(`hook/pipe_client/server.rs`의 기존 테스트와 같은 패턴).
//!
//! `attach_and_handshake`가 실제 인젝션(`inject_target`)을 호출한 뒤의 경로
//! (ConnectTimeout, handshake 실패)는 살아 있는 게임 프로세스가 있어야만
//! 재현되므로 여기서 억지로 덮지 않는다.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use lunahook_rs::params::{HookType, RawHookParam};
use lunahook_rs::protocol::{HOOK_PIPE, HOST_PIPE};

use super::*;
use crate::hook::pipe_client::FoundHook;

fn events_slot() -> EventSlot {
    Arc::new(std::sync::Mutex::new(Vec::new()))
}

fn drain(events: &EventSlot) -> Vec<HookEvent> {
    std::mem::take(&mut *events.lock().unwrap())
}

/// 텍스트가 아닌 알림은 선행 바이트를 물지 않으므로 매번 새 상태로 부른다.
fn notify(notification: Notification, events: &EventSlot) {
    handle_notification(1, notification, &mut LeadBytes::default(), events, 0, 0);
}

// --- handle_notification: 알림 종류별 분기 ----------------------------------

#[test]
fn engine_detected_notification_is_forwarded_as_an_event() {
    let events = events_slot();
    notify(
        Notification::EngineDetected("KiriKiri".to_string()),
        &events,
    );
    let drained = drain(&events);
    assert_eq!(drained.len(), 1);
    match &drained[0] {
        HookEvent::EngineDetected(name) => assert_eq!(name, "KiriKiri"),
        other => panic!("expected EngineDetected, got {other:?}"),
    }
}

#[test]
fn found_hook_notification_is_forwarded_as_an_event() {
    let events = events_slot();
    let found = FoundHook {
        hook_type_flags: HookType::USING_STRING.bits(),
        hook_address: 0x1000,
        hook_param: RawHookParam::default(),
        text: "candidate".to_string(),
    };
    notify(Notification::FoundHook(Box::new(found)), &events);
    let drained = drain(&events);
    assert_eq!(drained.len(), 1);
    match &drained[0] {
        HookEvent::FoundHook(found) => assert_eq!(found.text, "candidate"),
        other => panic!("expected FoundHook, got {other:?}"),
    }
}

#[test]
fn info_notifications_carry_the_message_regardless_of_warning_flag() {
    let events = events_slot();
    notify(
        Notification::Info {
            warning: true,
            message: "danger".to_string(),
        },
        &events,
    );
    notify(
        Notification::Info {
            warning: false,
            message: "info".to_string(),
        },
        &events,
    );
    let drained = drain(&events);
    assert_eq!(drained.len(), 2);
    for (event, expected) in drained.iter().zip(["danger", "info"]) {
        match event {
            HookEvent::Info(message) => assert_eq!(message, expected),
            other => panic!("expected Info, got {other:?}"),
        }
    }
}

/// Removed/Inserting/Ignored는 로그만 남기고 UI 이벤트를 만들지 않는다 —
/// 슬롯이 조용히 비어 있어야 한다.
#[test]
fn inserting_notification_preserves_the_reusable_hook_code() {
    let events = events_slot();
    notify(Notification::Removed(0x10), &events);
    notify(
        Notification::Inserting {
            address: 0x20,
            hook_code: "HQ0@20".into(),
        },
        &events,
    );
    notify(Notification::Ignored(99), &events);
    let drained = drain(&events);
    assert!(matches!(
        &drained[..],
        [HookEvent::HookInserted { address: 0x20, hook_code }] if hook_code == "HQ0@20"
    ));
}

// --- handle_text_notification: 디코딩 분기 ----------------------------------

fn text_notification(hook_type_flags: u64, payload: Vec<u8>) -> TextNotification {
    TextNotification {
        process_id: 7,
        thread_addr: 1,
        thread_ctx: 2,
        thread_ctx2: 3,
        hook_address: 0x1234,
        hook_type_flags,
        detected_codepage: 0,
        hook_name: "H1".to_string(),
        payload,
    }
}

#[test]
fn decodable_text_becomes_a_text_event_with_the_source_preserved() {
    let events = events_slot();
    let flags = (HookType::USING_STRING | HookType::CODEC_UTF8 | HookType::FULL_STRING).bits();
    let text = text_notification(flags, b"hello\0".to_vec());

    handle_text_notification(text, &mut LeadBytes::default(), &events, 0, 0);

    let drained = drain(&events);
    assert_eq!(drained.len(), 1);
    match &drained[0] {
        HookEvent::Text {
            text: hook_text, ..
        } => {
            assert_eq!(hook_text.text, "hello");
            assert_eq!(hook_text.hook_name, "H1");
            assert_eq!(hook_text.source.address, 1);
            assert_eq!(hook_text.source.context, 2);
            assert_eq!(hook_text.source.subcontext, 3);
            assert!(
                hook_text.full_string,
                "FULL_STRING 플래그가 전달되어야 한다"
            );
        }
        other => panic!("expected Text, got {other:?}"),
    }
}

/// USING_STRING이 없는 후크는 lunahook의 문자 단위 후크다(KiriKiri1/KiriKiri2).
/// 조각 하나하나가 이벤트로 올라와야 TextMerger가 문장으로 이을 수 있다.
#[test]
fn a_single_char_hook_type_still_produces_a_text_event() {
    let events = events_slot();
    let text = text_notification(
        HookType::CODEC_UTF16.bits(),
        0x3042u16.to_le_bytes().to_vec(),
    );
    handle_text_notification(text, &mut LeadBytes::default(), &events, 0, 0);

    let drained = drain(&events);
    assert_eq!(drained.len(), 1);
    match &drained[0] {
        HookEvent::Text {
            text: hook_text, ..
        } => {
            assert_eq!(hook_text.text, "あ");
            assert!(
                !hook_text.full_string,
                "문자 조각은 한 번에 문장을 내는 후크(FULL_STRING)가 아니다"
            );
        }
        other => panic!("expected Text, got {other:?}"),
    }
}

/// Waffle처럼 한 글자씩 내는 후크는 2바이트 글자를 알림 둘로 나눠 보낸다.
/// 선행 바이트는 이벤트를 만들지 않고 물고 있다가 다음 바이트와 짝을 짓는다.
#[test]
fn a_double_byte_character_split_across_notifications_becomes_one_event() {
    let events = events_slot();
    let mut lead_bytes = LeadBytes::default();
    let flags = HookType::USING_CHAR.bits();

    handle_text_notification(
        text_notification(flags, vec![0x82]),
        &mut lead_bytes,
        &events,
        0,
        0,
    );
    assert!(
        drain(&events).is_empty(),
        "선행 바이트만으로는 보여줄 글자가 없다"
    );

    handle_text_notification(
        text_notification(flags, vec![0xa0]),
        &mut lead_bytes,
        &events,
        0,
        0,
    );
    let drained = drain(&events);
    assert_eq!(drained.len(), 1);
    match &drained[0] {
        HookEvent::Text {
            text: hook_text, ..
        } => assert_eq!(hook_text.text, "あ"),
        other => panic!("expected Text, got {other:?}"),
    }
}

#[test]
fn an_empty_decoded_string_produces_no_text_event() {
    let events = events_slot();
    let flags = (HookType::USING_STRING | HookType::CODEC_UTF8).bits();
    let text = text_notification(flags, vec![0]);
    handle_text_notification(text, &mut LeadBytes::default(), &events, 0, 0);
    assert!(
        drain(&events).is_empty(),
        "빈 문자열은 UI에 보여줄 것이 없으므로 이벤트를 만들지 않는다"
    );
}

// --- run_notification_loop: 실제 named pipe 왕복 ----------------------------

/// DLL 역할을 하는 가짜 클라이언트. `hook/pipe_client/server.rs`의 기존 테스트와
/// 같은 패턴으로 두 파이프에 접속한다.
fn connect_fake_client(pid: u32) -> (std::fs::File, std::fs::File) {
    use std::fs::OpenOptions;
    use std::time::Duration;

    let mut hook_writer = None;
    let mut host_reader = None;
    while hook_writer.is_none() || host_reader.is_none() {
        if hook_writer.is_none() {
            hook_writer = OpenOptions::new()
                .write(true)
                .open(format!("{HOOK_PIPE}{pid}"))
                .ok();
        }
        if host_reader.is_none() {
            host_reader = OpenOptions::new()
                .read(true)
                .open(format!("{HOST_PIPE}{pid}"))
                .ok();
        }
        if hook_writer.is_none() || host_reader.is_none() {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    (hook_writer.unwrap(), host_reader.unwrap())
}

#[test]
fn run_notification_loop_dispatches_notifications_in_arrival_order_until_disconnect() {
    let pid = 0x5EED_2000;
    let server = PipeServer::create(pid).expect("파이프 생성");
    let shared = Arc::new(SessionShared::new());
    let events = events_slot();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();

    let client = std::thread::spawn(move || {
        let (mut hook_writer, _host_reader) = connect_fake_client(pid);
        // 서버의 wait_connect()가 이미 성공을 확인한 뒤에만 쓰고 끊는다 — 그렇지
        // 않으면 접속 완료 확인과 곧바로 이어지는 연결 끊김이 경합할 수 있다.
        let _ = ready_rx.recv();
        let engine =
            lunahook_rs::protocol::notify_engine_detected("KiriKiri").expect("engine frame");
        hook_writer.write_all(&engine).expect("엔진 알림 전송");
        let removed = lunahook_rs::protocol::notify_hook_removed(0x99).expect("removed frame");
        hook_writer.write_all(&removed).expect("제거 알림 전송");
        // 연결을 끊어 루프가 자연스럽게 끝나게 한다.
        drop(hook_writer);
    });

    server
        .wait_connect()
        .expect("클라이언트가 양쪽 파이프에 접속해야 한다");
    ready_tx
        .send(())
        .expect("클라이언트에 신호를 보낼 수 있어야 한다");
    run_notification_loop(pid, &server, &shared, &events, 0, 0);
    client.join().unwrap();

    let drained = drain(&events);
    // EngineDetected만 이벤트가 되고, Removed는 로그만 남긴다.
    assert_eq!(drained.len(), 1);
    match &drained[0] {
        HookEvent::EngineDetected(name) => assert_eq!(name, "KiriKiri"),
        other => panic!("expected EngineDetected, got {other:?}"),
    }
}

/// stop 플래그가 이미 서 있으면 파이프를 전혀 읽지 않고 즉시 반환해야 한다 —
/// 클라이언트가 붙어 있지 않아도(연결 대기 없이) 안전해야 한다.
#[test]
fn run_notification_loop_returns_immediately_when_already_stopped() {
    let pid = 0x5EED_2001;
    let server = PipeServer::create(pid).expect("파이프 생성");
    let shared = Arc::new(SessionShared::new());
    shared.stop.store(true, Ordering::SeqCst);
    let events = events_slot();

    // 접속한 클라이언트가 없어도 멈춰 있으면 절대 블로킹하지 않는다.
    run_notification_loop(pid, &server, &shared, &events, 0, 0);

    assert!(drain(&events).is_empty());
}

// --- run_session: 인젝션 이전에 자연스럽게 실패하는 경로 --------------------

fn dummy_shared() -> Arc<SessionShared> {
    Arc::new(SessionShared::new())
}

/// Windows pid는 4의 배수로 제한되므로 u32::MAX는 유효한 프로세스일 수 없다.
/// `inject::detect_arch`가 즉시 실패해 실제 인젝션은 전혀 시도하지 않는다.
#[test]
fn attaching_to_a_nonexistent_pid_reports_attach_failed_without_injecting() {
    let shared = dummy_shared();
    let events = events_slot();

    run_session(u32::MAX, "ghost.exe".to_string(), &shared, &events, 0, 0);

    let drained = drain(&events);
    assert_eq!(drained.len(), 1);
    match &drained[0] {
        HookEvent::AttachFailed { pid, error } => {
            assert_eq!(*pid, u32::MAX);
            assert!(!error.is_empty());
        }
        other => panic!("expected AttachFailed, got {other:?}"),
    }
}

/// 테스트 바이너리 옆에는 `hook/<arch>/lunahook_rs64.dll`이 없다
/// (`scripts/run_with_hooks.ps1`이 실제 anemone_rs.exe를 실행할 때만 배치한다).
/// 그래서 자기 자신의 pid로도 비트니스 판별(성공)까지는 가지만 DLL 존재
/// 확인에서 실패해, 실제 인젝션 없이 attach_and_handshake의 두 번째 갈림길을
/// 검증할 수 있다.
#[test]
fn attaching_to_self_without_a_bundled_dll_reports_a_missing_dll_message() {
    let shared = dummy_shared();
    let events = events_slot();

    run_session(
        std::process::id(),
        "self-test.exe".to_string(),
        &shared,
        &events,
        0,
        0,
    );

    let drained = drain(&events);
    assert_eq!(drained.len(), 1);
    match &drained[0] {
        HookEvent::AttachFailed { error, .. } => {
            assert!(
                error.contains("후킹 DLL이 없습니다"),
                "실제 메시지: {error}"
            );
        }
        other => panic!("expected AttachFailed, got {other:?}"),
    }
}

/// `SetupError::always`로 만든 오류는 세션이 이미 멈춘 상태였어도 절대
/// 억제되지 않는다 — `unless_stopped`만 억제 대상이다. 인젝션이 필요한
/// ConnectTimeout/handshake 실패(unless_stopped 경로)는 실제 게임 프로세스가
/// 있어야 재현되므로 이 테스트로 다루지 않는다.
#[test]
fn always_errors_are_reported_even_when_the_session_was_already_stopped() {
    let shared = dummy_shared();
    shared.stop.store(true, Ordering::SeqCst);
    let events = events_slot();

    run_session(
        std::process::id(),
        "self-test.exe".to_string(),
        &shared,
        &events,
        0,
        0,
    );

    assert_eq!(
        drain(&events).len(),
        1,
        "always 오류는 stop 여부와 무관하게 항상 보고되어야 한다"
    );
}
