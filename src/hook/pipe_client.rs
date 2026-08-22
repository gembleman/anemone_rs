//! LUNA_HOST/LUNA_HOOK named pipe 서버 측 구현.
//!
//! lunahook_rs.dll이 클라이언트다. DLL의 접속 루프
//! (`lunahook_rs::host::connect_and_run`)는 `LUNA_PIPE_AVAILABLE{pid}` 이벤트를
//! 기다렸다 두 파이프를 열고 핸드셰이크를 시작하므로, anemone은 반드시 서버로서
//! 파이프를 먼저 만들고 이벤트를 신호해야 한다.
//!
//! 두 파이프 모두 message mode다 (lunahook_rs::host::PipeIo 계약 —
//! `read_host` 1회 = 호스트가 쓴 명령 1개). 따라서 ReadFile 1회가 알림 1개,
//! WriteFile 1회가 명령 1개를 완전히 담는다.

use std::mem::{offset_of, size_of};
use std::sync::Mutex;
use std::time::Duration;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND,
    ReadFile, WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
    PIPE_TYPE_MESSAGE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForSingleObject};
use windows::core::{Error as WinError, HRESULT, PCWSTR, Result as WinResult};

use lunahook_rs::protocol::{
    COMPATIBLE_SIG_BYTES, HOOK_PIPE, HOST_PIPE, HookFoundNotif, HookInsertingNotif,
    HookRemovedNotif, HostInfo, HostInfoNotif, HostNotificationType, MESSAGE_SIZE,
    TextOutputHeader, VERSION_WIRE_SIZE,
};

/// 파이프 오류.
#[derive(Debug, thiserror::Error)]
pub enum PipeError {
    #[error("파이프 생성 실패: {0}")]
    Create(WinError),
    #[error("게임과의 연결이 끊겼습니다")]
    Disconnected,
    #[error("핸드셰이크 실패: {0}")]
    Handshake(String),
    #[error("DLL이 제한 시간 안에 파이프에 접속하지 않았습니다")]
    ConnectTimeout,
}

/// hook → host 방향으로 받은 알림 하나.
///
/// wire 바이트를 곧장 `RawHookParam`으로 재구성하지 않는다 — 그 안의 `JitType`
/// 은 무효 판별값일 수 있는(`#[repr(u32)]` 필드리스 enum) 신뢰할 수 없는 입력이고,
/// 무효한 enum 값을 만드는 것은 UB다. 필요한 정수/문자열 필드만 offset_of 기반으로
/// 안전하게 꺼낸다.
#[derive(Debug)]
pub enum Notification {
    Text(TextNotification),
    FoundHook(FoundHook),
    /// 후크 제거 통지 (address).
    Removed(u64),
    /// DLL이 후크 설치를 진행 중 (주소 + hookcode).
    Inserting {
        address: u64,
    },
    /// DLL 측 안내/경고 문자열.
    Info {
        warning: bool,
        message: String,
    },
    /// 형식을 알 수 없거나 무시해도 되는 알림.
    Ignored(u32),
}

#[derive(Debug, Clone)]
pub struct TextNotification {
    pub process_id: u32,
    pub thread_addr: u64,
    pub thread_ctx: u64,
    pub thread_ctx2: u64,
    pub hook_address: u64,
    pub hook_type_flags: u64,
    pub hook_name: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct FoundHook {
    pub hook_type_flags: u64,
    pub hook_address: u64,
    pub text: String,
}

/// pid에 대응하는 두 개의 message-mode 파이프 서버.
pub(crate) struct PipeServer {
    pid: u32,
    /// DLL이 WriteFile하는 텍스트 파이프 (`LUNA_HOOK{pid}`), inbound.
    hook_pipe: HANDLE,
    /// DLL이 ReadFile하는 명령 파이프 (`LUNA_HOST{pid}`), outbound.
    host_pipe: HANDLE,
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
            let _ = CancelIoEx(self.hook_pipe, None);
            let _ = CancelIoEx(self.host_pipe, None);
            let _ = DisconnectNamedPipe(self.hook_pipe);
            let _ = DisconnectNamedPipe(self.host_pipe);
            let _ = CloseHandle(self.hook_pipe);
            let _ = CloseHandle(self.host_pipe);
        }
    }
}

