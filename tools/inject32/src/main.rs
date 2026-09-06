//! x86 게임용 DLL 인젝션 헬퍼.
//!
//! x64 anemone은 WOW64 경계를 넘어 32비트 프로세스에 CreateRemoteThread를 쓸
//! 수 없다. 이 헬퍼는 i686으로 빌드되어 32비트 세계에서 같은 절차를 실행한다:
//! OpenProcess → VirtualAllocEx → WriteProcessMemory(DLL 경로) →
//! CreateRemoteThread(LoadLibraryW).
//!
//! kernel32는 부팅 단위로 모든 프로세스에서 같은 base에 매핑되므로(문서화된
//! 동작), 이 프로세스의 LoadLibraryW 주소가 대상에서도 유효하다.

#![windows_subsystem = "console"]
#![deny(unsafe_op_in_unsafe_fn)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
};
use windows_sys::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, OpenProcess, WaitForSingleObject,
};

const PROCESS_CREATE_THREAD: u32 = 0x0002;
const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
const PROCESS_VM_OPERATION: u32 = 0x0008;
const PROCESS_VM_WRITE: u32 = 0x0020;
const PROCESS_VM_READ: u32 = 0x0010;

/// LoadLibraryW 원격 스레드 대기 한도(밀리초).
const INJECTION_TIMEOUT_MS: u32 = 10_000;

#[derive(Debug, PartialEq, Eq)]
enum WaitStatus {
    Completed,
    Timeout,
    Failed(u32),
    Unexpected(u32),
}

