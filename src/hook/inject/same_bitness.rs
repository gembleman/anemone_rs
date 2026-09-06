//! 같은 비트니스 대상(x64 anemone → x64 게임)에 대한 인젝션/제거.
//!
//! `CreateRemoteThread(LoadLibraryW)` 절차로 DLL을 로드하고, 실패 정리나
//! attach 취소 시에는 대칭적으로 `FreeLibrary`를 원격 스레드로 호출해
//! 되돌린다.

use std::ffi::c_void;
use std::path::Path;

use std::io;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, HMODULE, WAIT_FAILED};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
};
use windows_sys::Win32::System::ProcessStatus::{K32EnumProcessModules, K32GetModuleFileNameExW};
use windows_sys::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, WaitForSingleObject,
};

use super::{InjectError, classify_wait_result, last_win_error, open_process};

/// 현재 프로세스와 같은 비트니스 대상에 DLL을 주입한다.
///
/// # Safety
/// `pid`는 살아 있는 프로세스여야 하고, `dll_path`는 존재하는 파일이어야 한다.
pub unsafe fn inject_same_bitness(pid: u32, dll_path: &Path) -> Result<u64, InjectError> {
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
            let kernel32 = GetModuleHandleW(windows_sys::core::w!("kernel32.dll"));
            if kernel32.is_null() {
                let error = io::Error::last_os_error();
                let _ = CloseHandle(process);
                return Err(InjectError::ThreadFailed(error));
            }
            let address = GetProcAddress(kernel32, c"LoadLibraryW".as_ptr().cast());
            if address.is_none() {
                let _ = CloseHandle(process);
                return Err(InjectError::LoadFailed);
            }
            // SAFETY: 실제 LoadLibraryW(LPCWSTR -> HMODULE, WINAPI)와 동일한 ABI다.
            // 스레드 루틴 타입으로 바꿔 쓰는 것이 CreateRemoteThread의 관례다.
            std::mem::transmute(address)
        };

        // DLL 절대경로(NUL 종결 UTF-16)를 대상 메모리에 복사한다.
        let mut path_utf16: Vec<u16> = dll_path.as_os_str().encode_wide().collect();
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
            let error = io::Error::from_raw_os_error(GetLastError() as i32);
            let _ = CloseHandle(process);
            return Err(InjectError::AllocFailed(error));
        }

        if WriteProcessMemory(
            process,
            remote,
            bytes.as_ptr().cast(),
            bytes.len(),
            std::ptr::null_mut(),
        ) == 0
        {
            let error = io::Error::last_os_error();
            cleanup_remote(&process, remote);
            let _ = CloseHandle(process);
            return Err(InjectError::WriteFailed(error));
        }

        match CreateRemoteThread(
            process,
            std::ptr::null_mut(),
            0,
            Some(load_library),
            remote.cast_const(),
            0,
            std::ptr::null_mut(),
        ) {
            thread if !thread.is_null() => run_injection_thread(thread, &process, remote, dll_path),
            _ => {
                let error = io::Error::last_os_error();
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
    dll_path: &Path,
) -> Result<u64, InjectError> {
    const INJECTION_TIMEOUT_MS: u32 = 10_000;
    // SAFETY: 네 핸들/포인터 모두 호출자 계약 하에 유효하다.
    unsafe {
        let wait = WaitForSingleObject(thread, INJECTION_TIMEOUT_MS);
        // WAIT_FAILED 직후에는 다른 Win32 API가 호출되기 전에 GetLastError를
        // 보존해야 한다. Timeout과 달리 이는 실제 핸들/권한 오류다.
        let wait_result = if wait == WAIT_FAILED {
            let error = last_win_error();
            classify_wait_result(wait, Some(error))
        } else {
            classify_wait_result(wait, None)
        };
        let completed = wait_result.is_ok();
        let result = match wait_result {
            Ok(()) => {
                let mut exit_code = 0u32;
                if GetExitCodeThread(thread, &mut exit_code) == 0 {
                    Err(InjectError::LoadFailed)
                } else {
                    // LoadLibraryW의 반환값(HMODULE)은 0이면 실패다. 스레드 종료
                    // 코드는 DWORD라 x64 HMODULE 전체를 담을 수 없으므로, 성공 후
                    // 대상 프로세스의 모듈 목록에서 실제 포인터를 다시 찾는다.
                    if exit_code == 0 {
                        Err(InjectError::LoadFailed)
                    } else {
                        find_remote_module(*process, dll_path)
                    }
                }
            }
            Err(error) => Err(error),
        };
        let _ = CloseHandle(thread);
        // 대기 실패/시간 초과 시 원격 스레드는 아직 `remote`의 DLL 경로를 읽고
        // 있을 수 있다. 완료가 확인되지 않은 버퍼는 작은 대상 프로세스 누수를
        // 감수하고 남겨 use-after-free를 피한다.
        if completed {
            cleanup_remote(process, remote);
        }
        let _ = CloseHandle(*process);
        result
    }
}

/// LoadLibraryW가 반환한 x64 HMODULE을 대상 프로세스의 모듈 목록에서 찾는다.
/// CreateRemoteThread의 종료 코드는 DWORD이므로 그 값을 포인터로 확장하면
/// 상위 주소가 잘려 FreeLibrary가 실패한다.
unsafe fn find_remote_module(process: HANDLE, dll_path: &Path) -> Result<u64, InjectError> {
    if dll_path.as_os_str().is_empty() {
        return Err(InjectError::LoadFailed);
    }
    let mut modules = vec![std::ptr::null_mut::<c_void>(); 256];
    loop {
        let mut needed = 0u32;
        let module_bytes = (modules.len() * size_of::<HMODULE>()) as u32;
        // SAFETY: process is the live query/read handle opened for injection and
        // modules/needed are valid writable buffers of the advertised sizes.
        let enumerated = unsafe {
            K32EnumProcessModules(process, modules.as_mut_ptr(), module_bytes, &mut needed)
        };
        if enumerated == 0 {
            return Err(InjectError::ThreadFailed(last_win_error()));
        }
        if needed as usize > module_bytes as usize {
            modules.resize(
                (needed as usize / size_of::<HMODULE>()) + 1,
                std::ptr::null_mut(),
            );
            continue;
        }
        let count = (needed as usize / size_of::<HMODULE>()).min(modules.len());
        for module in &modules[..count] {
            let mut path = vec![0u16; 32_768];
            let length = unsafe {
                K32GetModuleFileNameExW(process, *module, path.as_mut_ptr(), path.len() as u32)
            };
            if length == 0 {
                continue;
            }
            let actual = String::from_utf16_lossy(&path[..length as usize]);
            if module_paths_match(dll_path, Path::new(&actual)) {
                return Ok(*module as usize as u64);
            }
        }
        return Err(InjectError::LoadFailed);
    }
}

/// Compare module paths using the full path, not just the DLL filename. Windows
/// paths are case-insensitive and callers may provide a relative or non-canonical
/// spelling, so canonicalize when possible and retain a lexical fallback for
/// paths that cannot be resolved (for example a stale module-list entry).
fn module_paths_match(wanted: &Path, actual: &Path) -> bool {
    let wanted_canonical = std::fs::canonicalize(wanted);
    let actual_canonical = std::fs::canonicalize(actual);
    match (wanted_canonical, actual_canonical) {
        (Ok(wanted), Ok(actual))
            if normalized_windows_path(&wanted) == normalized_windows_path(&actual) =>
        {
            true
        }
        _ => normalized_windows_path(wanted) == normalized_windows_path(actual),
    }
}

fn normalized_windows_path(path: &Path) -> String {
    let mut value = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    if let Some(rest) = value.strip_prefix(r"\\?\unc\") {
        value = format!(r"\\{rest}");
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        value = rest.to_string();
    }
    value
}

/// # Safety
/// `process`는 유효한 VM 권한 핸들이어야 한다.
unsafe fn cleanup_remote(process: &HANDLE, remote: *mut c_void) {
    // SAFETY: remote는 VirtualAllocEx로 방금 할당한 페이지 범위의 시작점이다.
    unsafe {
        let _ = VirtualFreeEx(*process, remote, 0, MEM_RELEASE);
    }
}

/// 실패한 attach에서 방금 로드한 DLL을 같은 비트니스 대상에서 제거한다.
/// `module`은 LoadLibraryW 원격 스레드의 반환값(HMODULE)이다.
///
/// # Safety
/// `pid`는 살아 있는 같은 비트니스 프로세스이고 `module`은 그 프로세스의
/// 유효한 DLL 모듈 핸들이어야 한다.
pub unsafe fn uninject_same_bitness(pid: u32, module: u64) -> Result<(), InjectError> {
    unsafe {
        let process = open_process(pid)?;
        let result = (|| {
            let kernel32 = GetModuleHandleW(windows_sys::core::w!("kernel32.dll"));
            if kernel32.is_null() {
                return Err(InjectError::ThreadFailed(io::Error::last_os_error()));
            }
            let address = GetProcAddress(kernel32, c"FreeLibrary".as_ptr().cast());
            if address.is_none() {
                return Err(InjectError::LoadFailed);
            }
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
                return Err(InjectError::ThreadFailed(io::Error::last_os_error()));
            }
            let wait = WaitForSingleObject(thread, 10_000);
            let wait_result = if wait == WAIT_FAILED {
                // Preserve the thread-wait error before CloseHandle or any other
                // Win32 call can overwrite the thread-local last error.
                let error = last_win_error();
                classify_wait_result(wait, Some(error))
            } else {
                classify_wait_result(wait, None)
            };
            let result = match wait_result {
                Ok(()) => {
                    let mut exit_code = 0u32;
                    if GetExitCodeThread(thread, &mut exit_code) == 0 {
                        Err(InjectError::LoadFailed)
                    } else {
                        (exit_code != 0)
                            .then_some(())
                            .ok_or(InjectError::LoadFailed)
                    }
                }
                Err(error) => Err(error),
            };
            let _ = CloseHandle(thread);
            result
        })();
        let _ = CloseHandle(process);
        result
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/hook/inject/same_bitness.rs"]
mod tests;