fn pipe_name(prefix: &str, pid: u32) -> Vec<u16> {
    format!("{prefix}{pid}").encode_utf16().chain([0]).collect()
}

/// DLL의 파이프 접속 제한 시간. 인젝션 자체는 10초 안에 끝나고(inject.rs),
/// DLL은 로드 직후 이벤트를 기다렸다 곧바로 접속하므로 정상 경로에서는 이
/// 값의 일부만 쓴다. 초과했다는 것은 DLL이 죽었거나 초기화에 실패했다는 뜻이다.
const CONNECT_TIMEOUT_MS: u32 = 15_000;

/// 오버랩 I/O 1회용 헬퍼 — 완료 신호 이벤트를 OVERLAPPED에 결합한다.
///
/// 파이프 핸들이 FILE_FLAG_OVERLAPPED로 만들어졌으므로 모든 ReadFile/
/// WriteFile/ConnectNamedPipe에 OVERLAPPED가 필요하다. 대기는 이벤트로 하고,
/// 중단 시에는 GetOverlappedResult(bWait)로 커널 사용 종료를 확인한 뒤
/// 이벤트 핸들을 닫아 수명 문제를 없앤다.
struct OverlappedOp {
    overlapped: OVERLAPPED,
}

impl OverlappedOp {
    fn new() -> Result<Self, WinError> {
        // SAFETY: 이름 없는 수동 리셋 이벤트는 이 프로세스에서만 다룬다.
        let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }?;
        let overlapped = OVERLAPPED {
            hEvent: event,
            ..Default::default()
        };
        Ok(Self { overlapped })
    }

    /// Ok(true)=완료 신호, Ok(false)=제한 시간 도달(아직 진행 중), Err=대기 실패.
    ///
    /// # Safety
    /// self.overlapped가 현재 진행 중인 I/O에 결합돼 있어야 한다.
    fn wait(&self, timeout_ms: u32) -> Result<bool, ()> {
        // SAFETY: 이벤트 핸들은 new()가 만든 유효한 값이고 Drop에서 닫는다.
        match unsafe { WaitForSingleObject(self.overlapped.hEvent, timeout_ms) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(()),
        }
    }

    /// 진행 중일 수 있는 작업을 취소하고 커널이 OVERLAPPED를 다 쓸 때까지
    /// 기다린다 — 스택 OVERLAPPED/이벤트 정리 전의 수명 보장이 목적이다.
    fn cancel_and_settle(&self, pipe: HANDLE) {
        // SAFETY: pipe와 overlapped는 방금 건 작업의 유효한 조합이다.
        unsafe {
            let _ = CancelIoEx(pipe, Some(&self.overlapped));
            let mut transferred = 0u32;
            // bWait=true — 취소 완료(ERROR_OPERATION_ABORTED 포함)까지 확실히 기다린다.
            let _ = GetOverlappedResult(pipe, &self.overlapped, &mut transferred, true);
        }
    }
}

impl Drop for OverlappedOp {
    fn drop(&mut self) {
        // SAFETY: new()가 만든 이벤트 핸들을 정확히 한 번 닫는다. 호출자는
        // 진행 중인 작업을 cancel_and_settle이나 대기로 먼저 마쳐야 한다.
        unsafe {
            let _ = CloseHandle(self.overlapped.hEvent);
        }
    }
}

