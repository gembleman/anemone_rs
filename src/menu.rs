use windows::{
    Win32::{Foundation::*, UI::WindowsAndMessaging::*},
    core::*,
};

use crate::config::Config;

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
    pub const HOOK_SETTINGS: u16 = 113;
    pub const EXIT: u16 = 110;

    // 텍스트 크기 조절
    pub const TEXT_SIZE_UP: u16 = 201;
    pub const TEXT_SIZE_DOWN: u16 = 202;
}

pub struct ContextMenu {
    hmenu: HMENU,
}

impl ContextMenu {
    pub fn new() -> Result<Self> {
        // SAFETY: CreatePopupMenu requires no preconditions and returns a new valid menu handle.
        unsafe {
            let hmenu = CreatePopupMenu()?;
            Ok(Self { hmenu })
        }
    }

    pub fn build(&self, config: &Config) -> Result<()> {
        /// 체크 상태에 따른 메뉴 플래그
        #[inline]
        fn checked_flag(checked: bool) -> MENU_ITEM_FLAGS {
            if checked {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            }
        }

        // SAFETY: self.hmenu is a valid menu handle created by CreatePopupMenu. All
        // AppendMenuW calls use valid menu item IDs and static string literals (w! macro).
        // DeleteMenu with MF_BYPOSITION and index 0 removes items from the front.
        unsafe {
            // 메뉴 초기화 (기존 항목 제거)
            while GetMenuItemCount(Some(self.hmenu)) > 0 {
                let _ = DeleteMenu(self.hmenu, 0, MF_BYPOSITION);
            }

            // 윈도우 표시/숨김
            let show_text = if config.window_visible {
                w!("윈도우 숨기기")
            } else {
                w!("윈도우 표시")
            };
            AppendMenuW(self.hmenu, MF_STRING, id::WINDOW_SHOW as usize, show_text)?;
            AppendMenuW(self.hmenu, MF_SEPARATOR, 0, None)?;

            AppendMenuW(
                self.hmenu,
                checked_flag(config.click_through),
                id::CLICK_THROUGH as usize,
                w!("클릭 통과"),
            )?;
            AppendMenuW(
                self.hmenu,
                checked_flag(config.clipboard_watch),
                id::CLIPBOARD_WATCH as usize,
                w!("클립보드 감시"),
            )?;
            AppendMenuW(
                self.hmenu,
                checked_flag(config.magnetic_mode),
                id::MAGNETIC_MODE as usize,
                w!("자석 모드"),
            )?;
            AppendMenuW(self.hmenu, MF_SEPARATOR, 0, None)?;

            AppendMenuW(
                self.hmenu,
                checked_flag(config.background_visible),
                id::BACKGROUND_TOGGLE as usize,
                w!("배경 표시"),
            )?;
            AppendMenuW(
                self.hmenu,
                checked_flag(config.border_visible),
                id::BORDER_TOGGLE as usize,
                w!("테두리 표시"),
            )?;
            AppendMenuW(self.hmenu, MF_SEPARATOR, 0, None)?;

            AppendMenuW(self.hmenu, MF_STRING, id::TRANSLATE as usize, w!("번역"))?;
            AppendMenuW(
                self.hmenu,
                MF_STRING,
                id::FILE_TRANS as usize,
                w!("파일 번역"),
            )?;
            AppendMenuW(self.hmenu, MF_STRING, id::BACKLOG as usize, w!("백로그"))?;
            AppendMenuW(
                self.hmenu,
                MF_STRING,
                id::HOOK_SETTINGS as usize,
                w!("후크 설정"),
            )?;
            AppendMenuW(self.hmenu, MF_STRING, id::SETTINGS as usize, w!("설정"))?;
            AppendMenuW(self.hmenu, MF_SEPARATOR, 0, None)?;

            AppendMenuW(self.hmenu, MF_STRING, id::EXIT as usize, w!("종료"))?;

            Ok(())
        }
    }

    pub fn show(&self, hwnd: HWND, x: i32, y: i32) -> Result<()> {
        // SAFETY: hwnd is a valid window handle from the caller. self.hmenu is a valid
        // popup menu handle. SetForegroundWindow and TrackPopupMenu use valid handles.
        // PostMessageW with WM_NULL is the standard pattern to dismiss the menu properly.
        unsafe {
            let _ = SetForegroundWindow(hwnd);
            let _ = TrackPopupMenu(
                self.hmenu,
                TPM_LEFTALIGN | TPM_RIGHTBUTTON,
                x,
                y,
                None,
                hwnd,
                None,
            );
            PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0))?;
            Ok(())
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
