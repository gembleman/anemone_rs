//! 화면에 보이는 top-level 창 목록 수집 (후킹 대상 선택용).
//!
//! 후킹 대상은 "사람이 게임이라 판단할 창"이므로, 제목 있는 보이는 창만
//! 나열하고 자기 자신은 제외한다. 비트니스 표시는 참고용이며 권한이 없어
//! 판별에 실패하면 `None`으로 둔다.

use std::cell::RefCell;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, IsWindowVisible};
use windows::core::PWSTR;

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
    let result = unsafe { EnumWindows(Some(collect_callback), LPARAM(0)) };
    if result.is_err() {
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
    entries.sort_by_key(|entry| entry.title.to_lowercase());
    entries
}

extern "system" fn collect_callback(hwnd: HWND, _lparam: LPARAM) -> windows::core::BOOL {
    // SAFETY: hwnd는 EnumWindows가 제공한 유효한 핸들이다.
    let visible = unsafe { IsWindowVisible(hwnd) };
    if visible.as_bool() {
        COLLECTED.with(|slot| slot.borrow_mut().push((hwnd, window_pid(hwnd))));
    }
    true.into()
}

fn window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: 유효한 HWND와 출력 버퍼다.
    unsafe {
        let _ =
            windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    pid
}

fn window_title(hwnd: HWND) -> Option<String> {
    let mut buffer = [0u16; 512];
    // SAFETY: buffer는 쓰기 가능 버퍼다.
    let len = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    (len > 0).then(|| String::from_utf16_lossy(&buffer[..len as usize]))
}

fn process_image_name(pid: u32) -> String {
    // SAFETY: OpenProcess~CloseHandle까지 같은 스레드에서 핸들을 소유한다.
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
        .map(|_| len);
        let _ = CloseHandle(process);
        match result {
            Ok(len) => {
                let full = String::from_utf16_lossy(&buffer[..len as usize]);
                full.rsplit(['\\', '/'])
                    .next()
                    .unwrap_or_default()
                    .to_string()
            }
            Err(_) => String::new(),
        }
    }
}

fn process_arch(pid: u32) -> Option<Arch> {
    use windows::Win32::System::Threading::IsWow64Process;
    // SAFETY: OpenProcess~CloseHandle까지 같은 스레드에서 핸들을 소유한다.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut wow64 = windows::core::BOOL::default();
        let ok = IsWow64Process(process, &mut wow64);
        let _ = CloseHandle(process);
        ok.ok().map(|_| {
            if wow64.as_bool() {
                Arch::X86
            } else {
                Arch::X64
            }
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists_at_least_console_window_or_empty_without_panic() {
        // 환경에 따라 결과 크기가 다르지만 패닉/크래시가 없어야 한다.
        let _ = super::visible_windows();
    }

    #[test]
    fn self_process_is_excluded() {
        let list = super::visible_windows();
        assert!(list.iter().all(|entry| entry.pid != std::process::id()));
    }
}
