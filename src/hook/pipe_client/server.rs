//! `PipeServer` — pid에 대응하는 두 개의 message-mode 파이프 서버.
//!
//! lunahook_rs.dll이 클라이언트다. DLL의 접속 루프
//! (`lunahook_rs::host::connect_and_run`)는 `LUNA_PIPE_AVAILABLE{pid}` 이벤트를
//! 기다렸다 두 파이프를 열고 핸드셰이크를 시작하므로, anemone은 반드시 서버로서
//! 파이프를 먼저 만들고 이벤트를 신호해야 한다.
//!
//! 두 파이프 모두 message mode다 (lunahook_rs::host::PipeIo 계약 —
//! `read_host` 1회 = 호스트가 쓴 명령 1개). 따라서 ReadFile 1회가 알림 1개,
//! WriteFile 1회가 명령 1개를 완전히 담는다.

use std::io;
use std::sync::Mutex;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND,
    ReadFile, WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
    PIPE_TYPE_MESSAGE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent};

use lunahook_rs::protocol::{HOOK_PIPE, HOST_PIPE, PIPE_AVAILABLE_EVENT, rpc_frame, rpc_id};

use super::PipeError;
use super::handshake;
use super::notification::{Notification, parse_notification};
use super::overlapped::{
    OverlappedOp, finish_io, finish_io_with_timeout, last_win_error, start_io,
};
use super::security::{PipeSecurity, pipe_name};

/// DLL의 파이프 접속 제한 시간. 인젝션 자체는 10초 안에 끝나고(inject.rs),
/// DLL은 로드 직후 이벤트를 기다렸다 곧바로 접속하므로 정상 경로에서는 이
/// 값의 일부만 쓴다. 초과했다는 것은 DLL이 죽었거나 초기화에 실패했다는 뜻이다.
const CONNECT_TIMEOUT_MS: u32 = 15_000;

/// pid에 대응하는 두 개의 message-mode 파이프 서버.
pub(crate) struct PipeServer {
    pid: u32,
    /// DLL이 WriteFile하는 텍스트 파이프 (`LUNA_HOOK{pid}`), inbound.
    hook_pipe: HANDLE,
    /// DLL이 ReadFile하는 명령 파이프 (`LUNA_HOST{pid}`), outbound.
    host_pipe: HANDLE,
    /// `LUNA_PIPE_AVAILABLE{pid}` 명명 이벤트. 명명 커널 오브젝트는 참조
    /// 카운트 기반이라, SetEvent 직후 여기서 핸들을 닫아버리면 호스트가
    /// 유일한 소유자였던 경우 오브젝트 자체가 파괴된다. DLL의 파이프 스레드는
    /// loader lock 때문에 LoadLibraryW가 반환된 뒤에야 CreateEventW를 호출할
    /// 수 있어 호스트보다 항상 늦을 수 있으므로, PipeServer가 살아 있는 동안
    /// (즉 세션 내내) 이 핸들을 계속 열어 둬야 DLL이 나중에 같은 이름을 열 때
    /// 기존의 signaled 오브젝트를 그대로 얻는다. Drop에서만 닫는다.
    available_event: HANDLE,
    /// 명령 WriteFile 직렬화 (여러 스레드에서 보낼 수 있다).
    write_lock: Mutex<()>,
}

// SAFETY: HANDLE은 커널 오브젝트 참조다. Win32는 서로 다른 스레드에서의
// ReadFile/WriteFile/CancelIoEx를 허용하고, 명령 쓰기는 write_lock으로
// 직렬화한다. self는 create()가 만든 핸들을 drop까지 소유하므로 use-after-close
// 도 없다.
unsafe impl Send for PipeServer {}
unsafe impl Sync for PipeServer {}

impl Drop for PipeServer {
    fn drop(&mut self) {
        // SAFETY: 두 핸들은 create()가 만들고 drop까지 살아 있는 파이프 핸들이다.
        unsafe {
            let _ = CancelIoEx(self.hook_pipe, std::ptr::null());
            let _ = CancelIoEx(self.host_pipe, std::ptr::null());
            let _ = DisconnectNamedPipe(self.hook_pipe);
            let _ = DisconnectNamedPipe(self.host_pipe);
            let _ = CloseHandle(self.hook_pipe);
            let _ = CloseHandle(self.host_pipe);
        }
        // SAFETY: available_event는 create()가 만들고 drop까지 살아 있는
        // 이벤트 핸들이다. 세션이 끝나는 지금에서야 참조를 놓아 오브젝트가
        // 사라지게 한다.
        unsafe {
            let _ = CloseHandle(self.available_event);
        }
    }
}