/// 오버랩 I/O 시작 결과를 정규화한다. Ok(true)=진행 중(대기 필요),
/// Ok(false)=즉시 완료, Err=시작 실패(연결 문제).
fn start_io(result: WinResult<()>) -> Result<bool, PipeError> {
    match result {
        Ok(()) => Ok(false),
        Err(error) if error.code() == ERROR_IO_PENDING.to_hresult() => Ok(true),
        Err(_) => Err(PipeError::Disconnected),
    }
}

/// 진행 중인 오버랩 요청을 무한히 기다리고 전송 바이트 수를 돌려준다.
/// 연결 끊김/취소면 Err — 호출자는 이를 세션 종료로 해석한다.
///
/// # Safety
/// `op`는 현재 `pipe`에서 진행 중인 I/O에 결합돼 있어야 한다.
unsafe fn finish_io(pipe: HANDLE, op: &OverlappedOp) -> Result<u32, PipeError> {
    // SAFETY: op 내부 이벤트 핸들은 new()가 만든 유효한 값이다.
    unsafe {
        op.wait(INFINITE).map_err(|()| PipeError::Disconnected)?;
        let mut transferred = 0u32;
        // bWait=true — 이벤트 신호와 완료 기록 사이의 미세한 창을 흡수한다.
        GetOverlappedResult(pipe, &op.overlapped, &mut transferred, true)
            .map_err(|_| PipeError::Disconnected)?;
        Ok(transferred)
    }
}

