use std::mem::zeroed;

use windows_core::Error;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    UI::WindowsAndMessaging::{WM_LBUTTONUP, WM_RBUTTONUP},
};
type Result<T> = windows_core::Result<T>;
use windows_sys::Win32::{
    Foundation::POINT, Graphics::Gdi::ClientToScreen as ClientToScreenSys,
    UI::WindowsAndMessaging::GetCursorPos,
};

use super::{App, state};
use crate::menu;
use crate::window;

mod clipboard;
mod dialogs;
mod hotkeys;
mod magnetic;

#[cfg(test)]
pub(crate) use hotkeys::command_from_hotkey_id;

pub(crate) fn command_from_menu_id(id: u16) -> Option<state::AppCommand> {
    match id {
        menu::id::WINDOW_SHOW => Some(state::AppCommand::WindowShow),
        menu::id::CLICK_THROUGH => Some(state::AppCommand::ClickThrough),
        menu::id::CLIPBOARD_WATCH => Some(state::AppCommand::ClipboardWatch),
        menu::id::BACKGROUND_TOGGLE => Some(state::AppCommand::BackgroundToggle),
        menu::id::BORDER_TOGGLE => Some(state::AppCommand::BorderToggle),
        menu::id::MAGNETIC_MODE => Some(state::AppCommand::MagneticMode),
        menu::id::SETTINGS => Some(state::AppCommand::Settings),
        menu::id::TRANSLATE => Some(state::AppCommand::Translate),
        menu::id::BACKLOG => Some(state::AppCommand::Backlog),
        menu::id::FILE_TRANS => Some(state::AppCommand::FileTrans),
        menu::id::TEXT_SIZE_UP => Some(state::AppCommand::TextSizeUp),
        menu::id::TEXT_SIZE_DOWN => Some(state::AppCommand::TextSizeDown),
        menu::id::HOOK_FIND => Some(state::AppCommand::HookFind),
        menu::id::HOOK_STOP => Some(state::AppCommand::HookStop),
        menu::id::EXIT => Some(state::AppCommand::Exit),
        _ => None,
    }
}

impl App {
    fn show_context_menu(&mut self, x: i32, y: i32) -> Result<()> {
        self.context_menu_active = true;
        if !self.pause_clipboard_capture("컨텍스트 메뉴") {
            self.context_menu_active = false;
            return Ok(());
        }
        let result = (|| {
            let hook_active = self.model.runtime.hook_session.is_some();
            self.menu
                .build(&self.model.config, hook_active)
                .map_err(io_to_windows_error)?;
            if let Some(command) = self
                .menu
                .show(self.hwnd, x, y)
                .map_err(io_to_windows_error)?
            {
                self.handle_menu_command(command)?;
            }
            Ok(())
        })();
        self.context_menu_active = false;
        self.resume_clipboard_capture();
        result
    }

    pub(in crate::app) fn handle_menu_command(&mut self, cmd: u16) -> Result<()> {
        let Some(command) = command_from_menu_id(cmd) else {
            return Ok(());
        };

        let effects = self.model.update(state::AppAction::Command(command));
        self.run_effects(effects);
        Ok(())
    }

    /// 트레이 아이콘 이벤트 처리
    ///
    /// # Safety
    /// `hwnd`는 유효한 윈도우 핸들이어야 한다.
    pub(in crate::app) unsafe fn handle_tray_event(&mut self, lparam: LPARAM) {
        unsafe {
            match lparam as u32 {
                WM_LBUTTONUP => {
                    self.model.config.window_visible = true;
                    window::set_window_visible(self.hwnd, true);
                }
                WM_RBUTTONUP => {
                    let mut pt: POINT = zeroed();
                    let _ = GetCursorPos(&mut pt);
                    if let Err(e) = self.show_context_menu(pt.x, pt.y) {
                        tracing::warn!("tray show_context_menu failed: {e}");
                    }
                }
                _ => {}
            }
        }
    }

    /// 우클릭 컨텍스트 메뉴 처리
    ///
    /// # Safety
    /// `hwnd`는 유효한 윈도우 핸들이어야 한다.
    pub(in crate::app) unsafe fn handle_right_click(
        &mut self,
        hwnd: HWND,
        msg: u32,
        lparam: LPARAM,
    ) {
        unsafe {
            let mut pt = POINT {
                x: (lparam & 0xFFFF) as i16 as i32,
                y: ((lparam >> 16) & 0xFFFF) as i16 as i32,
            };
            if msg == WM_RBUTTONUP {
                let _ = ClientToScreenSys(hwnd, &mut pt);
            }
            if let Err(e) = self.show_context_menu(pt.x, pt.y) {
                tracing::warn!("show_context_menu failed: {e}");
            }
        }
    }
}

fn io_to_windows_error(error: std::io::Error) -> Error {
    let code = error.raw_os_error().unwrap_or(1) as u32;
    Error::new(
        windows_core::HRESULT((0x80070000u32 | code) as i32),
        "Win32 error",
    )
}
