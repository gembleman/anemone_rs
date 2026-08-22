//! 후킹 워커 스레드 — 요청 채널, 세션 수명, 이벤트 슬롯.
//!
//! `src/update/worker.rs`와 동일한 UI-스레드 무블로킹 모델이다. attach부터
//! 알림 읽기 루프까지 전부 세션 스레드가 담당하므로, 워커 스레드는 언제나
//! 새 요청을 받을 수 있다. 단일 게임 정책에 따라 동시에 최대 한 세션만
//! 살아 있고, 새 attach는 기존 세션 정리 후 시작된다.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use lunahook_rs::params::RawHookParam;
use lunahook_rs::protocol::{MESSAGE_SIZE, SearchParam, TEXT_BUFFER_SIZE};

use super::inject;
use super::pipe_client::{self, FoundHook, Notification, PipeError, PipeServer};
use super::text_bridge::{self, DecodeError, HookText};
use super::{Arch, dll_path, inject32_helper_path};

/// UI → 워커 요청.
pub enum HookRequest {
    /// 게임 프로세스에 붙는다. 기존 세션이 있으면 먼저 끝낸다.
    Attach {
        pid: u32,
        process_name: String,
    },
    /// 현재 세션을 끊는다.
    Detach,
    /// 후보 후크 설치 (자동 발견 결과 또는 GDI 폴백). wire 구조체가 크다(~1KB).
    NewHook(Box<RawHookParam>),
    RemoveHook(u64),
    /// 자동 탐색(`text` 비움) 또는 텍스트 검색. wire 구조체가 크다(~1.6KB).
    FindHook(Box<SearchParam>),
}

/// 워커 → UI 이벤트. 공유 슬롯에 쌓였다가 WM_APP 메시지로 깨어난
/// UI 스레드가 꺼내 간다.
#[derive(Debug)]
pub enum HookEvent {
    Attached {
        pid: u32,
        arch: Arch,
        process_name: String,
    },
    AttachFailed {
        pid: u32,
        error: String,
    },
    Detached {
        pid: u32,
        /// 사용자가 중지했는지(true) 연결이 끊겼는지(false).
        by_user: bool,
    },
    Text(HookText),
    FoundHook(FoundHook),
    /// DLL 로그/경고 (디버깅용).
    Info(String),
}

type EventSlot = Arc<Mutex<Vec<HookEvent>>>;

struct SessionShared {
    server: Mutex<Option<Arc<PipeServer>>>,
    stop: AtomicBool,
}

impl SessionShared {
    fn new() -> Self {
        Self {
            server: Mutex::new(None),
            stop: AtomicBool::new(false),
        }
    }

    fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

/// 후킹 워커와 그 수명을 소유하는 핸들. UI 스레드가 절대 블로킹하지 않는다.
pub struct HookWorker {
    sender: Mutex<Option<Sender<HookRequest>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    events: EventSlot,
}

impl HookWorker {
    /// 워커 스레드를 띄운다. `message`는 이벤트 도착을 알리는 WM_APP 계열
    /// 메시지다(HWND는 !Send라 usize로 경계를 넘는다 — update 워커와 동일).
    pub fn spawn(hwnd: HWND, message: u32) -> Self {
        let (tx, rx) = mpsc::channel::<HookRequest>();
        let events: EventSlot = Arc::new(Mutex::new(Vec::new()));
        let worker_events = Arc::clone(&events);
        let hwnd_raw = hwnd.0 as usize;

        let handle = std::thread::Builder::new()
            .name("anemone-hook".to_string())
            .spawn(move || Self::worker_thread(rx, worker_events, hwnd_raw, message))
            .map_err(|error| tracing::error!("후킹 워커 스레드를 시작하지 못했습니다: {error}"))
            .ok();

        Self {
            sender: Mutex::new(Some(tx)),
            handle: Mutex::new(handle),
            events,
        }
    }

    /// 요청을 큐에 넣고 즉시 반환한다. 워커가 이미 종료됐으면 Err.
    pub fn request(&self, request: HookRequest) -> Result<(), ()> {
        let sender = self.sender.lock().expect("hook sender poisoned");
        match sender.as_ref() {
            Some(tx) => tx.send(request).map_err(|_| ()),
            None => Err(()),
        }
    }

    /// 도착한 이벤트를 모두 꺼낸다. 도착 순서를 보존한다.
    pub fn drain_events(&self) -> Vec<HookEvent> {
        let mut events = self.events.lock().expect("hook events poisoned");
        std::mem::take(&mut *events)
    }