impl PipeServer {
    /// 두 파이프 서버를 만들고 DLL을 깨우는 이벤트를 신호한다.
    ///
    /// DLL이 이미 이벤트 대기 중이라도, 파이프가 존재한 뒤 열리면 되므로 순서
    /// 문제는 없다. 이벤트는 auto-reset이므로 DLL 하나만 깬다.
    pub fn create(pid: u32) -> Result<Self, PipeError> {
        const BUFFER_SIZE: u32 = 50_000;

        // SAFETY: 이름은 NUL 종결 UTF-16 버퍼, 플래그는 정적값이다. 반환된
        // 핸들은 성공 시 self가 소유하고, 이후 단계 실패 시 즉시 닫는다.
        // FILE_FLAG_OVERLAPPED: 접속 대기에 제한 시간을 두기 위해 오버랩 I/O로
        // 운영한다(connect_one). 따라서 이 핸들의 모든 ReadFile에도 OVERLAPPED가
        // 필요하다.
        let hook_pipe = unsafe {
            CreateNamedPipeW(
                PCWSTR(pipe_name(HOOK_PIPE, pid).as_ptr()),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                1,
                BUFFER_SIZE,
                BUFFER_SIZE,
                0,
                None,
            )
        };
        if hook_pipe.is_invalid() {
            return Err(PipeError::Create(last_win_error()));
        }

        // SAFETY: 동일하며, 실패 시 위에서 만든 hook_pipe를 닫고 반환한다.
        let host_pipe = unsafe {
            CreateNamedPipeW(
                PCWSTR(pipe_name(HOST_PIPE, pid).as_ptr()),
                PIPE_ACCESS_OUTBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                1,
                BUFFER_SIZE,
                BUFFER_SIZE,
                0,
                None,
            )
        };
        if host_pipe.is_invalid() {
            let error = last_win_error();
            unsafe {
                let _ = CloseHandle(hook_pipe);
            }
            return Err(PipeError::Create(error));
        }

        // SAFETY: 이름 없는(auto-reset) 이벤트는 이 프로세스에서만 다룬다.
        let event = unsafe { CreateEventW(None, false, false, PCWSTR::null()) };
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                unsafe {
                    let _ = CloseHandle(hook_pipe);
                    let _ = CloseHandle(host_pipe);
                }
                return Err(PipeError::Create(error));
            }
        };
        // SAFETY: 방금 만든 유효한 이벤트 핸들이다. 신호 후 즉시 닫아도 DLL의
        // WaitForSingleObject는 이미 signaled 상태를 본다.
        let set_result = unsafe { SetEvent(event) };
        unsafe {
            let _ = CloseHandle(event);
        }
        if let Err(error) = set_result {
            unsafe {
                let _ = CloseHandle(hook_pipe);
                let _ = CloseHandle(host_pipe);
            }
            return Err(PipeError::Create(error));
        }

        Ok(Self {
            pid,
            hook_pipe,
            host_pipe,
            write_lock: Mutex::new(()),
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// 정확히 `out.len()`바이트를 읽는다 (핸드셰이크용).
    fn read_exact(&self, out: &mut [u8]) -> Result<(), PipeError> {
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

    /// 파이프에서 한 덩어리를 읽고 실제로 읽은 바이트 수를 돌려준다.
    /// 연결 종료/취소 시 Err(Disconnected)다.
    fn read_chunk(&self, out: &mut [u8]) -> Result<u32, PipeError> {
        let mut op = OverlappedOp::new().map_err(PipeError::Create)?;
        // SAFETY: out은 유효한 쓰기 버퍼이고, op는 finish_io가 완료를 확인할
        // 때까지 살아 있어 커널이 OVERLAPPED 접근을 마친 뒤에 해제된다.
        unsafe {
            start_io(ReadFile(
                self.hook_pipe,
                Some(out),
                None,
                Some(&mut op.overlapped),
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
        let started = unsafe { ConnectNamedPipe(pipe, Some(&mut op.overlapped)) };
        let pending = match started {
            Ok(()) => false,
            // DLL이 ConnectNamedPipe 호출보다 먼저 접속한 경우다.
            Err(error) if error.code() == ERROR_PIPE_CONNECTED.to_hresult() => false,
            Err(error) => start_io(Err(error))?,
        };
        if !pending {
            return Ok(());
        }
        match op.wait(timeout.as_millis().min(u32::MAX as u128) as u32) {
            Ok(true) => Ok(()),
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

    /// `communication_initialize`의 호스트 측 절차:
    /// version+서명 확인 → 게임 cwd 수신 → 빈 파일 목록(-1 센티널) 전송 →
    /// PreparedOk 확인. 성공하면 게임 작업 폴더를 돌려준다(로깅용).
    pub fn handshake(&self) -> Result<String, PipeError> {
        let mut buffer = vec![0u8; 2048];

        // 1. LUNA_VERSION 4×u16 — DLL은 전부 0으로 보낸다(값 미사용).
        self.read_exact(&mut buffer[..VERSION_WIRE_SIZE])?;
        // 2. COMPATIBLE_SIG.
        self.read_exact(&mut buffer[..COMPATIBLE_SIG_BYTES.len()])?;
        if buffer[..COMPATIBLE_SIG_BYTES.len()] != COMPATIBLE_SIG_BYTES {
            return Err(PipeError::Handshake(
                "lunahook 버전 시그니처가 일치하지 않습니다".to_string(),
            ));
        }
        // 3. cwd: 길이(u32) → UTF-16 본문.
        self.read_exact(&mut buffer[..4])?;
        let cwd_units = u32::from_le_bytes(buffer[..4].try_into().expect("u32 length"));
        if cwd_units > 1024 {
            return Err(PipeError::Handshake(format!(
                "비정상적인 작업 폴더 길이: {cwd_units}"
            )));
        }
        let mut cwd_bytes = vec![0u8; cwd_units as usize * 2];
        self.read_exact(&mut cwd_bytes)?;
        let cwd_units: Vec<u16> = cwd_bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        let cwd = String::from_utf16_lossy(&cwd_units);

        // 4. 파일 목록: anemone은 아무 것도 보내지 않고 -1 센티널만 보낸다.
        self.write_command(&(-1i32).to_le_bytes())
            .map_err(|error| PipeError::Handshake(error.to_string()))?;

        // 5. PreparedOk (u32).
        self.read_exact(&mut buffer[..4])?;
        let prepared = u32::from_le_bytes(buffer[..4].try_into().expect("u32 prepared"));
        if prepared != HostNotificationType::PreparedOk as u32 {
            return Err(PipeError::Handshake(format!(
                "PreparedOk 대신 {prepared}을 받았습니다"
            )));
        }

        Ok(cwd)
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
        let guard = self.write_lock.lock().expect("host pipe write lock");
        let mut op = OverlappedOp::new().map_err(PipeError::Create)?;
        // SAFETY: host_pipe는 self가 소유한 유효한 핸들이고 command는 읽기
        // 가능 버퍼다. message mode에서 1회 WriteFile은 명령 1개와 동치다.
        unsafe {
            start_io(WriteFile(
                self.host_pipe,
                Some(command),
                None,
                Some(&mut op.overlapped),
            ))?;
            finish_io(self.host_pipe, &op)?;
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
        let detach = lunahook_rs::protocol::HostCommandType::Detach as u32;
        let _ = self.write_command(&detach.to_le_bytes());
        // SAFETY: 자기 소유 파이프에 대한 취소는 언제나 안전하다.
        unsafe {
            let _ = CancelIoEx(self.hook_pipe, None);
            let _ = CancelIoEx(self.host_pipe, None);
        }
    }
}

/// 직전 Win32 오류를 windows_core::Error로.
fn last_win_error() -> WinError {
    // SAFETY: GetLastError는 사전조건이 없다. 이 함수들은 실패 직후 한 스레드에서
    // 호출되므로 경합으로 값이 변하지 않는다.
    let code = unsafe { GetLastError() };
    WinError::from(HRESULT::from_win32(code.0))
}

/// 신뢰할 수 없는 wire 바이트 → [`Notification`].
fn parse_notification(bytes: &[u8]) -> Option<Notification> {
    if bytes.len() < 4 {
        return None;
    }
    let kind = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let Some(kind) = HostNotificationType::try_from(kind).ok() else {
        return Some(Notification::Ignored(kind));
    };

    match kind {
        HostNotificationType::Text | HostNotificationType::TextW => {
            // DLL의 로그 채널(Msg::*)은 Text와 같은 커맨드 값을 쓰고
            // `HostInfoNotif`(고정 512바이트)만으로 온다 — 원본 LunaHost처럼
            // 길이로 구별한다.
            if bytes.len() == size_of::<HostInfoNotif>() {
                return Some(Notification::Info {
                    warning: read_u32(bytes, offset_of!(HostInfoNotif, kind))
                        != HostInfo::Console as u32,
                    message: read_fixed_utf8(
                        bytes,
                        offset_of!(HostInfoNotif, message),
                        MESSAGE_SIZE,
                    ),
                });
            }
            parse_text_output(bytes).map(Notification::Text)
        }
        HostNotificationType::FoundHook => parse_found_hook(bytes).map(Notification::FoundHook),
        HostNotificationType::RmvHook => {
            if bytes.len() < size_of::<HookRemovedNotif>() {
                return None;
            }
            let offset = offset_of!(HookRemovedNotif, address);
            Some(Notification::Removed(read_u64(bytes, offset)))
        }
        HostNotificationType::InsertingHook => {
            if bytes.len() < size_of::<HookInsertingNotif>() {
                return None;
            }
            let offset = offset_of!(HookInsertingNotif, addr);
            Some(Notification::Inserting {
                address: read_u64(bytes, offset),
            })
        }
        HostNotificationType::PreparedOk
        | HostNotificationType::EmuInfo
        | HostNotificationType::I18nResp => Some(Notification::Ignored(kind as u32)),
    }
}

fn parse_text_output(bytes: &[u8]) -> Option<TextNotification> {
    let header_len = size_of::<TextOutputHeader>();
    if bytes.len() <= header_len {
        return None;
    }
    // ThreadParam은 정수 필드만 있어 임의 비트 패턴이 전부 유효하다.
    let tp_base = offset_of!(TextOutputHeader, tp);
    let hp_base = offset_of!(TextOutputHeader, hp);

    Some(TextNotification {
        process_id: read_u32(bytes, tp_base),
        thread_addr: read_u64(
            bytes,
            tp_base + offset_of!(lunahook_rs::protocol::ThreadParam, addr),
        ),
        thread_ctx: read_u64(
            bytes,
            tp_base + offset_of!(lunahook_rs::protocol::ThreadParam, ctx),
        ),
        thread_ctx2: read_u64(
            bytes,
            tp_base + offset_of!(lunahook_rs::protocol::ThreadParam, ctx2),
        ),
        hook_address: read_u64(
            bytes,
            hp_base + offset_of!(lunahook_rs::params::RawHookParam, address),
        ),
        hook_type_flags: read_u64(
            bytes,
            hp_base + offset_of!(lunahook_rs::params::RawHookParam, hook_type),
        ),
        hook_name: read_fixed_utf8(
            bytes,
            hp_base + offset_of!(lunahook_rs::params::RawHookParam, name),
            lunahook_rs::params::RawHookParam::default().name.len(),
        ),
        payload: bytes[header_len..].to_vec(),
    })
}

fn parse_found_hook(bytes: &[u8]) -> Option<FoundHook> {
    if bytes.len() < size_of::<HookFoundNotif>() {
        return None;
    }
    let hp_base = offset_of!(HookFoundNotif, hp);
    Some(FoundHook {
        hook_address: read_u64(
            bytes,
            hp_base + offset_of!(lunahook_rs::params::RawHookParam, address),
        ),
        hook_type_flags: read_u64(
            bytes,
            hp_base + offset_of!(lunahook_rs::params::RawHookParam, hook_type),
        ),
        text: read_fixed_utf16(bytes, offset_of!(HookFoundNotif, text), MESSAGE_SIZE),
    })
}

// --- 안전한 little-endian 필드 추출 helpers ------------------------------

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let Some(chunk) = bytes.get(offset..offset + 4) else {
        return 0;
    };
    u32::from_le_bytes(chunk.try_into().expect("4 bytes"))
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    let Some(chunk) = bytes.get(offset..offset + 8) else {
        return 0;
    };
    u64::from_le_bytes(chunk.try_into().expect("8 bytes"))
}

// --- host → hook 명령 직렬화 ---------------------------------------------

/// 유효한 `repr(C)` POD 명령 구조체를 wire 바이트로 복사한다.
///
/// 모든 빌더는 `mem::zeroed()`로 시작해 필드를 채우므로 패딩까지 정의된 값(0)
/// 을 가지고, 구조체 자체는 항상 유효한 상태다. 따라서 바이트 재해석은
/// 초기화되지 않은 메모리를 읽지 않는다.
unsafe fn pod_bytes<T>(value: &T) -> Vec<u8> {
    // SAFETY: T는 repr(C) POD이고 value는 완전히 초기화된 인스턴스다.
    unsafe { std::slice::from_raw_parts(value as *const T as *const u8, size_of::<T>()).to_vec() }
}

/// NEW_HOOK 명령. `hp`는 lunahook_rs가 정의한 유효한 값이어야 한다.
pub fn build_new_hook(hp: &lunahook_rs::params::RawHookParam) -> Vec<u8> {
    // SAFETY: zeroed 시작 — RawHookParam의 모든 비트 패턴은 정수/배열/기본값
    // enum(JitType::PC=0)으로 유효하다.
    unsafe {
        let mut cmd: lunahook_rs::protocol::InsertHookCmd = std::mem::zeroed();
        cmd.command = lunahook_rs::protocol::HostCommandType::NewHook as u32;
        cmd.hp = *hp;
        pod_bytes(&cmd)
    }
}

/// REMOVE_HOOK 명령.
pub fn build_remove_hook(address: u64) -> Vec<u8> {
    // SAFETY: zeroed 시작 — 전부 정수 필드다.
    unsafe {
        let mut cmd: lunahook_rs::protocol::RemoveHookCmd = std::mem::zeroed();
        cmd.command = lunahook_rs::protocol::HostCommandType::RemoveHook as u32;
        cmd.address = address;
        pod_bytes(&cmd)
    }
}

/// FIND_HOOK 명령. `sp.text`가 비었으면 범용 탐색, 있으면 텍스트 검색이다.
pub fn build_find_hook(sp: &lunahook_rs::protocol::SearchParam) -> Vec<u8> {
    // SAFETY: zeroed 시작 — 정수/고정 배열 필드만 있다.
    unsafe {
        let mut cmd: lunahook_rs::protocol::FindHookCmd = std::mem::zeroed();
        cmd.command = lunahook_rs::protocol::HostCommandType::FindHook as u32;
        cmd.sp = *sp;
        pod_bytes(&cmd)
    }
}

/// INSERT_PC_HOOKS 명령. `which == 0`이면 GDI/GDI+/D3DX 폴백 후크.
pub fn build_insert_pc_hooks(which: i32) -> Vec<u8> {
    // SAFETY: zeroed 시작 — 정수 필드만 있다.
    unsafe {
        let mut cmd: lunahook_rs::protocol::InsertPcHooksCmd = std::mem::zeroed();
        cmd.command = lunahook_rs::protocol::HostCommandType::InsertPcHooks as u32;
        cmd.which = which;
        pod_bytes(&cmd)
    }
}

/// 텍스트 검색용 FIND_HOOK payload. `text`가 화면에 보이는 문장과 일치해야 한다.
pub fn build_text_search_param(text: &str) -> lunahook_rs::protocol::SearchParam {
    use lunahook_rs::protocol::{PATTERN_SIZE, SearchParam};
    // SAFETY: SearchParam은 정수/고정 배열 필드만 있어 zeroed가 유효하다.
    unsafe {
        let mut sp: SearchParam = std::mem::zeroed();
        sp.search_time_ms = 30_000;
        sp.max_records = 20;
        sp.codepage = 932; // SHIFT_JIS
        let units: Vec<u16> = text.encode_utf16().take(PATTERN_SIZE - 1).collect();
        sp.text[..units.len()].copy_from_slice(&units);
        sp
    }
}

/// 범용 탐색용 FIND_HOOK payload (`sp.text` 비움 → DLL의 후보 수집 런타임).
pub fn build_general_search_param() -> lunahook_rs::protocol::SearchParam {
    // SAFETY: 동일하다.
    unsafe {
        let mut sp: lunahook_rs::protocol::SearchParam = std::mem::zeroed();
        sp.search_time_ms = 30_000;
        sp.max_records = 50;
        sp.codepage = 932;
        sp
    }
}

/// 고정 길이 NUL-종결 UTF-16 배열 필드를 문자열로.
fn read_fixed_utf16(bytes: &[u8], offset: usize, units: usize) -> String {
    let end = (offset + units * 2).min(bytes.len());
    let slice = &bytes[offset.min(end)..end];
    let collected: Vec<u16> = slice
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&collected)
}

/// 고정 길이 NUL-종결 UTF-8(원본은 로케일 바이트일 수도 있지만 이름 필드는
/// ASCII 범위가 일반적) 배열 필드를 손실 허용 문자열로.
fn read_fixed_utf8(bytes: &[u8], offset: usize, len: usize) -> String {
    let end = (offset + len).min(bytes.len());
    let slice = &bytes[offset.min(end)..end];
    let stop = slice
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..stop]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunahook_rs::params::HookType;
    use lunahook_rs::protocol::HostCommandType;
    use std::fs::OpenOptions;
    use std::time::Instant;

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

    /// anemone이 만든 NEW_HOOK 바이트를 lunahook_rs의 파서로 되읽는 왕복 검사.
    /// 주입 DLL과 같은 파서를 쓰므로, 이 테스트가 통과하면 wire 계약도 유지된다.
    #[test]
    fn new_hook_round_trips_through_lunahook_parser() {
        let hp = lunahook_rs::params::RawHookParam {
            address: 0x0000_7FF6_1234_5678,
            hook_type: (HookType::USING_STRING | HookType::CODEC_UTF16).bits(),
            offset: -80,
            codepage: 932,
            ..lunahook_rs::params::RawHookParam::default()
        };
        let mut hp = hp;
        let name = b"UserH1";
        hp.name[..name.len()].copy_from_slice(name);

        let bytes = build_new_hook(&hp);
        let parsed = lunahook_rs::protocol::parse_host_command(&bytes)
            .expect("lunahook parser accepts our NEW_HOOK");
        let lunahook_rs::protocol::HostCommand::NewHook(cmd) = parsed else {
            panic!("expected NewHook command");
        };
        assert_eq!(cmd.command, HostCommandType::NewHook as u32);
        assert_eq!(cmd.hp.address, hp.address);
        assert_eq!(cmd.hp.hook_type, hp.hook_type);
        assert_eq!(cmd.hp.offset, hp.offset);
        assert_eq!(cmd.hp.codepage, hp.codepage);
        assert_eq!(&cmd.hp.name[..6], b"UserH1");
    }

    #[test]
    fn remove_hook_and_pc_hooks_round_trip() {
        let bytes = build_remove_hook(0x1234);
        match lunahook_rs::protocol::parse_host_command(&bytes).expect("remove parses") {
            lunahook_rs::protocol::HostCommand::RemoveHook(cmd) => {
                assert_eq!(cmd.address, 0x1234);
            }
            _ => panic!("expected RemoveHook"),
        }

        let bytes = build_insert_pc_hooks(0);
        match lunahook_rs::protocol::parse_host_command(&bytes).expect("pc hooks parse") {
            lunahook_rs::protocol::HostCommand::InsertPcHooks(cmd) => {
                assert_eq!(cmd.which, 0);
            }
            _ => panic!("expected InsertPcHooks"),
        }
    }

    #[test]
    fn find_hook_text_search_carries_user_text() {
        let sp = build_text_search_param("テスト");
        let bytes = build_find_hook(&sp);
        match lunahook_rs::protocol::parse_host_command(&bytes).expect("find parses") {
            lunahook_rs::protocol::HostCommand::FindHook(cmd) => {
                let units: Vec<u16> = cmd
                    .sp
                    .text
                    .iter()
                    .take_while(|unit| **unit != 0)
                    .copied()
                    .collect();
                assert_eq!(String::from_utf16_lossy(&units), "テスト");
                assert_eq!(cmd.sp.search_time_ms, 30_000);
            }
            _ => panic!("expected FindHook"),
        }
    }

    /// DLL 로그 채널(HostInfoNotif, Text 커맨드와 동일 판별값)이 길이로
    /// 구별되는지 확인한다.
    #[test]
    fn host_info_notification_is_distinguished_by_length() {
        let info_size = size_of::<HostInfoNotif>();
        // TextOutput 헤더는 HostInfoNotif보다 크다 — 길이 구별의 전제.
        assert!(size_of::<TextOutputHeader>() > info_size);

        let mut buffer = vec![0u8; info_size];
        buffer[0] = HostNotificationType::Text as u32 as u8;
        buffer[offset_of!(HostInfoNotif, kind)] =
            lunahook_rs::protocol::HostInfo::Warning as u32 as u8;
        let message = b"cannot install";
        buffer[offset_of!(HostInfoNotif, message)
            ..offset_of!(HostInfoNotif, message) + message.len()]
            .copy_from_slice(message);

        match parse_notification(&buffer).expect("parses") {
            Notification::Info { warning, message } => {
                assert!(warning);
                assert_eq!(message, "cannot install");
            }
            other => panic!("expected Info, got {other:?}"),
        }
    }
}
