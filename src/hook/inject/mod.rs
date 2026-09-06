//! 대상 프로세스에 lunahook DLL을 주입한다.
//!
//! - 같은 비트니스(x64 anemone → x64 게임): `CreateRemoteThread(LoadLibraryW)`.
//!   kernel32는 부팅 단위로 ASLR base가 모든 프로세스에서 동일하므로(문서화된
//!   동작), 이 프로세스의 LoadLibraryW 주소를 대상에 그대로 쓸 수 있다.
//!   ([`same_bitness`] 모듈)
//! - 다른 비트니스(x64 anemone → x86 게임): WOW64 경계를 넘는 원격 스레드
//!   생성은 불가능하므로, 32비트로 빌드된 헬퍼(`tools/inject32`)를 스폰해
//!   같은 절차를 32비트 세계에서 실행한다. ([`helper`] 모듈)

mod helper;
mod same_bitness;

use std::io;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Threading::{
    IsWow64Process, OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_CREATE_THREAD,
    PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
};

use super::Arch;

pub use helper::{inject_via_helper, uninject_via_helper};
pub use same_bitness::{inject_same_bitness, uninject_same_bitness};

/// 인젝션 실패 유형. UI 안내 문구와 1:1로 대응한다.
#[derive(Debug, thiserror::Error)]
pub enum InjectError {
    #[error("대상 프로세스를 열 수 없습니다 (관리자 권한이 필요할 수 있습니다): {0}")]
    OpenProcess(io::Error),
    #[error("대상 프로세스에 메모리를 할당할 수 없습니다: {0}")]
    AllocFailed(io::Error),
    #[error("DLL 경로를 대상 프로세스에 쓰지 못했습니다: {0}")]
    WriteFailed(io::Error),
    #[error("원격 스레드를 만들지 못했습니다: {0}")]
    ThreadFailed(io::Error),
    #[error("LoadLibraryW 스레드가 실패했습니다 (DLL 로드 거부 또는 의존성 누락)")]
    LoadFailed,
    #[error("인젝션이 제한 시간 안에 끝나지 않았습니다")]
    Timeout,
    #[error("원격 스레드 대기에 실패했습니다: {0}")]
    WaitFailed(io::Error),
    #[error("x86 게임 인젝터 헬퍼가 없거나 실패했습니다: {0}")]
    Helper(String),
    #[error("프로세스 비트니스를 판별하지 못했습니다: {0}")]
    ArchDetection(io::Error),
}

/// pid의 비트니스를 판별한다. WOW64 하에서 돌면 x86이다.
///
/// # Safety
/// `pid`가 실제 존재하는 프로세스 식별자여야 한다.
pub unsafe fn detect_arch(pid: u32) -> Result<Arch, InjectError> {
    // SAFETY: pid는 호출자가 보증하고, open_process가 돌려준 핸들은 이 함수가
    // 반환되기 전에 닫는다.
    unsafe {
        let process = open_process(pid)?;
        let result = detect_arch_of(process);
        let _ = CloseHandle(process);
        result
    }
}

/// # Safety
/// `process`는 유효한 PROCESS_QUERY_INFORMATION 권한의 핸들이어야 한다.
unsafe fn detect_arch_of(process: HANDLE) -> Result<Arch, InjectError> {
    let mut wow64 = 0i32;
    // SAFETY: process는 호출자가 보증하는 유효한 핸들이고 wow64는 유효한 출력 버퍼다.
    unsafe {
        if IsWow64Process(process, &mut wow64) == 0 {
            return Err(InjectError::ArchDetection(io::Error::last_os_error()));
        }
        Ok(if wow64 != 0 { Arch::X86 } else { Arch::X64 })
    }
}

fn open_process(pid: u32) -> Result<HANDLE, InjectError> {
    // windows crate의 비트플래그 조합은 const 컨텍스트에서 쓸 수 없어 함수로 둔다.
    fn injection_rights() -> PROCESS_ACCESS_RIGHTS {
        PROCESS_CREATE_THREAD
            | PROCESS_QUERY_INFORMATION
            | PROCESS_VM_OPERATION
            | PROCESS_VM_WRITE
            | PROCESS_VM_READ
    }
    // SAFETY: OpenProcess는 권한 조합만 검증하며 추가 사전조건이 없다.
    let process = unsafe { OpenProcess(injection_rights(), 0, pid) };
    if process.is_null() {
        Err(InjectError::OpenProcess(io::Error::last_os_error()))
    } else {
        Ok(process)
    }
}

fn last_win_error() -> io::Error {
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}

/// WaitForSingleObject의 세 가지 의미 있는 반환을 순수하게 분류한다.
/// `WAIT_FAILED`의 error는 호출자가 즉시 보존한 GetLastError를 전달한다.
fn classify_wait_result(wait: u32, wait_error: Option<io::Error>) -> Result<(), InjectError> {
    match wait {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err(InjectError::Timeout),
        WAIT_FAILED => {
            Err(InjectError::WaitFailed(wait_error.unwrap_or_else(|| {
                io::Error::other("WaitForSingleObject failed")
            })))
        }
        other => Err(InjectError::WaitFailed(io::Error::other(format!(
            "WaitForSingleObject returned unexpected status 0x{other:08X}"
        )))),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/hook/inject/mod.rs"]
mod tests;
