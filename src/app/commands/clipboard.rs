//! 클립보드 감시 일시정지/재개 정책과 창 상태(magnetic·topmost·visibility) 동기화.

use crate::app::{App, state};
use crate::dialogs::SettingsDialog;
use crate::window;

/// 클립보드 일시정지 세션 식별자를 하나 발급하고 다음 값을 계산한다.
///
/// `0`은 "세션 없음"을 뜻하는 sentinel이라 절대 발급하지 않는다 — `u64::MAX`에서
/// `wrapping_add(1)`하면 자연스럽게 0이 나오므로, 그 경우에만 1로 건너뛴다.
fn advance_pause_session_counter(current: u64) -> (u64, u64) {
    let issued = current;
    let mut next = current.wrapping_add(1);
    if next == 0 {
        next = 1;
    }
    (issued, next)
}

impl App {
    pub(in crate::app) fn clipboard_capture_paused(&self) -> bool {
        state::clipboard_capture_is_paused(
            self.translate_dialog_session.is_some(),
            self.file_trans_dialog_session.is_some(),
            self.settings_dialog_active,
            self.context_menu_active,
            self.model.runtime.overlay_notice.is_some(),
            self.model.runtime.hook_session.is_some(),
        )
    }

    pub(super) fn next_clipboard_pause_session(&mut self) -> u64 {
        let (session, next) = advance_pause_session_counter(self.next_clipboard_pause_session);
        self.next_clipboard_pause_session = next;
        session
    }

    pub(in crate::app) fn pause_clipboard_capture(&mut self, reason: &str) -> bool {
        if self.clipboard.is_watching()
            && let Err(error) = self.clipboard.stop()
        {
            tracing::error!("Failed to pause clipboard listener for {reason}: {error}");
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "클립보드 감시 오류",
                &format!("{reason} 사용 중 클립보드 감시를 중지하지 못했습니다.\n\n{error}"),
            );
            return false;
        }
        self.cancel_clipboard_translation();
        true
    }

    pub(in crate::app) fn resume_clipboard_capture(&mut self) {
        if self.clipboard_capture_paused()
            || !self.model.config.clipboard_watch
            || self.clipboard.is_watching()
        {
            return;
        }
        if let Err(error) = self.clipboard.start() {
            self.model.config.clipboard_watch = false;
            tracing::error!("Failed to resume clipboard listener after temporary pause: {error}");
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "클립보드 감시 오류",
                &format!("클립보드 감시를 재개하지 못했습니다.\n\n{error}"),
            );
        }
        if let Some(hwnd) = SettingsDialog::current_hwnd() {
            SettingsDialog::set_clipboard_checked(hwnd, self.model.config.clipboard_watch);
        }
    }

    /// Config 기반 magnetic, click-through, topmost, visibility, clipboard 정책을 적용한다.
    pub(in crate::app) fn sync_window_state(&mut self) {
        let cfg = &self.model.config;
        let click_through = cfg.click_through;
        let topmost = cfg.window_topmost;
        let configured_visible = cfg.window_visible;
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
        if magnetic_changed && !self.services.config_save.request(self.model.config.clone()) {
            tracing::error!("Failed to request magnetic mode persistence");
        }

        let visible = configured_visible || self.model.runtime.overlay_notice.is_some();
        let watch = state::should_watch_clipboard(
            self.model.config.clipboard_watch,
            self.clipboard_capture_paused(),
        );

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

    pub(in crate::app) fn apply_clipboard_watch(&mut self, enabled: bool) {
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
}

#[cfg(test)]
#[path = "../../../tests/unit/app/commands/clipboard.rs"]
mod tests;
