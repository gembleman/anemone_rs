use std::mem::zeroed;

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, POINT, WPARAM},
        Graphics::Gdi::ClientToScreen,
        UI::WindowsAndMessaging::{
            GetCursorPos, IsWindow, PostMessageW, SetForegroundWindow, WM_CLOSE, WM_LBUTTONUP,
            WM_RBUTTONUP,
        },
    },
    core::Result,
};

use super::{App, state};
use crate::dialogs::{
    BacklogDialog, FileTransDialog, HookSettingsDialog, SettingsDialog, TranslateDialog,
};
use crate::hotkey::HotkeyManager;
use crate::magnetic::MagneticManager;
use crate::window;

impl App {
    fn show_context_menu(&mut self, x: i32, y: i32) -> Result<()> {
        self.menu.build(&self.config.borrow())?;
        if let Some(command) = self.menu.show(self.hwnd, x, y)? {
            self.handle_menu_command(command)?;
        }
        Ok(())
    }

    pub(super) fn handle_menu_command(&mut self, cmd: u16) -> Result<()> {
        let Some(command) = state::AppCommand::from_menu_id(cmd) else {
            return Ok(());
        };

        match command {
            state::AppCommand::WindowShow => {
                let mut cfg = self.config.borrow_mut();
                cfg.toggle_window_visible();
                let visible = cfg.window_visible;
                drop(cfg);
                window::set_window_visible(self.hwnd, visible);
            }
            state::AppCommand::ClickThrough => {
                let mut cfg = self.config.borrow_mut();
                cfg.toggle_click_through();
                let click_through = cfg.click_through;
                drop(cfg);
                window::set_click_through(self.hwnd, click_through);
            }
            state::AppCommand::ClipboardWatch => {
                let mut cfg = self.config.borrow_mut();
                cfg.toggle_clipboard_watch();
                let watch = cfg.clipboard_watch;
                drop(cfg);
                if watch {
                    self.clipboard.start();
                } else {
                    self.clipboard.stop();
                    self.cancel_clipboard_translation();
                }
            }
            state::AppCommand::BackgroundToggle => {
                self.config.borrow_mut().toggle_background_visible();
                self.paint()?;
            }
            state::AppCommand::BorderToggle => {
                self.config.borrow_mut().toggle_border_visible();
                self.paint()?;
            }
            state::AppCommand::MagneticMode => {
                let enabled = !self.config.borrow().magnetic_mode;
                self.apply_magnetic_request(enabled);
            }
            state::AppCommand::Settings => {
                self.open_settings_dialog();
            }
            state::AppCommand::Translate => {
                self.open_translate_dialog();
            }
            state::AppCommand::Backlog => {
                self.open_backlog_dialog();
            }
            state::AppCommand::FileTrans => {
                self.open_file_trans_dialog();
            }
            state::AppCommand::HookSettings => {
                self.open_hook_settings_dialog();
            }
            state::AppCommand::TextSizeUp => {
                let mut cfg = self.config.borrow_mut();
                let new_size = (cfg.translation_style.size + 1).min(100);
                cfg.translation_style.size = new_size;
                cfg.name_style.size = new_size;
                cfg.original_style.size = new_size;
                drop(cfg);
                self.paint()?;
            }
            state::AppCommand::TextSizeDown => {
                let mut cfg = self.config.borrow_mut();
                let new_size = (cfg.translation_style.size - 1).max(6);
                cfg.translation_style.size = new_size;
                cfg.name_style.size = new_size;
                cfg.original_style.size = new_size;
                drop(cfg);
                self.paint()?;
            }
            // DestroyWindow의 동기 재진입은 RefCell borrow와 충돌한다. WM_CLOSE를
            // queue에 넣어 borrow가 풀린 다음 정상 종료 흐름을 시작한다.
            state::AppCommand::Exit => unsafe {
                if let Err(e) = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) {
                    tracing::error!("PostMessageW(WM_CLOSE) failed: {e}");
                }
            },
        }
        Ok(())
    }

    /// 기존 dialog에는 focus하고 없으면 새로 만든다.
    fn open_dialog_generic<F, E>(hwnd_storage: &mut Option<HWND>, dialog_name: &str, create_fn: F)
    where
        F: FnOnce() -> std::result::Result<HWND, E>,
        E: std::fmt::Display,
    {
        if let Some(hwnd) = *hwnd_storage {
            // SAFETY: hwnd was previously returned by a successful dialog creation call.
            // IsWindow validates it is still a valid window before use.
            unsafe {
                if IsWindow(Some(hwnd)).as_bool() {
                    let _ = SetForegroundWindow(hwnd);
                    return;
                }
            }
        }

        match create_fn() {
            Ok(hwnd) => {
                *hwnd_storage = Some(hwnd);
            }
            Err(e) => {
                tracing::error!("Failed to open {} dialog: {}", dialog_name, e);
            }
        }
    }

    /// 설정 대화상자 열기
    fn open_settings_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let config = self.config.clone();
        Self::open_dialog_generic(&mut self.dialogs.settings, "settings", || {
            SettingsDialog::show(main_hwnd, config, None)
        });
    }

    /// 번역 대화상자 열기
    fn open_translate_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let config = self.config.clone();
        Self::open_dialog_generic(&mut self.dialogs.translate, "translate", || {
            TranslateDialog::show(main_hwnd, config)
        });
    }

    /// 백로그 대화상자 열기
    fn open_backlog_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let store = self.backlog_store.clone();
        Self::open_dialog_generic(&mut self.dialogs.backlog, "backlog", || {
            BacklogDialog::show(main_hwnd, store)
        });
    }

    /// 파일 번역 대화상자 열기
    fn open_file_trans_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let config = self.config.clone();
        Self::open_dialog_generic(&mut self.dialogs.file_trans, "file_trans", || {
            FileTransDialog::show(main_hwnd, config)
        });
    }

    /// 후크 설정 대화상자 열기
    fn open_hook_settings_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let config = self.config.clone();
        Self::open_dialog_generic(&mut self.dialogs.hook_settings, "hook_settings", || {
            HookSettingsDialog::show(main_hwnd, config)
        });
    }

    /// Config 기반 magnetic, click-through, topmost, visibility, clipboard 정책을 적용한다.
    pub(super) fn sync_window_state(&mut self) {
        let cfg = self.config.borrow();
        let click_through = cfg.click_through;
        let topmost = cfg.window_topmost;
        let visible = cfg.window_visible;
        let watch = cfg.clipboard_watch;
        let magnetic_enabled = cfg.magnetic_mode;
        drop(cfg);

        // Magnetic 연결 실패 시 저장값과 checkbox를 모두 비활성화한다.
        let active_before = self.magnetic.is_some();
        if let Err(error) = self.set_magnetic_enabled(magnetic_enabled) {
            tracing::error!("Failed to synchronize magnetic mode: {error}");
        }
        let magnetic_changed = active_before != self.magnetic.is_some()
            || magnetic_enabled != self.config.borrow().magnetic_mode;
        self.sync_magnetic_checkbox();
        if magnetic_changed && let Err(error) = self.config.borrow().save() {
            tracing::error!("Failed to persist magnetic mode synchronization: {error}");
        }

        window::set_click_through(self.hwnd, click_through);
        window::set_topmost(self.hwnd, topmost);
        window::set_window_visible(self.hwnd, visible);

        if watch && !self.clipboard.is_watching() {
            self.clipboard.start();
        } else if !watch && self.clipboard.is_watching() {
            self.clipboard.stop();
            self.cancel_clipboard_translation();
        }
    }

    /// Idempotently synchronize the persisted setting and the live WinEvent hook.
    pub(super) fn set_magnetic_enabled(&mut self, enabled: bool) -> Result<()> {
        match state::magnetic_action(enabled, self.magnetic.is_some()) {
            state::MagneticAction::Noop => {
                self.config.borrow_mut().magnetic_mode = enabled;
            }
            state::MagneticAction::Start => {
                let mut magnetic = MagneticManager::new(self.hwnd, self.config.clone());
                if let Err(error) = magnetic.start() {
                    self.config.borrow_mut().magnetic_mode = false;
                    return Err(error);
                }
                self.magnetic = Some(magnetic);
                self.config.borrow_mut().magnetic_mode = true;
            }
            state::MagneticAction::Stop => {
                if let Some(mut magnetic) = self.magnetic.take() {
                    magnetic.stop();
                }
                self.config.borrow_mut().magnetic_mode = false;
            }
        }
        Ok(())
    }

    pub(super) fn apply_magnetic_request(&mut self, enabled: bool) {
        if let Err(error) = self.set_magnetic_enabled(enabled) {
            tracing::error!("Failed to apply magnetic mode request: {error}");
        }
        self.sync_magnetic_checkbox();
        if let Err(error) = self.config.borrow().save() {
            tracing::error!("Failed to persist magnetic mode: {error}");
        }
    }

    fn sync_magnetic_checkbox(&self) {
        if let Some(hwnd) = self.dialogs.settings {
            SettingsDialog::set_magnetic_checked(hwnd, self.config.borrow().magnetic_mode);
        }
    }

    pub(super) fn handle_hotkey(&mut self, id: i32) -> Result<()> {
        if let Some(cmd) = HotkeyManager::to_menu_command(id) {
            self.handle_menu_command(cmd)?;
        }
        Ok(())
    }

    /// 트레이 아이콘 이벤트 처리
    ///
    /// # Safety
    /// `hwnd`는 유효한 윈도우 핸들이어야 한다.
    pub(super) unsafe fn handle_tray_event(&mut self, lparam: LPARAM) {
        unsafe {
            match lparam.0 as u32 {
                WM_LBUTTONUP => {
                    self.config.borrow_mut().window_visible = true;
                    window::set_window_visible(self.hwnd, true);
                }
                WM_RBUTTONUP => {
                    let mut pt: POINT = zeroed();
                    GetCursorPos(&mut pt).ok();
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
    pub(super) unsafe fn handle_right_click(&mut self, hwnd: HWND, msg: u32, lparam: LPARAM) {
        unsafe {
            let mut pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            if msg == WM_RBUTTONUP {
                let _ = ClientToScreen(hwnd, &mut pt);
            }
            if let Err(e) = self.show_context_menu(pt.x, pt.y) {
                tracing::warn!("show_context_menu failed: {e}");
            }
        }
    }
}