    /// 채널을 닫고 세션·워커를 제한 시간 안에 join한다. 못 끝내면 detach하고
    /// 넘어간다 — 앱 종료가 멈추지 않게 하는 update 워커와 같은 정책이다.
    /// 남은 스레드는 프로세스 종료 시 OS가 회수한다.
    pub fn shutdown(&self) {
        {
            let mut sender = self.sender.lock().expect("hook sender poisoned");
            *sender = None;
        }
        let handle = self.handle.lock().expect("hook handle poisoned").take();
        if let Some(handle) = handle {
            let start = std::time::Instant::now();
            const MAX_WAIT: Duration = Duration::from_millis(2000);
            while !handle.is_finished() && start.elapsed() < MAX_WAIT {
                std::thread::sleep(Duration::from_millis(10));
            }
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                tracing::warn!("hook worker did not finish in {MAX_WAIT:?}, leaving detached");
            }
        }
    }

    fn push_event(events: &EventSlot, hwnd_raw: usize, message: u32, event: HookEvent) {
        events.lock().expect("hook events poisoned").push(event);
        notify(hwnd_raw, message);
    }

    fn worker_thread(rx: Receiver<HookRequest>, events: EventSlot, hwnd_raw: usize, message: u32) {
        // 현재 세션의 shared 상태. attach 사이에 교체된다.
        let mut session: Option<(u32, Arc<SessionShared>)> = None;

        while let Ok(request) = rx.recv() {
            match request {
                HookRequest::Attach { pid, process_name } => {
                    // 단일 게임 정책: 기존 세션을 먼저 끝낸다.
                    if let Some((old_pid, old_shared)) = session.take() {
                        stop_session(&old_shared);
                        emit_detached(&events, old_pid, true);
                    }
                    let shared = Arc::new(SessionShared::new());
                    session = Some((pid, Arc::clone(&shared)));
                    Self::run_session(pid, process_name, &shared, &events, hwnd_raw, message);
                }
                HookRequest::Detach => {
                    if let Some((pid, shared)) = session.take() {
                        stop_session(&shared);
                        emit_detached(&events, pid, true);
                    }
                }
                HookRequest::NewHook(hp) => {
                    send_command(&session, pipe_client::build_new_hook(&hp));
                }
                HookRequest::RemoveHook(address) => {
                    send_command(&session, pipe_client::build_remove_hook(address));
                }
                HookRequest::FindHook(sp) => {
                    send_command(&session, pipe_client::build_find_hook(&sp));
                }
            }
        }
        // 채널 종료(shutdown): 마지막 세션도 정리한다.
        if let Some((_, shared)) = session.take() {
            stop_session(&shared);
        }
    }

