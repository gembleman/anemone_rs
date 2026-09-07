//! 후킹 워커 스레드 — 요청 채널, 세션 수명, 이벤트 슬롯.
//!
//! `src/update/worker.rs`와 동일한 UI-스레드 무블로킹 모델이다. attach부터
//! 알림 읽기 루프까지 전부 세션 스레드가 담당하므로, 워커 스레드는 언제나
//! 새 요청을 받을 수 있다. 단일 게임 정책에 따라 동시에 최대 한 세션만
//! 살아 있고, 새 attach는 기존 세션 정리 후 시작된다.
//!
//! attach → 파이프 핸드셰이크 → 알림 읽기 루프 본체는 [`session`] 모듈이
//! 담당한다.

mod session;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

use lunahook_rs::params::RawHookParam;
use lunahook_rs::protocol::SearchParam;

use super::Arch;
use super::pipe_client::{self, FoundHook, PipeServer};
use super::text_bridge::HookText;

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
    FindHook {
        search: Box<SearchParam>,
        generation: u64,
    },
}

/// 워커 → UI 이벤트. 공유 슬롯에 쌓였다가 WM_APP 메시지로 깨어난
/// UI 스레드가 꺼내 간다.
#[derive(Debug)]
pub enum HookEvent {
    Attached {
        pid: u32,
        arch: Arch,
        process_name: String,
        /// 대상 실행 파일의 SHA-256(소문자 16진). 읽기 실패 시 `None`.
        exe_sha256: Option<String>,
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
    Text {
        text: HookText,
        received_at: std::time::Instant,
    },
    EngineDetected(String),
    FoundHook {
        found: Box<FoundHook>,
        generation: u64,
    },
    HookInserted {
        address: u64,
        hook_code: String,
    },
    /// DLL 로그/경고 (디버깅용).
    Info(String),
}

pub(super) type EventSlot = Arc<EventQueue>;

pub(super) struct EventQueue {
    queue: Mutex<Vec<HookEvent>>,
    notification_pending: AtomicBool,
}

impl EventQueue {
    fn new() -> Self {
        Self {
            queue: Mutex::new(Vec::new()),
            notification_pending: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<HookEvent>> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub(super) struct SessionShared {
    server: Mutex<Option<Arc<PipeServer>>>,
    stop: AtomicBool,
    connected: AtomicBool,
    /// FIND_HOOK 결과를 현재 검색 세대에 귀속한다. wire 알림에는 세대가
    /// 없으므로 세션이 명령을 받는 순간 갱신하고, 읽기 스레드가 이를 붙인다.
    search_generation: std::sync::atomic::AtomicU64,
}

impl SessionShared {
    fn new() -> Self {
        Self {
            server: Mutex::new(None),
            stop: AtomicBool::new(false),
            connected: AtomicBool::new(false),
            search_generation: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub(super) fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// 잠긴 서버 슬롯을 잡는다. 뮤텍스가 poison돼도(다른 스레드가 패닉한
    /// 채로 잠금을 들고 있었어도) 슬롯 내용 자체는 여전히 유효하므로 그대로
    /// 회복해 계속 쓴다.
    pub(super) fn server_slot(&self) -> std::sync::MutexGuard<'_, Option<Arc<PipeServer>>> {
        self.server
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

struct ActiveSession {
    pid: u32,
    shared: Arc<SessionShared>,
    handle: JoinHandle<()>,
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
        let events = Arc::new(EventQueue::new());
        let worker_events = Arc::clone(&events);
        let hwnd_raw = hwnd as usize;

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
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match sender.as_ref() {
            Some(tx) => tx.send(request).map_err(|_| ()),
            None => Err(()),
        }
    }

    /// 도착한 이벤트를 모두 꺼낸다. 도착 순서를 보존한다.
    #[allow(dead_code)]
    pub fn drain_events(&self) -> Vec<HookEvent> {
        self.drain_events_batch(usize::MAX)
    }

    /// 도착 순서를 유지한 채 최대 `limit`개만 꺼낸다. UI가 호출하는 경로는
    /// 반드시 유한한 limit을 사용해 워커 이벤트가 paint/input을 독점하지
    /// 않게 한다.
    pub fn drain_events_batch(&self, limit: usize) -> Vec<HookEvent> {
        let mut events = self.events.lock();
        if limit >= events.len() {
            let drained = std::mem::take(&mut *events);
            self.events
                .notification_pending
                .store(false, Ordering::Release);
            return drained;
        }
        events.drain(..limit).collect()
    }

    pub fn has_pending_events(&self) -> bool {
        !self.events.lock().is_empty()
    }

    /// 잔여 이벤트를 위한 알림을 다시 게시한다. 큐 잠금을 잡은 채 상태를
    /// 바꾸고 게시하므로 push와의 경쟁에서 알림 상태가 영구히 고착되지 않는다.
    pub(crate) fn notify_pending_events(&self, hwnd_raw: usize, message: u32) {
        let queue = self.events.lock();
        if queue.is_empty() {
            self.events
                .notification_pending
                .store(false, Ordering::Release);
            return;
        }
        self.events
            .notification_pending
            .store(true, Ordering::Release);
        if notify(hwnd_raw, message).is_err() {
            self.events
                .notification_pending
                .store(false, Ordering::Release);
        }
    }

    /// 채널을 닫고 세션·워커를 제한 시간 안에 join한다. 못 끝내면 detach하고
    /// 넘어간다 — 앱 종료가 멈추지 않게 하는 update 워커와 같은 정책이다.
    /// 남은 스레드는 프로세스 종료 시 OS가 회수한다.
    pub fn shutdown(&self) {
        {
            let mut sender = self
                .sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *sender = None;
        }
        let handle = self
            .handle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
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

    fn worker_thread(rx: Receiver<HookRequest>, events: EventSlot, hwnd_raw: usize, message: u32) {
        // 현재 세션의 shared 상태. attach 사이에 교체된다.
        let mut session: Option<ActiveSession> = None;

        while let Ok(request) = rx.recv() {
            match request {
                HookRequest::Attach { pid, process_name } => {
                    // 단일 게임 정책: 기존 세션을 먼저 끝낸다.
                    if let Some(old) = session.take() {
                        let old_pid = old.pid;
                        stop_and_join(old);
                        push_event(
                            &events,
                            hwnd_raw,
                            message,
                            HookEvent::Detached {
                                pid: old_pid,
                                by_user: true,
                            },
                        );
                    }
                    let shared = Arc::new(SessionShared::new());
                    let thread_shared = Arc::clone(&shared);
                    let thread_events = Arc::clone(&events);
                    let handle = std::thread::Builder::new()
                        .name(format!("anemone-hook-session-{pid}"))
                        .spawn(move || {
                            session::run_session(
                                pid,
                                process_name,
                                &thread_shared,
                                &thread_events,
                                hwnd_raw,
                                message,
                            )
                        });
                    match handle {
                        Ok(handle) => {
                            session = Some(ActiveSession {
                                pid,
                                shared,
                                handle,
                            })
                        }
                        Err(error) => push_event(
                            &events,
                            hwnd_raw,
                            message,
                            HookEvent::AttachFailed {
                                pid,
                                error: format!("후킹 세션 스레드를 시작하지 못했습니다: {error}"),
                            },
                        ),
                    }
                }
                HookRequest::Detach => {
                    if let Some(active) = session.take() {
                        let pid = active.pid;
                        stop_and_join(active);
                        push_event(
                            &events,
                            hwnd_raw,
                            message,
                            HookEvent::Detached { pid, by_user: true },
                        );
                    }
                }
                HookRequest::NewHook(hp) => {
                    send_command(&session, pipe_client::build_new_hook(&hp));
                }
                HookRequest::RemoveHook(address) => {
                    send_command(&session, pipe_client::build_remove_hook(address));
                }
                HookRequest::FindHook { search, generation } => {
                    if let Some(active) = session.as_ref() {
                        active
                            .shared
                            .search_generation
                            .store(generation, Ordering::Release);
                    }
                    send_command(&session, pipe_client::build_find_hook(&search));
                }
            }
        }
        // 채널 종료(shutdown): 마지막 세션도 정리한다.
        if let Some(active) = session.take() {
            stop_and_join(active);
        }
    }
}

/// 이벤트를 슬롯에 쌓고 UI 스레드를 깨운다.
pub(super) fn push_event(events: &EventSlot, hwnd_raw: usize, message: u32, event: HookEvent) {
    let mut queue = events.lock();
    // 빈 큐에서 비어 있지 않은 큐로 바뀔 때만 깨운다. UI가 한 번에
    // 제한된 수만 꺼내도 남은 이벤트가 다시 알림을 만들므로, 이벤트마다
    // PostMessageW를 호출해 메시지 큐를 폭발시키지 않는다.
    if !events.notification_pending.swap(true, Ordering::AcqRel) {
        queue.push(event);
        if notify(hwnd_raw, message).is_err() {
            // 게시 실패를 소비하지 않고 되돌린다. 다음 push가 다시 게시하고,
            // UI가 잔여 큐를 확인하는 경로도 재시도할 수 있다.
            events.notification_pending.store(false, Ordering::Release);
        }
    } else {
        queue.push(event);
    }
}

/// 세션을 안전하게 끝낸다: stop 플래그 → DETACH/CancelIoEx로 대기 중인
/// 파이프 I/O를 깬다.
fn stop_session(shared: &Arc<SessionShared>) {
    shared.stop.store(true, Ordering::SeqCst);
    if let Some(server) = shared.server_slot().as_ref() {
        if shared.connected.load(Ordering::Acquire) {
            server.shutdown();
        } else {
            server.cancel_pending();
        }
    }
}

fn stop_and_join(active: ActiveSession) {
    stop_session(&active.shared);
    let start = std::time::Instant::now();
    while !active.handle.is_finished() && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(10));
    }
    if active.handle.is_finished() {
        let _ = active.handle.join();
    } else {
        tracing::warn!(
            pid = active.pid,
            "hook session did not finish after stop request"
        );
    }
}

/// 설치/제거/탐색 명령을 현재 세션으로 보낸다. 세션이 없으면 조용히 무시한다.
fn send_command(session: &Option<ActiveSession>, bytes: Vec<u8>) {
    let Some(active) = session else {
        return;
    };
    let guard = active.shared.server_slot();
    if let Some(server) = guard.as_ref()
        && let Err(error) = server.write_command(&bytes)
    {
        tracing::warn!("후킹 명령 전송 실패: {error}");
    }
}

/// 이벤트가 준비됐음을 UI 스레드에 알린다.
fn notify(hwnd_raw: usize, message: u32) -> Result<(), ()> {
    // SAFETY: UI 스레드가 소유한 창은 App이 종료될 때까지 유효하다. shutdown은
    // App 소멸 경로에서 호출되므로 그 뒤에는 새 알림이 없다.
    let result = unsafe { PostMessageW(hwnd_raw as HWND, message, 0, 0) };
    if result == 0 {
        tracing::warn!("후킹 이벤트 알림을 게시하지 못했습니다");
        Err(())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/hook/worker/mod.rs"]
mod tests;
