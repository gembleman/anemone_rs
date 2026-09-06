use std::io;
use windows_sys::Win32::{
    Foundation::{GetLastError, HWND, SetLastError},
    UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DeleteMenu, DestroyMenu, GetMenuItemCount, MF_BYPOSITION,
        MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, PostMessageW, SetForegroundWindow,
        TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
    },
};

use crate::config::Config;

fn popup_flags() -> u32 {
    TPM_LEFTALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY
}

// 메뉴 ID 정의
pub mod id {
    pub const WINDOW_SHOW: u16 = 101;
    pub const CLICK_THROUGH: u16 = 103;
    pub const CLIPBOARD_WATCH: u16 = 104;
    pub const BACKGROUND_TOGGLE: u16 = 105;
    pub const BORDER_TOGGLE: u16 = 106;
    pub const MAGNETIC_MODE: u16 = 107;
    pub const SETTINGS: u16 = 108;
    pub const BACKLOG: u16 = 109;
    pub const TRANSLATE: u16 = 111;
    pub const FILE_TRANS: u16 = 112;
    pub const HOOK_FIND: u16 = 114;
    pub const HOOK_STOP: u16 = 115;
    pub const EXIT: u16 = 110;

    // 텍스트 크기 조절
    pub const TEXT_SIZE_UP: u16 = 201;
    pub const TEXT_SIZE_DOWN: u16 = 202;
}

/// 체크 상태에 따른 메뉴 플래그
#[inline]
fn checked_flag(checked: bool) -> u32 {
    if checked {
        MF_STRING | MF_CHECKED
    } else {
        MF_STRING
    }
}

/// 활성/비활성 상태에 따른 메뉴 플래그
#[inline]
pub fn enabled_flag(enabled: bool) -> u32 {
    if enabled {
        MF_STRING
    } else {
        MF_GRAYED | MF_STRING
    }
}

/// "클립보드 감시" 항목의 플래그.
///
/// 후킹 세션 중에는 클립보드 감시가 자동으로 멈추므로(`app::state::
/// clipboard_capture_is_paused`) 지금 토글해도 즉시 달라지는 게 없다. 헛클릭을
/// 막기 위해 항목을 비활성화하되, 체크 표시는 저장된 설정 그대로 둔다 — 후킹을
/// 끊으면 그 설정으로 되돌아가기 때문이다.
#[inline]
fn clipboard_watch_flag(watching: bool, hook_active: bool) -> u32 {
    checked_flag(watching) | if hook_active { MF_GRAYED } else { 0 }
}

pub struct ContextMenu {
    hmenu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
}

impl ContextMenu {
    pub fn new() -> io::Result<Self> {
        // SAFETY: CreatePopupMenu requires no preconditions and returns a new valid menu handle.
        unsafe {
            let hmenu = CreatePopupMenu();
            if hmenu.is_null() {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { hmenu })
        }
    }