/// WaitForSingleObject 결과를 Win32 오류와 timeout으로 분리한다. 호출자는
/// WAIT_FAILED인 경우 이 함수를 부르기 전에 GetLastError를 보존해야 한다.
fn classify_wait_result(wait: u32, error_code: u32) -> WaitStatus {
    match wait {
        WAIT_OBJECT_0 => WaitStatus::Completed,
        WAIT_TIMEOUT => WaitStatus::Timeout,
        WAIT_FAILED => WaitStatus::Failed(error_code),
        other => WaitStatus::Unexpected(other),
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 4 && args[1] == "--uninject" {
        let Ok(pid) = args[2].parse::<u32>() else {
            fail("pid가 올바르지 않습니다");
        };
        let Ok(module) = args[3].parse::<u32>() else {
            fail("모듈 핸들이 올바르지 않습니다");
        };
        if let Err(reason) = uninject(pid, module) {
            fail(&reason);
        }
        return;
    }
    if args.len() != 3 {
        fail("사용법: anemone_inject32.exe <pid> <dll_path> 또는 --uninject <pid> <module>");
    }
    let Ok(pid) = args[1].parse::<u32>() else {
        fail("pid가 올바르지 않습니다");
    };
    let dll_path = args[2].clone();
    if !std::path::Path::new(&dll_path).is_file() {
        fail(&format!("DLL이 없습니다: {dll_path}"));
    }

    match inject(pid, &dll_path) {
        Ok(module) => println!("{module}"),
        Err(reason) => fail(&reason),
    }
}

type LoadLibraryWFn = unsafe extern "system" fn(*const u16) -> u32;

fn inject(pid: u32, dll_path: &str) -> Result<u32, String> {
    // SAFETY: 각 Win32 호출은 직전 단계의 핸들/포인터만 사용하며, 실패 시
    // 이미 할당한 자원을 즉시 정리한다. GetLastError 검증도 같은 스레드의
    // 실패 직후에 이뤄진다.
    unsafe {
        let rights = PROCESS_CREATE_THREAD
            | PROCESS_QUERY_INFORMATION
            | PROCESS_VM_OPERATION
            | PROCESS_VM_WRITE
            | PROCESS_VM_READ;
        let process = OpenProcess(rights, 0, pid);
        if process.is_null() {
            return Err(format!(
                "OpenProcess({pid}) 실패: {} (관리자 권한 필요일 수 있음)",
                GetLastError()
            ));
        }
        let result = inject_into(process, dll_path);
        let _ = CloseHandle(process);
        result
    }
}

fn uninject(pid: u32, module: u32) -> Result<(), String> {
    unsafe {
        let rights = PROCESS_CREATE_THREAD
            | PROCESS_QUERY_INFORMATION
            | PROCESS_VM_OPERATION
            | PROCESS_VM_WRITE
            | PROCESS_VM_READ;
        let process = OpenProcess(rights, 0, pid);
        if process.is_null() {
            return Err(format!("OpenProcess({pid}) 실패: {}", GetLastError()));
        }
        let kernel32 = GetModuleHandleW(windows_sys::core::w!("kernel32.dll"));
        if kernel32.is_null() {
            let error = GetLastError();
            let _ = CloseHandle(process);
            return Err(format!("kernel32 모듈 조회 실패: {error}"));
        }
        let Some(address) = GetProcAddress(kernel32, c"FreeLibrary".as_ptr().cast()) else {
            let _ = CloseHandle(process);
            return Err("FreeLibrary 주소를 찾지 못했습니다".to_string());
        };
        let free_library: unsafe extern "system" fn(*mut c_void) -> u32 =
            std::mem::transmute(address);
        let thread = CreateRemoteThread(
            process,
            std::ptr::null(),
            0,
            Some(free_library),
            module as usize as *mut c_void,
            0,
            std::ptr::null_mut(),
        );
        if thread.is_null() {
            let _ = CloseHandle(process);
            return Err(format!(
                "FreeLibrary 원격 스레드 생성 실패: {}",
                GetLastError()
            ));
        }
        let wait = WaitForSingleObject(thread, INJECTION_TIMEOUT_MS);
        // WAIT_FAILED 직후 GetLastError를 보존한다. CloseHandle/다른 호출 뒤에는
        // 이 값이 덮어써질 수 있다.
        let error_code = if wait == WAIT_FAILED { GetLastError() } else { 0 };
        let wait_status = classify_wait_result(wait, error_code);
        let result = match wait_status {
            WaitStatus::Completed => {
                let mut exit_code = 0u32;
                if GetExitCodeThread(thread, &mut exit_code) == 0 {
                    Err("GetExitCodeThread 실패".to_string())
                } else if exit_code == 0 {
                    Err("FreeLibrary가 실패했습니다".to_string())
                } else {
                    Ok(())
                }
            }
            WaitStatus::Timeout => Err("FreeLibrary 대기가 제한 시간을 초과했습니다".to_string()),
            WaitStatus::Failed(error) => {
                Err(format!("WaitForSingleObject 실패: Win32 오류 {error}"))
            }
            WaitStatus::Unexpected(status) => Err(format!(
                "WaitForSingleObject 예기치 않은 반환값: 0x{status:08X}"
            )),
        };
        let _ = CloseHandle(thread);
        let _ = CloseHandle(process);
        result
    }
}

/// # Safety
/// `process`는 위 권한 조합으로 연 유효한 핸들이어야 한다.
unsafe fn inject_into(process: *mut c_void, dll_path: &str) -> Result<u32, String> {
    unsafe {
        // GetModuleHandleW(null)은 exe 자신의 핸들을 돌려주므로 kernel32를
        // 이름으로 명시해야 한다.
        let kernel32 = GetModuleHandleW(windows_sys::core::w!("kernel32.dll"));
        if kernel32.is_null() {
            return Err("kernel32 모듈을 찾지 못했습니다".to_string());
        }
        let load_library_address = GetProcAddress(kernel32, c"LoadLibraryW".as_ptr().cast());
        let Some(load_library_address) = load_library_address else {
            return Err("LoadLibraryW 주소를 찾지 못했습니다".to_string());
        };
        let load_library: LoadLibraryWFn = std::mem::transmute(load_library_address);

        let mut path_utf16: Vec<u16> = std::ffi::OsStr::new(dll_path).encode_wide().collect();
        path_utf16.push(0);

        let bytes = std::slice::from_raw_parts(
            path_utf16.as_ptr().cast::<u8>(),
            path_utf16.len() * size_of::<u16>(),
        );

        let remote = VirtualAllocEx(
            process,
            std::ptr::null(),
            bytes.len(),
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        );
        if remote.is_null() {
            return Err(format!("VirtualAllocEx 실패: {}", GetLastError()));
        }
        if WriteProcessMemory(
            process,
            remote,
            bytes.as_ptr().cast(),
            bytes.len(),
            std::ptr::null_mut(),
        ) == 0
        {
            let _ = VirtualFreeEx(process, remote, 0, MEM_RELEASE);
            return Err(format!("WriteProcessMemory 실패: {}", GetLastError()));
        }

        let thread_start: unsafe extern "system" fn(*mut c_void) -> u32 =
            std::mem::transmute(load_library);
        let thread = CreateRemoteThread(
            process,
            std::ptr::null(),
            0,
            Some(thread_start),
            remote.cast_const(),
            0,
            std::ptr::null_mut(),
        );
        if thread.is_null() {
            let error = GetLastError();
            let _ = VirtualFreeEx(process, remote, 0, MEM_RELEASE);
            return Err(format!("CreateRemoteThread 실패: {error}"));
        }

        let wait = WaitForSingleObject(thread, INJECTION_TIMEOUT_MS);
        // WAIT_FAILED의 실제 오류를 CloseHandle보다 먼저 확보한다.
        let error_code = if wait == WAIT_FAILED { GetLastError() } else { 0 };
        match classify_wait_result(wait, error_code) {
            WaitStatus::Completed => {}
            WaitStatus::Timeout => {
                let _ = CloseHandle(thread);
                // 원격 스레드가 아직 DLL 경로를 읽는 중일 수 있으므로 완료가
                // 확인되지 않은 버퍼는 남겨 use-after-free를 피한다.
                return Err("인젝션이 제한 시간 안에 끝나지 않았습니다".to_string());
            }
            WaitStatus::Failed(error) => {
                let _ = CloseHandle(thread);
                return Err(format!("WaitForSingleObject 실패: Win32 오류 {error}"));
            }
            WaitStatus::Unexpected(status) => {
                let _ = CloseHandle(thread);
                return Err(format!(
                    "WaitForSingleObject 예기치 않은 반환값: 0x{status:08X}"
                ));
            }
        }
        let mut exit_code = 0u32;
        let got_exit = GetExitCodeThread(thread, &mut exit_code);
        let _ = CloseHandle(thread);
        let _ = VirtualFreeEx(process, remote, 0, MEM_RELEASE);
        if got_exit == 0 {
            return Err("GetExitCodeThread 실패".to_string());
        }
        if exit_code == 0 {
            return Err("LoadLibraryW가 실패했습니다 (DLL 의존성/아키텍처 확인 필요)".to_string());
        }
        Ok(exit_code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_failed_preserves_error_and_is_not_timeout() {
        assert_eq!(classify_wait_result(WAIT_FAILED, 6), WaitStatus::Failed(6));
        assert_eq!(classify_wait_result(WAIT_TIMEOUT, 0), WaitStatus::Timeout);
        assert_eq!(classify_wait_result(WAIT_OBJECT_0, 0), WaitStatus::Completed);
    }
}
