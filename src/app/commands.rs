use std::mem::zeroed;

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, POINT},
        Graphics::Gdi::ClientToScreen,
        UI::WindowsAndMessaging::{GetCursorPos, WM_LBUTTONUP, WM_RBUTTONUP},
    },
    core::Result,
};

use super::{App, state};
use crate::dialogs::{BacklogDialog, FileTransDialog, SettingsDialog, TranslateDialog};
use crate::hotkey::HotkeyManager;
use crate::magnetic::MagneticManager;
use crate::window;

impl App {
    fn show_context_menu(&mut self, x: i32, y: i32) -> Result<()> {
        self.menu.build(&self.model.config)?;
        if let Some(command) = self.menu.show(self.hwnd, x, y)? {
            self.handle_menu_command(command)?;
        }
        Ok(())
    }

    pub(super) fn handle_menu_command(&mut self, cmd: u16) -> Result<()> {
        let Some(command) = state::AppCommand::from_menu_id(cmd) else {
            return Ok(());
        };

        let effects = self.model.update(state::AppAction::Command(command));
        self.run_effects(effects);
        Ok(())
    }

    /// 각 dialog의 instance registry가 기존 창 focus와 새 창 생성을 책임진다.
    fn open_dialog_generic<F, E>(dialog_name: &str, create_fn: F)
    where
        F: FnOnce() -> std::result::Result<HWND, E>,
        E: std::fmt::Display,
    {
        match create_fn() {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Failed to open {} dialog: {}", dialog_name, e);
            }
        }
    }

    /// 설정 대화상자 열기
    pub(super) fn open_settings_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let config = self.model.config.clone();
        let actions = self.action_sender();
        Self::open_dialog_generic("settings", || {
            SettingsDialog::show(main_hwnd, config, Some(actions))
        });
    }

    fn clipboard_capture_paused(&self) -> bool {
        self.translate_dialog_session.is_some() || self.file_trans_dialog_session.is_some()
    }

    fn next_clipboard_pause_session(&mut self) -> u64 {
        let session = self.next_clipboard_pause_session;
        self.next_clipboard_pause_session = self.next_clipboard_pause_session.wrapping_add(1);
        if self.next_clipboard_pause_session == 0 {
            self.next_clipboard_pause_session = 1;
        }
        session
    }

    fn pause_clipboard_for_translation_dialog(&mut self, dialog_title: &str) -> bool {
        if self.clipboard.is_watching()
            && let Err(error) = self.clipboard.stop()
        {
            tracing::error!("Failed to pause clipboard listener for {dialog_title}: {error}");
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "클립보드 감시 오류",
                &format!(
                    "{dialog_title}을(를) 여는 동안 클립보드 감시를 중지하지 못했습니다.\n\n{error}"
                ),
            );
            return false;
        }
        self.cancel_clipboard_translation();
        true
    }

    /// 번역 대화상자 열기
    pub(super) fn open_translate_dialog(&mut self) {
        if let Some(session) = TranslateDialog::current_session() {
            self.translate_dialog_session = Some(session);
            if !self.pause_clipboard_for_translation_dialog("번역 창") {
                return;
            }
            let main_hwnd = self.hwnd;
            let config = self.model.config.clone();
            let translation = self.services.translation_ui.clone();
            let actions = self.action_sender();
            Self::open_dialog_generic("translate", || {
                TranslateDialog::show(main_hwnd, config, translation, actions, session)
            });
            return;
        }
        // 파괴 알림 action보다 재열기 명령이 먼저 도착한 경우 이전 session을 폐기한다.
        self.translate_dialog_session = None;

        // 창 생성보다 먼저 listener와 이미 예약된 자동 번역을 멈춰, 초기화 중 발생한
        // clipboard 변경도 수동 번역 경로로만 소비되게 한다.
        if !self.pause_clipboard_for_translation_dialog("번역 창") {
            return;
        }

        let session = self.next_clipboard_pause_session();
        self.translate_dialog_session = Some(session);

        let main_hwnd = self.hwnd;
        let config = self.model.config.clone();
        let translation = self.services.translation_ui.clone();
        let actions = self.action_sender();
        if let Err(error) = TranslateDialog::show(main_hwnd, config, translation, actions, session)
        {
            self.translate_dialog_session = None;
            tracing::error!("Failed to open translate dialog: {error}");
            self.resume_clipboard_after_translation_dialogs();
        }
    }

    pub(super) fn handle_translate_dialog_closed(&mut self, session: u64) {
        if self.translate_dialog_session != Some(session) {
            return;
        }
        self.translate_dialog_session = None;
        self.resume_clipboard_after_translation_dialogs();
    }

    fn resume_clipboard_after_translation_dialogs(&mut self) {
        if self.clipboard_capture_paused()
            || !self.model.config.clipboard_watch
            || self.clipboard.is_watching()
        {
            return;
        }
        if let Err(error) = self.clipboard.start() {
            self.model.config.clipboard_watch = false;
            tracing::error!(
                "Failed to resume clipboard listener after translation dialog: {error}"
            );
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "클립보드 감시 오류",
                &format!("번역 창을 닫은 뒤 클립보드 감시를 재개하지 못했습니다.\n\n{error}"),
            );
        }
        if let Some(hwnd) = SettingsDialog::current_hwnd() {
            SettingsDialog::set_clipboard_checked(hwnd, self.model.config.clipboard_watch);
        }
    }

    /// 백로그 대화상자 열기
    pub(super) fn open_backlog_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let store = self.model.backlog.clone();
        let actions = self.action_sender();
        Self::open_dialog_generic("backlog", || BacklogDialog::show(main_hwnd, store, actions));
    }

    /// 파일 번역 대화상자 열기
    pub(super) fn open_file_trans_dialog(&mut self) {
        if let Some(session) = FileTransDialog::current_session() {
            self.file_trans_dialog_session = Some(session);
            if !self.pause_clipboard_for_translation_dialog("파일 번역 창") {
                return;
            }
            let main_hwnd = self.hwnd;
            let config = self.model.config.clone();
            let supervisor = self.services.file_translation.clone();
            let actions = self.action_sender();
            Self::open_dialog_generic("file_trans", || {
                FileTransDialog::show(main_hwnd, config, supervisor, actions, session)
            });
            return;
        }
        self.file_trans_dialog_session = None;

        if !self.pause_clipboard_for_translation_dialog("파일 번역 창") {
            return;
        }
        let session = self.next_clipboard_pause_session();
        self.file_trans_dialog_session = Some(session);

        let main_hwnd = self.hwnd;
        let config = self.model.config.clone();
        let supervisor = self.services.file_translation.clone();
        let actions = self.action_sender();
        if let Err(error) = FileTransDialog::show(main_hwnd, config, supervisor, actions, session) {
            self.file_trans_dialog_session = None;
            tracing::error!("Failed to open file_trans dialog: {error}");
            self.resume_clipboard_after_translation_dialogs();
        }
    }

    pub(super) fn handle_file_trans_dialog_closed(&mut self, session: u64) {
        if self.file_trans_dialog_session != Some(session) {
            return;
        }
        self.file_trans_dialog_session = None;
        self.resume_clipboard_after_translation_dialogs();
    }

    /// Config 기반 magnetic, click-through, topmost, visibility, clipboard 정책을 적용한다.
    pub(super) fn sync_window_state(&mut self) {
        let cfg = &self.model.config;
        let click_through = cfg.click_through;
        let topmost = cfg.window_topmost;
        let visible = cfg.window_visible;
        let watch =
            state::should_watch_clipboard(cfg.clipboard_watch, self.clipboard_capture_paused());
        let magnetic_enabled = cfg.magnetic_mode;
        // Magnetic 연결 실패 시 저장값과 checkbox를 모두 비활성화한다.
        let active_before = self.magnetic.is_some();
        if let Err(error) = self.set_magnetic_enabled(magnetic_enabled) {
            tracing::error!("Failed to synchronize magnetic mode: {error}");
        }
        if let Some(magnetic) = self.magnetic.as_mut() {
            magnetic.update_policy(
                self.model.config.magnetic_minimize,
                self.model.config.window_visible,
            );
        }
        let magnetic_changed = active_before != self.magnetic.is_some()
            || magnetic_enabled != self.model.config.magnetic_mode;
        self.sync_magnetic_checkbox();
        if magnetic_changed && let Err(error) = self.model.config.save() {
            tracing::error!("Failed to persist magnetic mode synchronization: {error}");
        }

        window::set_click_through(self.hwnd, click_through);
        window::set_topmost(self.hwnd, topmost);
        window::set_window_visible(self.hwnd, visible);

        if watch && !self.clipboard.is_watching() {
            if let Err(error) = self.clipboard.start() {
                self.model.config.clipboard_watch = false;
                tracing::error!("Failed to start clipboard listener: {error}");
                crate::dialogs::helpers::show_error_message(
                    self.hwnd,
                    "클립보드 감시 오류",
                    &format!("클립보드 감시를 시작하지 못했습니다.\n\n{error}"),
                );
            }
        } else if !watch && self.clipboard.is_watching() {
            match self.clipboard.stop() {
                Ok(()) => self.cancel_clipboard_translation(),
                Err(error) => {
                    self.model.config.clipboard_watch = true;
                    tracing::error!("Failed to stop clipboard listener: {error}");
                    crate::dialogs::helpers::show_error_message(
                        self.hwnd,
                        "클립보드 감시 오류",
                        &format!("클립보드 감시를 중지하지 못했습니다.\n\n{error}"),
                    );
                }
            }
        }
        if let Some(hwnd) = SettingsDialog::current_hwnd() {
            SettingsDialog::set_clipboard_checked(hwnd, self.model.config.clipboard_watch);
        }
    }

    pub(super) fn apply_clipboard_watch(&mut self, enabled: bool) {
        let should_watch = state::should_watch_clipboard(enabled, self.clipboard_capture_paused());
        let result = if should_watch {
            self.clipboard.start()
        } else {
            self.clipboard.stop()
        };
        match result {
            Ok(()) => {
                self.model.config.clipboard_watch = enabled;
                if !enabled {
                    self.cancel_clipboard_translation();
                }
            }
            Err(error) => {
                self.model.config.clipboard_watch = !enabled;
                tracing::error!("Failed to change clipboard listener state: {error}");
                crate::dialogs::helpers::show_error_message(
                    self.hwnd,
                    "클립보드 감시 오류",
                    &format!("클립보드 감시 상태를 변경하지 못했습니다.\n\n{error}"),
                );
            }
        }
        if let Some(hwnd) = SettingsDialog::current_hwnd() {
            SettingsDialog::set_clipboard_checked(hwnd, self.model.config.clipboard_watch);
        }
    }

    /// Idempotently synchronize the persisted setting and the live WinEvent hook.
    pub(super) fn set_magnetic_enabled(&mut self, enabled: bool) -> Result<()> {
        match state::magnetic_action(enabled, self.magnetic.is_some()) {
            state::MagneticAction::Noop => {
                self.model.config.magnetic_mode = enabled;
            }
            state::MagneticAction::Start => {
                let mut magnetic = MagneticManager::new(
                    self.hwnd,
                    self.model.config.magnetic_minimize,
                    self.model.config.window_visible,
                );
                if let Err(error) = magnetic.start() {
                    self.model.config.magnetic_mode = false;
                    return Err(error);
                }
                self.magnetic = Some(magnetic);
                self.model.config.magnetic_mode = true;
            }
            state::MagneticAction::Stop => {
                if let Some(mut magnetic) = self.magnetic.take() {
                    magnetic.stop();
                }
                self.model.config.magnetic_mode = false;
            }
        }
        Ok(())
    }

    pub(super) fn apply_magnetic_request(&mut self, enabled: bool) {
        if let Err(error) = self.set_magnetic_enabled(enabled) {
            tracing::error!("Failed to apply magnetic mode request: {error}");
        }
        self.sync_magnetic_checkbox();
        if let Err(error) = self.model.config.save() {
            tracing::error!("Failed to persist magnetic mode: {error}");
        }
    }

    fn sync_magnetic_checkbox(&self) {
        if let Some(hwnd) = SettingsDialog::current_hwnd() {
            SettingsDialog::set_magnetic_checked(hwnd, self.model.config.magnetic_mode);
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
                    self.model.config.window_visible = true;
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
