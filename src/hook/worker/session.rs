//! 세션 스레드 본체: attach → 파이프 핸드셰이크 → 알림 읽기 루프.
//!
//! [`run_session`]은 실패 분기가 많아 원래 하나의 거대한 함수였다. 여기서는
//! 두 축으로 나눈다.
//!
//! - 인젝션 실패 정리는 [`InjectedGuard`]가 Drop으로 자동 처리한다. 성공
//!   경로에서만 [`InjectedGuard::disarm`]으로 소유권을 넘기므로, 나머지
//!   실패 반환은 별도 정리 코드 없이 스코프를 벗어나기만 하면 된다.
//! - attach~핸드셰이크는 [`attach_and_handshake`]로, 알림 읽기 루프는
//!   [`run_notification_loop`]/[`handle_notification`]으로 분리했다.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use lunahook_rs::params::HookType;
use lunahook_rs::protocol::{MESSAGE_SIZE, TEXT_BUFFER_SIZE};

use super::super::pipe_client::{Notification, PipeError, PipeServer, TextNotification};
use super::super::text_bridge::{HookSource, HookText, LeadBytes};
use super::super::{Arch, dll_path, inject, inject32_helper_path};
use super::{EventSlot, HookEvent, SessionShared, push_event};
use crate::logging::LUNAHOOK_TARGET;

/// TextOutput_T(+payload) 외 알림(FoundHook 등 고정 크기 구조체)을 담기 위한 여유분.
const MAX_NOTIFICATION_OVERHEAD: usize = MESSAGE_SIZE * 2;

/// 인젝션된 DLL을 스코프 종료 시 자동으로 제거하는 가드.
///
/// attach 실패 분기마다 `cleanup_injected()`를 직접 호출하던 방식을 Drop으로
/// 대체한다. 성공 경로에서만 [`disarm`](Self::disarm)으로 소유권을 넘기고,
/// 그 밖의 모든 실패 반환에서는 스코프 이탈 시 자동으로 uninject된다.
struct InjectedGuard {
    pid: u32,
    arch: Arch,
    module: Option<u64>,
}

impl InjectedGuard {
    fn armed(pid: u32, arch: Arch, module: u64) -> Self {
        Self {
            pid,
            arch,
            module: Some(module),
        }
    }

    /// 성공 경로 전용: 더 이상 자동으로 제거하지 않는다.
    fn disarm(mut self) {
        self.module = None;
    }
}

impl Drop for InjectedGuard {
    fn drop(&mut self) {
        let Some(module) = self.module.take() else {
            return;
        };
        let result = match self.arch {
            Arch::X64 => {
                // SAFETY: module은 이 attach에서 대상 프로세스의 LoadLibraryW가
                // 반환한 HMODULE이다. 실패 경로에서만 한 번 제거한다.
                unsafe { inject::uninject_same_bitness(self.pid, module) }
            }
            Arch::X86 => inject::uninject_via_helper(&inject32_helper_path(), self.pid, module),
        };
        if let Err(error) = result {
            tracing::warn!(
                pid = self.pid,
                "인젝션 실패 정리(uninject)에 실패했습니다: {error}"
            );
        }
    }
}

/// 세션 설정 실패 메시지와, 사용자 detach로 인한 실패라 AttachFailed 이벤트를
/// 억제해야 하는지 여부.
struct SetupError {
    message: String,
    suppress_if_stopped: bool,
}

impl SetupError {
    fn always(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            suppress_if_stopped: false,
        }
    }

    fn unless_stopped(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            suppress_if_stopped: true,
        }
    }
}

/// attach → 파이프 핸드셰이크 → 알림 읽기 루프. 세션 스레드 본체다.
///
/// 모든 실패는 AttachFailed/Detached 이벤트로 변환되며 반환한다.
pub(super) fn run_session(
    pid: u32,
    process_name: String,
    shared: &Arc<SessionShared>,
    events: &EventSlot,
    hwnd_raw: usize,
    message: u32,
) {
    tracing::info!(target: LUNAHOOK_TARGET, "attach 시작: pid={pid} process={process_name}");
    let (arch, server) = match attach_and_handshake(pid, shared) {
        Ok(value) => value,
        Err(error) => {
            tracing::info!(target: LUNAHOOK_TARGET, "attach 실패: pid={pid} {}", error.message);
            if !error.suppress_if_stopped || !shared.is_stopped() {
                push_event(
                    events,
                    hwnd_raw,
                    message,
                    HookEvent::AttachFailed {
                        pid,
                        error: error.message,
                    },
                );
            }
            return;
        }
    };

    // 대상 exe 지문. 파일 I/O가 섞이지만 attach 경로의 일부로 워커
    // 스레드에서 수행되므로 UI는 막히지 않는다.
    let exe_sha256 = super::super::compute_exe_digest(pid);
    push_event(
        events,
        hwnd_raw,
        message,
        HookEvent::Attached {
            pid,
            arch,
            process_name,
            exe_sha256,
        },
    );

    // 핸드셰이크 직후 DLL의 `hijack_on_connect`가 엔진별 후크를 먼저
    // 설치하고, 아무 엔진도 맞지 않을 때만 자체 GDI/PC 폴백을 설치한다.
    // 여기서 INSERT_PC_HOOKS(0)을 앞질러 보내면 ShinaRio의 전용
    // GetTextExtentPoint32A 후크를 generic GDI 후크가 덮어쓸 수 있다.
    run_notification_loop(pid, &server, shared, events, hwnd_raw, message);

    // 정리. 사용자 중지가 아니면 연결 끊김으로 보고한다.
    let by_user = shared.is_stopped();
    tracing::info!(
        target: LUNAHOOK_TARGET,
        "세션 종료: pid={pid} {}",
        if by_user {
            "사용자 중지"
        } else {
            "연결 끊김"
        }
    );
    drop(server);
    shared.connected.store(false, Ordering::Release);
    *shared.server_slot() = None;
    if !by_user {
        push_event(
            events,
            hwnd_raw,
            message,
            HookEvent::Detached {
                pid,
                by_user: false,
            },
        );
    }
}

