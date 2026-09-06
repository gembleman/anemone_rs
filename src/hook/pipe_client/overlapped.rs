//! 오버랩(비동기) 파이프 I/O 1회용 헬퍼.
//!
//! 파이프 핸들이 FILE_FLAG_OVERLAPPED로 만들어지므로 모든 ReadFile/WriteFile/
//! ConnectNamedPipe에 OVERLAPPED가 필요하다. 대기는 이벤트로 하고, 중단
//! 시에는 GetOverlappedResult(bWait)로 커널 사용 종료를 확인한 뒤 이벤트
//! 핸들을 닫아 수명 문제를 없앤다.

use std::io;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};

use super::PipeError;

/// 오버랩 I/O 1회용 헬퍼 — 완료 신호 이벤트를 OVERLAPPED에 결합한다.
pub(super) struct OverlappedOp {
    pub(super) overlapped: OVERLAPPED,
}

impl OverlappedOp {
    pub(super) fn new() -> Result<Self, io::Error> {
        // SAFETY: 이름 없는 수동 리셋 이벤트는 이 프로세스에서만 다룬다.
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
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
    pub(super) fn wait(&self, timeout_ms: u32) -> Result<bool, ()> {
        // SAFETY: 이벤트 핸들은 new()가 만든 유효한 값이고 Drop에서 닫는다.
        match unsafe { WaitForSingleObject(self.overlapped.hEvent, timeout_ms) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(()),
        }
    }

    /// 진행 중일 수 있는 작업을 취소하고 커널이 OVERLAPPED를 다 쓸 때까지
    /// 기다린다 — 스택 OVERLAPPED/이벤트 정리 전의 수명 보장이 목적이다.
    pub(super) fn cancel_and_settle(&self, pipe: HANDLE) {
        // SAFETY: pipe와 overlapped는 방금 건 작업의 유효한 조합이다.
        unsafe {
            let _ = CancelIoEx(pipe, &self.overlapped);
            let mut transferred = 0u32;
            // bWait=true — 취소 완료(ERROR_OPERATION_ABORTED 포함)까지 확실히 기다린다.
            let _ = GetOverlappedResult(pipe, &self.overlapped, &mut transferred, 1);
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
pub(super) fn start_io(result: i32) -> Result<bool, PipeError> {
    if result != 0 {
        return Ok(false);
    }
    if unsafe { GetLastError() } == ERROR_IO_PENDING {
        Ok(true)
    } else {
        Err(PipeError::Disconnected)
    }
}

/// 진행 중인 오버랩 요청을 무한히 기다리고 전송 바이트 수를 돌려준다.
/// 연결 끊김/취소면 Err — 호출자는 이를 세션 종료로 해석한다.
///
/// # Safety
/// `op`는 현재 `pipe`에서 진행 중인 I/O에 결합돼 있어야 한다.
pub(super) unsafe fn finish_io(pipe: HANDLE, op: &OverlappedOp) -> Result<u32, PipeError> {
    // SAFETY: op 내부 이벤트 핸들은 new()가 만든 유효한 값이다.
    unsafe {
        if op.wait(INFINITE).is_err() {
            // 대기 API 실패만으로 커널이 OVERLAPPED 사용을 끝냈다고 볼 수 없다.
            // 스택의 op를 버리기 전에 취소 완료까지 기다린다.
            op.cancel_and_settle(pipe);
            return Err(PipeError::Disconnected);
        }
        let mut transferred = 0u32;
        // bWait=true — 이벤트 신호와 완료 기록 사이의 미세한 창을 흡수한다.
        if GetOverlappedResult(pipe, &op.overlapped, &mut transferred, 1) == 0 {
            return Err(PipeError::Disconnected);
        }
        Ok(transferred)
    }
}

/// 제한 시간 안에 오버랩 작업을 끝내고, 초과하면 커널 I/O가 끝난 뒤
/// OVERLAPPED를 해제한다.
pub(super) unsafe fn finish_io_with_timeout(
    pipe: HANDLE,
    op: &OverlappedOp,
    timeout: Duration,
) -> Result<u32, PipeError> {
    match op.wait(timeout.as_millis().min(u32::MAX as u128) as u32) {
        Ok(true) => unsafe {
            let mut transferred = 0u32;
            if GetOverlappedResult(pipe, &op.overlapped, &mut transferred, 1) == 0 {
                return Err(PipeError::Disconnected);
            }
            Ok(transferred)
        },
        Ok(false) => {
            op.cancel_and_settle(pipe);
            Err(PipeError::Disconnected)
        }
        Err(()) => {
            op.cancel_and_settle(pipe);
            Err(PipeError::Disconnected)
        }
    }
}

/// 직전 Win32 오류를 io::Error로.
pub(super) fn last_win_error() -> io::Error {
    // SAFETY: GetLastError는 사전조건이 없다. 이 함수들은 실패 직후 한 스레드에서
    // 호출되므로 경합으로 값이 변하지 않는다.
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}
