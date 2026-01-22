use windows::{
    core::*,
    Win32::{
        Foundation::*,
        UI::WindowsAndMessaging::*,
    },
};

use crate::config::Config;

// 메뉴 ID 정의
pub mod id {
    pub const WINDOW_SHOW: u16 = 101;
    #[allow(dead_code)]
    pub const WINDOW_HIDE: u16 = 102;
    pub const CLICK_THROUGH: u16 = 103;
    pub const CLIPBOARD_WATCH: u16 = 104;
    pub const BACKGROUND_TOGGLE: u16 = 105;
    pub const BORDER_TOGGLE: u16 = 106;
    pub const MAGNETIC_MODE: u16 = 107;
    pub const SETTINGS: u16 = 108;
    pub const BACKLOG: u16 = 109;
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
        unsafe {
            let hmenu = CreatePopupMenu()?;
            Ok(Self { hmenu })
        }
    }

    pub fn build(&self, config: &Config) -> Result<()> {
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
            AppendMenuW(
                self.hmenu,
                MF_STRING,
                id::WINDOW_SHOW as usize,
                show_text,
            )?;

            AppendMenuW(self.hmenu, MF_SEPARATOR, 0, None)?;

            // 클릭 통과
            let click_flags = if config.click_through {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            };
            AppendMenuW(
                self.hmenu,
                click_flags,
                id::CLICK_THROUGH as usize,
                w!("클릭 통과"),
            )?;

            // 클립보드 감시
            let clip_flags = if config.clipboard_watch {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            };
            AppendMenuW(
                self.hmenu,
                clip_flags,
                id::CLIPBOARD_WATCH as usize,
                w!("클립보드 감시"),
            )?;

            // 자석 모드
            let magnet_flags = if config.magnetic_mode {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            };
            AppendMenuW(
                self.hmenu,
                magnet_flags,
                id::MAGNETIC_MODE as usize,
                w!("자석 모드"),
            )?;

            AppendMenuW(self.hmenu, MF_SEPARATOR, 0, None)?;

            // 배경 표시
            let bg_flags = if config.background_visible {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            };
            AppendMenuW(
                self.hmenu,
                bg_flags,
                id::BACKGROUND_TOGGLE as usize,
                w!("배경 표시"),
            )?;

            // 테두리 표시
            let border_flags = if config.border_visible {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            };
            AppendMenuW(
                self.hmenu,
                border_flags,
                id::BORDER_TOGGLE as usize,
                w!("테두리 표시"),
            )?;

            AppendMenuW(self.hmenu, MF_SEPARATOR, 0, None)?;

            // 백로그
            AppendMenuW(
                self.hmenu,
                MF_STRING,
                id::BACKLOG as usize,
                w!("백로그"),
            )?;

            // 설정
            AppendMenuW(
                self.hmenu,
                MF_STRING,
                id::SETTINGS as usize,
                w!("설정"),
            )?;

            AppendMenuW(self.hmenu, MF_SEPARATOR, 0, None)?;

            // 종료
            AppendMenuW(
                self.hmenu,
                MF_STRING,
                id::EXIT as usize,
                w!("종료"),
            )?;

            Ok(())
        }
    }

    pub fn show(&self, hwnd: HWND, x: i32, y: i32) -> Result<()> {
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

    #[allow(dead_code)]
    pub fn handle(&self) -> HMENU {
        self.hmenu
    }
}

impl Drop for ContextMenu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.hmenu);
        }
    }
}