    /// 항목 하나를 끝에 덧붙인다.
    ///
    /// 트레이 메뉴를 통째로 만드는 `build`와 달리, 목록 컨트롤의 우클릭 메뉴처럼
    /// 호출부가 항목을 직접 구성하는 팝업에 쓴다.
    pub fn append(&self, flags: u32, id: u16, text: &str) -> io::Result<()> {
        let wide = crate::win32::to_wide(text);
        // SAFETY: self.hmenu는 new()가 만든 유효한 팝업 메뉴이고, wide는 이 호출이
        // 끝날 때까지 살아 있는 NUL 종료 문자열이다.
        if unsafe { AppendMenuW(self.hmenu, flags, id as usize, wide.as_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn build(&self, config: &Config, hook_active: bool) -> io::Result<()> {
        // SAFETY: self.hmenu is a valid menu handle created by CreatePopupMenu. All
        // AppendMenuW calls use valid menu item IDs and static string literals (w! macro).
        // DeleteMenu with MF_BYPOSITION and index 0 removes items from the front.
        unsafe {
            macro_rules! append {
                ($flags:expr, $id:expr, $text:expr) => {{
                    let wide = crate::win32::to_wide($text);
                    if AppendMenuW(self.hmenu, $flags, $id as usize, wide.as_ptr()) == 0 {
                        return Err(io::Error::last_os_error());
                    }
                }};
                ($flags:expr, $id:expr) => {{
                    if AppendMenuW(self.hmenu, $flags, $id as usize, std::ptr::null()) == 0 {
                        return Err(io::Error::last_os_error());
                    }
                }};
            }
            // 메뉴 초기화 (기존 항목 제거)
            while GetMenuItemCount(self.hmenu) > 0 {
                let _ = DeleteMenu(self.hmenu, 0, MF_BYPOSITION);
            }

            // 윈도우 표시/숨김
            let show_text = if config.window_visible {
                "윈도우 숨기기"
            } else {
                "윈도우 표시"
            };
            append!(MF_STRING, id::WINDOW_SHOW, show_text);
            append!(MF_SEPARATOR, 0);

            append!(
                checked_flag(config.click_through),
                id::CLICK_THROUGH,
                "클릭 통과"
            );
            append!(
                clipboard_watch_flag(config.clipboard_watch, hook_active),
                id::CLIPBOARD_WATCH,
                "클립보드 감시"
            );
            append!(
                checked_flag(config.magnetic_mode),
                id::MAGNETIC_MODE,
                "자석 모드"
            );
            append!(MF_SEPARATOR, 0);

            append!(
                checked_flag(config.background_visible),
                id::BACKGROUND_TOGGLE,
                "배경 표시"
            );
            append!(
                checked_flag(config.border_visible),
                id::BORDER_TOGGLE,
                "테두리 표시"
            );
            append!(MF_SEPARATOR, 0);

            append!(MF_STRING, id::TRANSLATE, "번역");
            append!(MF_STRING, id::FILE_TRANS, "파일 번역");
            append!(MF_STRING, id::HOOK_FIND, "후킹 관리…");
            append!(enabled_flag(hook_active), id::HOOK_STOP, "후킹 중지");
            append!(MF_STRING, id::BACKLOG, "백로그");
            append!(MF_STRING, id::SETTINGS, "설정");
            append!(MF_SEPARATOR, 0);

            append!(MF_STRING, id::EXIT, "종료");
        }
        Ok(())
    }

    /// Show the popup without sending `WM_COMMAND` to the owner window.
    ///
    /// Returning the selected command keeps menu tracking from re-entering the app's
    /// `RefCell<App>` through a synchronous owner notification.
    pub fn show(&self, hwnd: HWND, x: i32, y: i32) -> io::Result<Option<u16>> {
        // SAFETY: hwnd is a valid window handle from the caller. self.hmenu is a valid
        // popup menu handle. SetForegroundWindow and TrackPopupMenu use valid handles.
        // PostMessageW with WM_NULL is the standard pattern to dismiss the menu properly.
        unsafe {
            let _ = SetForegroundWindow(hwnd);
            // With TPM_RETURNCMD, zero means either cancellation or failure. Clear the
            // thread error first so a non-zero value afterwards can be reported precisely.
            SetLastError(0);
            let command =
                TrackPopupMenu(self.hmenu, popup_flags(), x, y, 0, hwnd, std::ptr::null());
            if command == 0 {
                let error = GetLastError();
                if error != 0 {
                    return Err(io::Error::from_raw_os_error(error as i32));
                }
            }
            let _ = PostMessageW(hwnd, WM_NULL, 0, 0);
            Ok((command != 0).then_some(command as u16))
        }
    }
}

impl Drop for ContextMenu {
    fn drop(&mut self) {
        // SAFETY: self.hmenu is a valid menu handle created by CreatePopupMenu in new().
        // DestroyMenu is the correct cleanup for popup menus and is idempotent.
        unsafe {
            let _ = DestroyMenu(self.hmenu);
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/menu.rs"]
mod tests;