    /// attach → 파이프 핸드셰이크 → 알림 읽기 루프. 세션 스레드 본체다.
    ///
    /// 모든 실패는 AttachFailed/Detached 이벤트로 변환되며 반환한다.
    fn run_session(
        pid: u32,
        process_name: String,
        shared: &Arc<SessionShared>,
        events: &EventSlot,
        hwnd_raw: usize,
        message: u32,
    ) {
        let fail = |error: String| {
            Self::push_event(
                events,
                hwnd_raw,
                message,
                HookEvent::AttachFailed { pid, error },
            );
        };

        // 1. 비트니스 판별 + DLL 준비 확인.
        // SAFETY: pid는 사용자가 방금 고른 살아 있는 프로세스 식별자다.
        let arch = match unsafe { inject::detect_arch(pid) } {
            Ok(arch) => arch,
            Err(error) => {
                fail(error.to_string());
                return;
            }
        };
        let dll = dll_path(arch);
        if !dll.is_file() {
            fail(format!("후킹 DLL이 없습니다: {}", dll.display()));
            return;
        }

        // 2. 인젝션.
        let injected = match arch {
            Arch::X64 =>
            // SAFETY: 위 detect_arch가 성공한 살아 있는 대상이다.
            unsafe { inject::inject_same_bitness(pid, &dll) },
            Arch::X86 => inject::inject_via_helper(&inject32_helper_path(), pid, &dll),
        };
        if let Err(error) = injected {
            fail(error.to_string());
            return;
        }

        // 3. 파이프 서버 생성 + 접속 대기 + 핸드셰이크.
        let server = match PipeServer::create(pid).and_then(|server| {
            server.wait_connect()?;
            Ok(server)
        }) {
            Ok(server) => Arc::new(server),
            Err(error @ PipeError::Create(_)) => {
                fail(error.to_string());
                return;
            }
            Err(PipeError::ConnectTimeout) => {
                // DLL이 죽었거나 초기화에 실패한 상태다. 영원히 기다리지 않고
                // 여기서 실패 처리한다.
                fail(
                    "게임이 후킹 DLL에 접속하지 않았습니다 (접속 대기 제한 시간 초과)".to_string(),
                );
                return;
            }
            Err(_) => {
                // DLL이 접속하기 전에 죽었거나 detach됐다.
                fail("게임이 후킹 DLL에 접속하지 않았습니다".to_string());
                return;
            }
        };
        *shared.server.lock().expect("server slot") = Some(Arc::clone(&server));

        let cwd = match server.handshake() {
            Ok(cwd) => cwd,
            Err(error) => {
                if !shared.is_stopped() {
                    fail(error.to_string());
                }
                return;
            }
        };
        tracing::info!(pid, ?arch, %cwd, "후킹 세션 연결됨");
        debug_assert_eq!(server.pid(), pid);

        Self::push_event(
            events,
            hwnd_raw,
            message,
            HookEvent::Attached {
                pid,
                arch,
                process_name,
            },
        );

        // 4. GDI 폴백 후크 — 엔진 탐지 없이도 많은 게임의 텍스트가 잡힌다.
        let gdi = pipe_client::build_insert_pc_hooks(0);
        if let Err(error) = server.write_command(&gdi) {
            tracing::warn!("GDI 폴백 후크 설치 명령 실패: {error}");
        }

        // 5. 알림 읽기 루프. ReadFile 실패 = 연결 끊김 또는 cancel.
        let mut buffer = vec![0u8; TEXT_BUFFER_SIZE + MAX_NOTIFICATION_OVERHEAD];
        loop {
            if shared.is_stopped() {
                break;
            }
            let Some(notification) = server.read_notification(&mut buffer) else {
                break;
            };
            match notification {
                Notification::Text(text) => {
                    tracing::trace!(
                        pid = text.process_id,
                        address = text.hook_address,
                        name = %text.hook_name,
                        ctx2 = text.thread_ctx2,
                        "텍스트 수신"
                    );
                    match text_bridge::decode_payload(text.hook_type_flags, 0, &text.payload) {
                        Ok(decoded) if decoded.is_empty() => {}
                        Ok(decoded) => Self::push_event(
                            events,
                            hwnd_raw,
                            message,
                            HookEvent::Text(HookText {
                                source: (text.thread_addr, text.thread_ctx),
                                text: decoded,
                            }),
                        ),
                        Err(DecodeError::NotString) => tracing::trace!(
                            address = text.hook_address,
                            "문자열이 아닌 payload 무시"
                        ),
                    }
                }
                Notification::FoundHook(found) => {
                    tracing::info!(
                        address = found.hook_address,
                        flags = found.hook_type_flags,
                        text = %found.text,
                        "후보 후크 발견"
                    );
                    Self::push_event(events, hwnd_raw, message, HookEvent::FoundHook(found));
                }
                Notification::Info {
                    warning,
                    message: info_message,
                } => {
                    if warning {
                        tracing::warn!(pid, "DLL 경고: {info_message}");
                    } else {
                        tracing::info!(pid, "DLL: {info_message}");
                    }
                    Self::push_event(events, hwnd_raw, message, HookEvent::Info(info_message));
                }
                Notification::Removed(address) => {
                    tracing::debug!(address, "후크 제거 통지")
                }
                Notification::Inserting { address } => {
                    tracing::debug!(address, "후크 설치 진행")
                }
                Notification::Ignored(kind) => tracing::trace!(kind, "무시된 알림"),
            }
        }

        // 6. 정리. 사용자 중지가 아니면 연결 끊김으로 보고한다.
        let by_user = shared.is_stopped();
        drop(server);
        *shared.server.lock().expect("server slot") = None;
        if !by_user {
            emit_detached(events, pid, false);
        }
    }
}

/// TextOutput_T(+payload) 외 알림(FoundHook 등 고정 크기 구조체)을 담기 위한 여유분.
const MAX_NOTIFICATION_OVERHEAD: usize = MESSAGE_SIZE * 2;

fn emit_detached(events: &EventSlot, pid: u32, by_user: bool) {
    events
        .lock()
        .expect("hook events poisoned")
        .push(HookEvent::Detached { pid, by_user });
}

/// 세션을 안전하게 끝낸다: stop 플래그 → DETACH/CancelIoEx로 대기 중인
/// 파이프 I/O를 깬다.
fn stop_session(shared: &Arc<SessionShared>) {
    shared.stop.store(true, Ordering::SeqCst);
    if let Some(server) = shared.server.lock().expect("server slot").as_ref() {
        server.shutdown();
    }
}

/// 설치/제거/탐색 명령을 현재 세션으로 보낸다. 세션이 없으면 조용히 무시한다.
fn send_command(session: &Option<(u32, Arc<SessionShared>)>, bytes: Vec<u8>) {
    let Some((_, shared)) = session else {
        return;
    };
    let guard = shared.server.lock().expect("server slot");
    if let Some(server) = guard.as_ref()
        && let Err(error) = server.write_command(&bytes)
    {
        tracing::warn!("후킹 명령 전송 실패: {error}");
    }
}

/// 이벤트가 준비됐음을 UI 스레드에 알린다.
fn notify(hwnd_raw: usize, message: u32) {
    let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
    // SAFETY: UI 스레드가 소유한 창은 App이 종료될 때까지 유효하다. shutdown은
    // App 소멸 경로에서 호출되므로 그 뒤에는 새 알림이 없다.
    let result = unsafe { PostMessageW(Some(hwnd), message, WPARAM(0), LPARAM(0)) };
    if let Err(error) = result {
        tracing::warn!("후킹 이벤트 알림을 게시하지 못했습니다: {error}");
    }
}