/// 1~3단계: 비트니스 판별 → 인젝션 → 파이프 서버 생성 → 접속 대기 →
/// 핸드셰이크. 실패하면 [`InjectedGuard`]가 스코프 종료 시 자동으로
/// uninject한다.
fn attach_and_handshake(
    pid: u32,
    shared: &Arc<SessionShared>,
) -> Result<(Arch, Arc<PipeServer>), SetupError> {
    // SAFETY: pid는 사용자가 방금 고른 살아 있는 프로세스 식별자다.
    let arch = unsafe { inject::detect_arch(pid) }
        .map_err(|error| SetupError::always(error.to_string()))?;
    let dll = dll_path(arch);
    if !dll.is_file() {
        return Err(SetupError::always(format!(
            "후킹 DLL이 없습니다: {}",
            dll.display()
        )));
    }

    let guard = inject_target(pid, arch, &dll).map_err(SetupError::always)?;

    let server = match PipeServer::create(pid) {
        Ok(server) => Arc::new(server),
        Err(error @ PipeError::Create(_)) => return Err(SetupError::always(error.to_string())),
        Err(PipeError::ConnectTimeout) => {
            return Err(SetupError::always(
                "게임이 후킹 DLL에 접속하지 않았습니다 (접속 대기 제한 시간 초과)".to_string(),
            ));
        }
        Err(_) => {
            return Err(SetupError::always(
                "게임이 후킹 DLL에 접속하지 않았습니다".to_string(),
            ));
        }
    };
    // 접속 대기 도중의 detach가 파이프를 취소할 수 있도록 미리 공유한다.
    *shared.server_slot() = Some(Arc::clone(&server));

    if let Err(error) = server.wait_connect() {
        server.cancel_pending();
        *shared.server_slot() = None;
        let message = match error {
            PipeError::ConnectTimeout => {
                "게임이 후킹 DLL에 접속하지 않았습니다 (접속 대기 제한 시간 초과)".to_string()
            }
            other => format!("게임이 후킹 DLL에 접속하지 않았습니다: {other}"),
        };
        return Err(SetupError::unless_stopped(message));
    }
    shared.connected.store(true, Ordering::Release);

    let cwd = match server.handshake() {
        Ok(cwd) => cwd,
        Err(error) => {
            server.cancel_pending();
            shared.connected.store(false, Ordering::Release);
            *shared.server_slot() = None;
            return Err(SetupError::unless_stopped(error.to_string()));
        }
    };
    tracing::info!(pid, ?arch, %cwd, "후킹 세션 연결됨");
    tracing::info!(target: LUNAHOOK_TARGET, "핸드셰이크 완료: pid={pid} arch={arch:?} dll cwd={cwd}");
    debug_assert_eq!(server.pid(), pid);

    guard.disarm();
    Ok((arch, server))
}

/// 비트니스에 맞는 절차로 DLL을 주입하고, 실패 시 자동 정리되는 가드로 감싼다.
fn inject_target(pid: u32, arch: Arch, dll: &Path) -> Result<InjectedGuard, String> {
    let module = match arch {
        Arch::X64 => {
            // SAFETY: 위 detect_arch가 성공한 살아 있는 대상이다.
            unsafe { inject::inject_same_bitness(pid, dll) }
        }
        Arch::X86 => inject::inject_via_helper(&inject32_helper_path(), pid, dll),
    }
    .map_err(|error| error.to_string())?;
    Ok(InjectedGuard::armed(pid, arch, module))
}

