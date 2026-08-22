//! 대상 프로세스에 lunahook DLL을 주입한다.
//!
//! - 같은 비트니스(x64 anemone → x64 게임): `CreateRemoteThread(LoadLibraryW)`.
//!   kernel32는 부팅 단위로 ASLR base가 모든 프로세스에서 동일하므로(문서화된
//!   동작), 이 프로세스의 LoadLibraryW 주소를 대상에 그대로 쓸 수 있다.
//! - 다른 비트니스(x64 anemone → x86 게임): WOW64 경계를 넘는 원격 스레드
//!   생성은 불가능하므로, 32비트로 빌드된 헬퍼(`tools/inject32`)를 스폰해
//!   같은 절차를 32비트 세계에서 실행한다.

use std::ffi::c_void;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
};
use windows::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, IsWow64Process, OpenProcess, PROCESS_ACCESS_RIGHTS,
    PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ,
    PROCESS_VM_WRITE, WaitForSingleObject,
};
use windows::core::{Error as WinError, PCWSTR};

use super::Arch;

/// 인젝션 실패 유형. UI 안내 문구와 1:1로 대응한다.
#[derive(Debug, thiserror::Error)]
pub enum InjectError {
    #[error("대상 프로세스를 열 수 없습니다 (관리자 권한이 필요할 수 있습니다): {0}")]
    OpenProcess(WinError),
    #[error("대상 프로세스에 메모리를 할당할 수 없습니다: {0}")]
    AllocFailed(WinError),
    #[error("DLL 경로를 대상 프로세스에 쓰지 못했습니다: {0}")]
    WriteFailed(WinError),
    #[error("원격 스레드를 만들지 못했습니다: {0}")]
    ThreadFailed(WinError),
    #[error("LoadLibraryW 스레드가 실패했습니다 (DLL 로드 거부 또는 의존성 누락)")]
    LoadFailed,
    #[error("인젝션이 제한 시간 안에 끝나지 않았습니다")]
    Timeout,
    #[error("x86 게임 인젝터 헬퍼가 없거나 실패했습니다: {0}")]
    Helper(String),
    #[error("프로세스 비트니스를 판별하지 못했습니다: {0}")]
    ArchDetection(WinError),
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
    let mut wow64 = windows::core::BOOL::default();
    // SAFETY: process는 호출자가 보증하는 유효한 핸들이고 wow64는 유효한 출력 버퍼다.
    unsafe {
        IsWow64Process(process, &mut wow64).map_err(InjectError::ArchDetection)?;
        Ok(if wow64.as_bool() {
            Arch::X86
        } else {
            Arch::X64
        })
    }
}

/// 현재 프로세스와 같은 비트니스 대상에 DLL을 주입한다.
///
/// # Safety
/// `pid`는 살아 있는 프로세스여야 하고, `dll_path`는 존재하는 파일이어야 한다.
pub unsafe fn inject_same_bitness(pid: u32, dll_path: &Path) -> Result<(), InjectError> {
    use std::os::windows::ffi::OsStrExt;

    // SAFETY: 아래의 모든 Win32 호출은 같은 스레드에서 순차 실행되며, 각 단계가
    // 실패하면 이전 단계의 자원(VirtualAllocEx 원격 페이지, process 핸들)을 즉시
    // 정리한 뒤 반환한다.
    unsafe {
        let process = open_process(pid)?;

        // kernel32는 부팅 단위로 모든 프로세스에서 같은 base에 매핑되므로 이
        // 프로세스의 함수 주소가 대상에서도 유효하다.
        let load_library: unsafe extern "system" fn(*mut c_void) -> u32 = {
            // GetModuleHandleW(null)은 exe 자신의 핸들을 돌려주므로 kernel32를
            // 이름으로 명시해야 한다.
            let kernel32 = GetModuleHandleW(PCWSTR::from_raw(
                windows::core::w!("kernel32.dll").as_ptr(),
            ))
            .map_err(InjectError::ThreadFailed)?;
            let address = GetProcAddress(kernel32, windows::core::s!("LoadLibraryW"))
                .ok_or(InjectError::LoadFailed)?;
            // SAFETY: 실제 LoadLibraryW(LPCWSTR -> HMODULE, WINAPI)와 동일한 ABI다.
            // 스레드 루틴 타입으로 바꿔 쓰는 것이 CreateRemoteThread의 관례다.
            std::mem::transmute(address)
        };

        // DLL 절대경로(NUL 종결 UTF-16)를 대상 메모리에 복사한다.
        let mut path_utf16: Vec<u16> = dll_path.as_os_str().encode_wide().collect();
        path_utf16.push(0);
        let bytes = std::slice::from_raw_parts(
            path_utf16.as_ptr().cast::<u8>(),
            path_utf16.len() * std::mem::size_of::<u16>(),
        );

        let remote = VirtualAllocEx(
            process,
            None,
            bytes.len(),
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        );
        if remote.is_null() {
            let error = WinError::from(windows::core::HRESULT::from_win32(
                windows::Win32::Foundation::GetLastError().0,
            ));
            let _ = CloseHandle(process);
            return Err(InjectError::AllocFailed(error));
        }

        if let Err(error) =
            WriteProcessMemory(process, remote, bytes.as_ptr().cast(), bytes.len(), None)
        {
            cleanup_remote(&process, remote);
            let _ = CloseHandle(process);
            return Err(InjectError::WriteFailed(error));
        }

        match CreateRemoteThread(
            process,
            None,
            0,
            Some(load_library),
            Some(remote.cast_const()),
            0,
            None,
        ) {
            Ok(thread) => run_injection_thread(thread, &process, remote),
            Err(error) => {
                cleanup_remote(&process, remote);
                let _ = CloseHandle(process);
                Err(InjectError::ThreadFailed(error))
            }
        }
    }
}