impl PipeServer {
    /// 두 파이프 서버를 만들고 DLL을 깨우는 이벤트를 신호한다.
    ///
    /// 이벤트는 auto-reset이므로 DLL 하나만 깬다. DLL의 파이프 스레드는 loader
    /// lock 때문에 LoadLibraryW가 반환된 뒤에야 CreateEventW를 호출할 수
    /// 있으므로, 호스트가 먼저 이 이벤트를 만들어 SetEvent하는 경우가 흔하다.
    /// 이때 호스트가 핸들을 곧바로 닫으면 참조 카운트가 0이 되어 명명 오브젝트
    /// 자체가 파괴되고, DLL이 뒤늦게 같은 이름으로 CreateEventW하면 signaled가
    /// 아닌 새 오브젝트를 얻어 WaitForSingleObject에서 영원히 멈춘다. 이를
    /// 막기 위해 반환된 이벤트 핸들은 self.available_event로 세션 내내
    /// 살려 두고 Drop에서만 닫는다. (DLL이 먼저 대기 중이었다면 SetEvent가
    /// 그 인스턴스를 바로 깨우므로 어느 순서든 안전하다.)
    pub fn create(pid: u32) -> Result<Self, PipeError> {
        const BUFFER_SIZE: u32 = 50_000;
        let mut security = PipeSecurity::permissive().map_err(PipeError::Create)?;

        // SAFETY: 이름은 NUL 종결 UTF-16 버퍼, 플래그는 정적값이다. 반환된
        // 핸들은 성공 시 self가 소유하고, 이후 단계 실패 시 즉시 닫는다.
        // FILE_FLAG_OVERLAPPED: 접속 대기에 제한 시간을 두기 위해 오버랩 I/O로
        // 운영한다(connect_one). 따라서 이 핸들의 모든 ReadFile에도 OVERLAPPED가
        // 필요하다.
        let hook_pipe = unsafe {
            CreateNamedPipeW(
                pipe_name(HOOK_PIPE, pid).as_ptr(),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                1,
                BUFFER_SIZE,
                BUFFER_SIZE,
                0,
                security.as_mut_ptr(),
            )
        };
        if hook_pipe.is_null() || hook_pipe == (-1isize as HANDLE) {
            return Err(PipeError::Create(last_win_error()));
        }

        // SAFETY: 동일하며, 실패 시 위에서 만든 hook_pipe를 닫고 반환한다.
        let host_pipe = unsafe {
            CreateNamedPipeW(
                pipe_name(HOST_PIPE, pid).as_ptr(),
                PIPE_ACCESS_OUTBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                1,
                BUFFER_SIZE,
                BUFFER_SIZE,
                0,
                security.as_mut_ptr(),
            )
        };
        if host_pipe.is_null() || host_pipe == (-1isize as HANDLE) {
            let error = last_win_error();
            unsafe {
                let _ = CloseHandle(hook_pipe);
            }
            return Err(PipeError::Create(error));
        }

        // SAFETY: 이름은 NUL 종결 UTF-16이고 security는 호출 동안 유효하다.
        let event_name = pipe_name(PIPE_AVAILABLE_EVENT, pid);
        let event = unsafe { CreateEventW(security.as_ptr(), 0, 0, event_name.as_ptr()) };
        let event = if !event.is_null() {
            event
        } else {
            let error = io::Error::last_os_error();
            unsafe {
                let _ = CloseHandle(hook_pipe);
                let _ = CloseHandle(host_pipe);
            }
            return Err(PipeError::Create(error));
        };
        // SAFETY: DLL이 같은 이름으로 연 auto-reset 이벤트를 깨운다. 이 핸들은
        // 실패 시에만 여기서 닫고, 성공하면 self.available_event로 세션 내내
        // 열어 둔다 (구조체 필드 주석 참고 — 명명 오브젝트가 참조 카운트로
        // 파괴되는 것을 막기 위함).
        let set_result = unsafe { SetEvent(event) };
        if set_result == 0 {
            let error = io::Error::last_os_error();
            unsafe {
                let _ = CloseHandle(event);
                let _ = CloseHandle(hook_pipe);
                let _ = CloseHandle(host_pipe);
            }
            return Err(PipeError::Create(error));
        }

        Ok(Self {
            pid,
            hook_pipe,
            host_pipe,
            available_event: event,
            write_lock: Mutex::new(()),
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// 정확히 `out.len()`바이트를 읽는다 (핸드셰이크용).
    pub(super) fn read_exact(&self, out: &mut [u8]) -> Result<(), PipeError> {
        let mut filled = 0usize;
        while filled < out.len() {
            let read_bytes = self.read_chunk(&mut out[filled..])?;
            if read_bytes == 0 {
                return Err(PipeError::Disconnected);
            }
            filled += read_bytes as usize;
        }
        Ok(())
    }

    /// message-mode 파이프에서 한 메시지를 통째로 읽는다. `read_exact`처럼
    /// 작은 버퍼로 RPC 헤더만 먼저 읽으면, 헤더 뒤 payload가 같은 메시지에
    /// 포함된 경우 `ERROR_MORE_DATA`가 발생하므로 프레임 수신에는 이 경로를
    /// 사용해야 한다.
    pub(super) fn read_message(&self, out: &mut [u8]) -> Result<usize, PipeError> {
        let read_bytes = self.read_chunk(out)?;
        if read_bytes == 0 {
            return Err(PipeError::Disconnected);
        }
        Ok(read_bytes as usize)
    }

    /// 파이프에서 한 덩어리를 읽고 실제로 읽은 바이트 수를 돌려준다.
    /// 연결 종료/취소 시 Err(Disconnected)다.
    fn read_chunk(&self, out: &mut [u8]) -> Result<u32, PipeError> {
        let mut op = OverlappedOp::new().map_err(PipeError::Create)?;
        // SAFETY: out은 유효한 쓰기 버퍼이고, op는 finish_io가 완료를 확인할
        // 때까지 살아 있어 커널이 OVERLAPPED 접근을 마친 뒤에 해제된다.
        unsafe {
            let mut transferred = 0u32;
            start_io(ReadFile(
                self.hook_pipe,
                out.as_mut_ptr(),
                out.len() as u32,
                &mut transferred,
                &mut op.overlapped,
            ))?
        };
        // SAFETY: op는 진행 중인 ReadFile에 결합돼 있다.
        unsafe { finish_io(self.hook_pipe, &op) }
    }

    /// DLL의 접속을 기다린다. 두 파이프가 모두 연결되면 Ok.
    ///
    /// DLL이 죽었거나 접속하지 않는 비정상 상태에서 세션이 영원히 붙어 있지
    /// 않도록, 제한 시간 안에 접속하지 못하면 ConnectTimeout으로 실패한다.
    pub fn wait_connect(&self) -> Result<(), PipeError> {
        self.wait_connect_within(Duration::from_millis(u64::from(CONNECT_TIMEOUT_MS)))
    }

    /// 제한 시간을 지정하는 내부 경로 (테스트에서 짧게 쓴다).
    fn wait_connect_within(&self, timeout: Duration) -> Result<(), PipeError> {
        self.connect_one(self.hook_pipe, timeout)?;
        self.connect_one(self.host_pipe, timeout)
    }

    fn connect_one(&self, pipe: HANDLE, timeout: Duration) -> Result<(), PipeError> {
        let mut op = OverlappedOp::new().map_err(PipeError::Create)?;
        // SAFETY: pipe는 self가 소유한 유효한 서버 파이프 핸들이고 op는 이 함수
        // 전체에서 살아 있다. 타임아웃/실패 경로에서는 cancel_and_settle로
        // 커널의 OVERLAPPED 접근이 끝났음을 보장한 뒤 반환한다.
        let started = unsafe { ConnectNamedPipe(pipe, &mut op.overlapped) };
        // 즉시 성공했거나(started != 0) 클라이언트가 이미 붙어 있으면
        // (ERROR_PIPE_CONNECTED) 대기할 I/O가 없다.
        let pending = if started != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED {
            false
        } else {
            start_io(0)?
        };
        if !pending {
            return Ok(());
        }
        match op.wait(timeout.as_millis().min(u32::MAX as u128) as u32) {
            Ok(true) => {
                // 이벤트 신호는 성공이 아니라 I/O 완료만 뜻한다. 연결 실패도
                // 신호 상태가 될 수 있으므로 실제 결과를 반드시 확인한다.
                let mut transferred = 0u32;
                if unsafe { GetOverlappedResult(pipe, &op.overlapped, &mut transferred, 1) } == 0 {
                    Err(PipeError::Disconnected)
                } else {
                    Ok(())
                }
            }
            Ok(false) => {
                op.cancel_and_settle(pipe);
                Err(PipeError::ConnectTimeout)
            }
            Err(()) => {
                op.cancel_and_settle(pipe);
                Err(PipeError::Disconnected)
            }
        }
    }

    /// `communication_initialize`의 호스트 측 절차. [`handshake::perform_handshake`]로
    /// 위임한다. 성공하면 게임 작업 폴더를 돌려준다(로깅용).
    pub fn handshake(&self) -> Result<String, PipeError> {
        handshake::perform_handshake(self)
    }

    /// 알림 1개를 `out`(최소 TEXT_BUFFER_SIZE + 헤더 크기)에 읽어 파싱한다.
    /// 연결 종료/취소 시 None.
    pub fn read_notification(&self, out: &mut [u8]) -> Option<Notification> {
        let read_bytes = match self.read_chunk(out) {
            Ok(bytes) => bytes as usize,
            Err(PipeError::Disconnected) => return None,
            Err(error) => {
                tracing::warn!("파이프 읽기 실패: {error}");
                return None;
            }
        };
        if read_bytes == 0 || read_bytes > out.len() {
            return None;
        }
        parse_notification(&out[..read_bytes])
    }

    /// 직렬화된 명령 1개를 HOST_PIPE로 보낸다.
    pub fn write_command(&self, command: &[u8]) -> Result<(), PipeError> {
        self.write_command_with_timeout(command, None)
    }

    fn write_command_with_timeout(
        &self,
        command: &[u8],
        timeout: Option<Duration>,
    ) -> Result<(), PipeError> {
        let guard = self
            .write_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut op = OverlappedOp::new().map_err(PipeError::Create)?;
        // SAFETY: host_pipe는 self가 소유한 유효한 핸들이고 command는 읽기
        // 가능 버퍼다. message mode에서 1회 WriteFile은 명령 1개와 동치다.
        unsafe {
            let mut transferred = 0u32;
            start_io(WriteFile(
                self.host_pipe,
                command.as_ptr(),
                command.len() as u32,
                &mut transferred,
                &mut op.overlapped,
            ))?;
            if let Some(timeout) = timeout {
                finish_io_with_timeout(self.host_pipe, &op, timeout)?;
            } else {
                finish_io(self.host_pipe, &op)?;
            }
        }
        drop(guard);
        Ok(())
    }
    /// 세션을 끝낸다: DETACH 명령(최선) → 대기 중 I/O 취소.
    ///
    /// DETACH를 받은 DLL은 한 번의 연결만 닫고 다시 이벤트를 기다린다. anemone이
    /// 이벤트를 다시 신호하지 않으면 DLL 스레드는 잠자 상태로 남는다(CPU 미사용) —
    /// 원본 LunaHost와 동일한 거동이다.
    pub fn shutdown(&self) {
        let detach = rpc_frame(rpc_id::DETACH, &[]).unwrap_or_else(|| {
            tracing::error!("DETACH 프레임이 파이프 버퍼 크기를 초과했습니다 (있을 수 없는 상황)");
            Vec::new()
        });
        // 먼저 읽기 대기를 깨워 DLL이 종료 경로에 들어가게 한다. 반대 순서로
        // 보내면 고장 난 DLL이 HOST_PIPE를 읽지 않는 동안 무기한 대기할 수 있다.
        self.cancel_pending();
        let _ = self.write_command_with_timeout(&detach, Some(Duration::from_millis(500)));
    }

    /// 연결 대기 또는 읽기 중인 OVERLAPPED 작업을 중단한다. 아직 클라이언트가
    /// 접속하지 않은 서버에도 안전하게 호출할 수 있어 attach 취소에 사용한다.
    pub fn cancel_pending(&self) {
        // SAFETY: 자기 소유 파이프에 대한 취소는 언제나 안전하다.
        unsafe {
            let _ = CancelIoEx(self.hook_pipe, std::ptr::null());
            let _ = CancelIoEx(self.host_pipe, std::ptr::null());
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/hook/pipe_client/server.rs"]
mod tests;