/// 알림 읽기 루프. ReadFile 실패(연결 끊김/취소) 또는 stop 요청 시 반환한다.
fn run_notification_loop(
    pid: u32,
    server: &PipeServer,
    shared: &Arc<SessionShared>,
    events: &EventSlot,
    hwnd_raw: usize,
    message: u32,
) {
    let mut buffer = vec![0u8; TEXT_BUFFER_SIZE + MAX_NOTIFICATION_OVERHEAD];
    // 물고 있는 선행 바이트는 세션 안에서만 뜻이 있다.
    let mut lead_bytes = LeadBytes::default();
    loop {
        if shared.is_stopped() {
            break;
        }
        let Some(notification) = server.read_notification(&mut buffer) else {
            break;
        };
        handle_notification(
            pid,
            notification,
            &mut lead_bytes,
            shared,
            events,
            hwnd_raw,
            message,
        );
    }
}

fn handle_notification(
    pid: u32,
    notification: Notification,
    lead_bytes: &mut LeadBytes,
    shared: &SessionShared,
    events: &EventSlot,
    hwnd_raw: usize,
    message: u32,
) {
    match notification {
        Notification::Text(text) => {
            handle_text_notification(text, lead_bytes, events, hwnd_raw, message)
        }
        Notification::EngineDetected(engine_name) => {
            tracing::info!(%engine_name, "게임 엔진 탐지");
            tracing::info!(target: LUNAHOOK_TARGET, "엔진 탐지: {engine_name}");
            push_event(
                events,
                hwnd_raw,
                message,
                HookEvent::EngineDetected(engine_name),
            );
        }
        Notification::FoundHook(found) => {
            tracing::info!(
                address = found.hook_address,
                flags = found.hook_type_flags,
                text = %found.text,
                "후보 후크 발견"
            );
            tracing::info!(
                target: LUNAHOOK_TARGET,
                "후보 발견: @{:x} flags={:x} {}",
                found.hook_address,
                found.hook_type_flags,
                found.text
            );
            let generation = shared.search_generation.load(Ordering::Acquire);
            push_event(
                events,
                hwnd_raw,
                message,
                HookEvent::FoundHook { found, generation },
            );
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
            tracing::info!(
                target: LUNAHOOK_TARGET,
                "[dll{}] {info_message}",
                if warning { " 경고" } else { "" }
            );
            push_event(events, hwnd_raw, message, HookEvent::Info(info_message));
        }
        Notification::Removed(address) => {
            tracing::debug!(address, "후크 제거 통지");
            tracing::info!(target: LUNAHOOK_TARGET, "후크 제거: @{address:x}");
        }
        Notification::Inserting { address, hook_code } => {
            tracing::debug!(address, "후크 설치 진행");
            tracing::info!(target: LUNAHOOK_TARGET, "후크 설치: {hook_code} @{address:x}");
            push_event(
                events,
                hwnd_raw,
                message,
                HookEvent::HookInserted { address, hook_code },
            );
        }
        Notification::Ignored(kind) => tracing::trace!(kind, "무시된 알림"),
    }
}

fn handle_text_notification(
    text: TextNotification,
    lead_bytes: &mut LeadBytes,
    events: &EventSlot,
    hwnd_raw: usize,
    message: u32,
) {
    let received_at = std::time::Instant::now();
    tracing::trace!(
        pid = text.process_id,
        address = text.hook_address,
        name = %text.hook_name,
        ctx2 = text.thread_ctx2,
        "텍스트 수신"
    );
    let source = HookSource {
        address: text.thread_addr,
        context: text.thread_ctx,
        subcontext: text.thread_ctx2,
    };
    let decoded = lead_bytes.decode(
        source,
        text.hook_type_flags,
        text.detected_codepage as u16,
        &text.payload,
    );
    if decoded.is_empty() {
        return;
    }
    // `lunactl`의 텍스트 줄과 같은 모양으로, 사용자가 고른 스레드의 것만
    // 남긴다.
    //
    // 고르지 않은 스레드까지 남기면 파일명을 쉬지 않고 뱉는 훅 하나가 회전
    // 상한(4MB × 3)을 순식간에 채워, 정작 보려던 대사를 밀어낸다. 어느
    // 스레드가 무엇을 내는지는 후킹 관리 창의 스트림 목록에서 본다.
    if crate::hook::is_selected_text_source(source) {
        tracing::info!(
            target: LUNAHOOK_TARGET,
            "[text @{:x} t={:x} ctx={:x} ctx2={:x} {}]{decoded}",
            text.hook_address,
            text.hook_type_flags,
            source.context,
            source.subcontext,
            text.hook_name,
        );
    }
    push_event(
        events,
        hwnd_raw,
        message,
        HookEvent::Text {
            text: HookText {
                source,
                hook_name: text.hook_name,
                text: decoded,
                full_string: HookType::from_bits_truncate(text.hook_type_flags)
                    .contains(HookType::FULL_STRING),
            },
            received_at,
        },
    );
}

#[cfg(test)]
#[path = "../../../tests/unit/hook/worker/session.rs"]
mod tests;
