//! 화면에 보이는 top-level 창 목록 수집 (후킹 관리용).
//!
//! 후킹 대상은 "사람이 게임이라 판단할 창"이므로, 제목 있는 보이는 창만
//! 나열하고 자기 자신은 제외한다. 비트니스 표시는 참고용이며 권한이 없어
//! 판별에 실패하면 `None`으로 둔다.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows_sys::Win32::System::Threading::{
    IsWow64Process, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, IsWindowVisible};
use windows_sys::core::BOOL;

use super::Arch;

#[derive(Debug, Clone)]
pub struct ProcessEntry {
    pub pid: u32,
    /// 창 제목.
    pub title: String,
    /// 실행 파일 이름 (예: `game.exe`). 알 수 없으면 빈 문자열.
    pub name: String,
    /// 비트니스. 권한 부족 등으로 판별 실패 시 `None`.
    pub arch: Option<Arch>,
}

thread_local! {
    static COLLECTED: RefCell<Vec<(HWND, u32)>> = const { RefCell::new(Vec::new()) };
}

/// 후킹 대상 후보 목록. 제목 순 정렬.
pub fn visible_windows() -> Vec<ProcessEntry> {
    COLLECTED.with(|slot| slot.borrow_mut().clear());
    // SAFETY: EnumWindows는 콜백에 시스템이 만든 유효한 HWND만 넘긴다.
    let result = unsafe { EnumWindows(Some(collect_callback), 0) };
    if result == 0 {
        return Vec::new();
    }
    let raw = COLLECTED.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
    let self_pid = std::process::id();
    let mut entries: Vec<ProcessEntry> = raw
        .into_iter()
        .filter(|(hwnd, pid)| {
            *pid != self_pid && window_title(*hwnd).is_some_and(|title| !title.is_empty())
        })
        .map(|(hwnd, pid)| ProcessEntry {
            pid,
            title: window_title(hwnd).unwrap_or_default(),
            name: process_image_name(pid),
            arch: process_arch(pid),
        })
        .collect();
    deduplicate_processes(&mut entries);
    entries.sort_by_key(|entry| entry.title.to_lowercase());
    entries
}

fn deduplicate_processes(entries: &mut Vec<ProcessEntry>) {
    let mut seen_pids = HashSet::new();
    entries.retain(|entry| seen_pids.insert(entry.pid));
}

extern "system" fn collect_callback(hwnd: HWND, _lparam: LPARAM) -> BOOL {
    // SAFETY: hwnd는 EnumWindows가 제공한 유효한 핸들이다.
    let visible = unsafe { IsWindowVisible(hwnd) };
    if visible != 0 {
        COLLECTED.with(|slot| slot.borrow_mut().push((hwnd, window_pid(hwnd))));
    }
    1
}

fn window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: 유효한 HWND와 출력 버퍼다.
    unsafe {
        let _ =
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, &mut pid);
    }
    pid
}

fn window_title(hwnd: HWND) -> Option<String> {
    let mut buffer = [0u16; 512];
    // SAFETY: buffer는 쓰기 가능 버퍼다.
    let len = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    (len > 0).then(|| String::from_utf16_lossy(&buffer[..len as usize]))
}

fn process_image_name(pid: u32) -> String {
    process_image_full_path(pid)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_string()
}

/// 프로세스 실행 파일의 전체 경로. 권한 부족 등으로 실패하면 `None`.
pub(crate) fn process_image_full_path(pid: u32) -> Option<PathBuf> {
    // SAFETY: OpenProcess~CloseHandle까지 같은 스레드에서 핸들을 소유한다.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let result =
            QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut len);
        let succeeded = result != 0;
        let _ = CloseHandle(process);
        if !succeeded {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(
            &buffer[..len as usize],
        )))
    }
}

fn process_arch(pid: u32) -> Option<Arch> {
    // SAFETY: OpenProcess~CloseHandle까지 같은 스레드에서 핸들을 소유한다.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut wow64 = BOOL::default();
        let ok = IsWow64Process(process, &mut wow64);
        let _ = CloseHandle(process);
        (ok != 0).then_some(if wow64 != 0 { Arch::X86 } else { Arch::X64 })
    }
}

#[cfg(test)]
#[path = "../../tests/unit/hook/process_list.rs"]
mod tests;