/// LoadLibraryW 원격 스레드를 기다리고 결과를 판정한 뒤 정리한다.
///
/// # Safety
/// `thread`/`process`는 유효한 핸들이고 `remote`는 직전에 할당한 대상 메모리다.
unsafe fn run_injection_thread(
    thread: HANDLE,
    process: &HANDLE,
    remote: *mut c_void,
) -> Result<(), InjectError> {
    const INJECTION_TIMEOUT_MS: u32 = 10_000;
    // SAFETY: 네 핸들/포인터 모두 호출자 계약 하에 유효하다.
    unsafe {
        let wait = WaitForSingleObject(thread, INJECTION_TIMEOUT_MS);
        let result = if wait != WAIT_OBJECT_0 {
            Err(InjectError::Timeout)
        } else {
            let mut exit_code = 0u32;
            GetExitCodeThread(thread, &mut exit_code).map_err(|_| InjectError::LoadFailed)?;
            // LoadLibraryW의 반환값(HMODULE)은 0이면 실패다.
            if exit_code == 0 {
                Err(InjectError::LoadFailed)
            } else {
                Ok(())
            }
        };
        let _ = CloseHandle(thread);
        cleanup_remote(process, remote);
        result
    }
}

/// # Safety
/// `process`는 유효한 VM 권한 핸들이어야 한다.
unsafe fn cleanup_remote(process: &HANDLE, remote: *mut c_void) {
    // SAFETY: remote는 VirtualAllocEx로 방금 할당한 페이지 범위의 시작점이다.
    unsafe {
        let _ = VirtualFreeEx(*process, remote, 0, MEM_RELEASE);
    }
}

/// x86 게임용: 32비트 헬퍼 exe를 스폰해 인젝션을 위임한다.
///
/// 헬퍼는 `<helper> <pid> <dll_path>` 인자로 받아 같은 절차를 수행하고,
/// exit code 0으로 성공을 알린다.
pub fn inject_via_helper(helper: &Path, pid: u32, dll_path: &Path) -> Result<(), InjectError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if !helper.is_file() {
        return Err(InjectError::Helper(format!(
            "{} 파일이 없습니다",
            helper.display()
        )));
    }
    let output = std::process::Command::new(helper)
        .arg(pid.to_string())
        .arg(dll_path)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| InjectError::Helper(error.to_string()))?;

    if output.status.success() {
        return Ok(());
    }
    // 헬퍼는 실패 원인을 stderr 한 줄로 남긴다.
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.trim();
    if detail.is_empty() {
        return Err(InjectError::Helper(format!(
            "exit code {}",
            output.status.code().unwrap_or(-1)
        )));
    }
    Err(InjectError::Helper(detail.to_string()))
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
    unsafe { OpenProcess(injection_rights(), false, pid) }.map_err(InjectError::OpenProcess)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_process_is_native_bitness() {
        // SAFETY: 자기 자신의 pid는 항상 유효하다.
        let arch = unsafe { detect_arch(std::process::id()) }.expect("self arch");
        assert_eq!(arch, Arch::X64, "anemone은 x64 전용 빌드다");
    }

    #[test]
    fn helper_missing_file_is_reported() {
        let result = inject_via_helper(
            Path::new("Z:/존재하지않음/inject32.exe"),
            1,
            Path::new("a.dll"),
        );
        assert!(matches!(result, Err(InjectError::Helper(_))));
    }
}
