use super::*;
use std::fs::OpenOptions;
use std::time::Instant;
use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
use windows_sys::Win32::System::Threading::WaitForSingleObject;

/// 접속자가 없으면 세션이 영원히 붙어 있지 않고 ConnectTimeout으로 끝난다.
#[test]
fn wait_connect_times_out_without_client() {
    // 실제 pid와의 충돌을 피하기 위한 임의 식별자다. 파이프 이름은 단순
    // 텍스트일 뿐이라 어떤 값이든 된다.
    let server = PipeServer::create(0x5EED_C0DE).expect("파이프 생성");
    let started = Instant::now();
    let result = server.wait_connect_within(Duration::from_millis(300));
    assert!(
        matches!(result, Err(PipeError::ConnectTimeout)),
        "ConnectTimeout을 기대했는데 {result:?}"
    );
    // 2회 접속 대기(300ms씩) + 취소 정리 여유분.
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// 클라이언트가 양쪽 파이프에 접속하면 wait_connect는 성공한다.
/// 서버 ConnectNamedPipe보다 먼저 열리는 경합도 ERROR_PIPE_CONNECTED로
/// 흡수되는지 함께 확인한다.
#[test]
fn wait_connect_accepts_clients_on_both_pipes() {
    let pid = 0x5EED_C0DF;
    let server = PipeServer::create(pid).expect("파이프 생성");

    // HOOK_PIPE는 inbound 서버라 클라이언트는 쓰기 전용, HOST_PIPE는
    // outbound 서버라 읽기 전용으로 연다.
    let clients = std::thread::spawn(move || {
        for (prefix, write) in [(HOOK_PIPE, true), (HOST_PIPE, false)] {
            // 상수 자체에 \\.\pipe\ 프리픽스가 포함돼 있다.
            let name = format!("{prefix}{pid}");
            let mut options = OpenOptions::new();
            let file = loop {
                let opened = if write {
                    options.read(false).write(true).open(&name)
                } else {
                    options.read(true).write(false).open(&name)
                };
                match opened {
                    Ok(file) => break file,
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            };
            std::mem::forget(file);
        }
    });

    server
        .wait_connect_within(Duration::from_secs(5))
        .expect("양쪽 파이프 모두 접속돼야 한다");
    clients.join().expect("클라이언트 스레드");
}

/// 뒤늦게 도착한 DLL도 `LUNA_PIPE_AVAILABLE{pid}` 신호를 받는다.
///
/// 실제 DLL의 파이프 스레드는 loader lock 때문에 LoadLibraryW가 반환된
/// 뒤에야 CreateEventW에 도달하므로, 거의 항상 호스트보다 늦다. 그래서
/// 여기서도 `PipeServer::create`가 이벤트를 만들어 신호한 **다음에야**
/// 같은 이름을 연다. create가 신호 직후 핸들을 닫아버리면 명명 오브젝트가
/// 참조 카운트 0으로 파괴되고, 아래 CreateEventW는 signaled가 아닌 새
/// 오브젝트를 얻어 영원히 깨어나지 못한다.
#[test]
fn late_client_still_observes_pipe_available_signal() {
    let pid = 0x5EED_C0E0;
    let server = PipeServer::create(pid).expect("파이프 생성");

    // create가 이미 SetEvent까지 끝낸 뒤에 여는 것이 이 테스트의 핵심이다.
    let name = pipe_name(PIPE_AVAILABLE_EVENT, pid);
    // SAFETY: name은 NUL로 끝나는 UTF-16 버퍼다. 기존 오브젝트가 살아
    // 있으면 그것을 열고, 없으면 새로 만든다 — 후자가 곧 회귀 상황이다.
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) };
    assert!(!event.is_null(), "이벤트를 열지 못했다");

    // SAFETY: 방금 얻은 유효한 이벤트 핸들이다.
    let wait = unsafe { WaitForSingleObject(event, 500) };
    // SAFETY: 동일 핸들을 한 번만 닫는다. 단언 전에 닫아야 실패해도 샌다.
    unsafe {
        let _ = CloseHandle(event);
    }
    assert_eq!(
        wait, WAIT_OBJECT_0,
        "create가 이벤트 핸들을 세션 내내 열어 두지 않아 신호가 유실됐다"
    );

    // 대기 동안 서버가 살아 있어야 한다는 계약을 명시한다.
    drop(server);
}
