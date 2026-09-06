//! 자석 모드 시작/중지, 대상 선택, 안내 오버레이 표시.

use windows_core::Error;
use windows_sys::Win32::{
    Foundation::{E_FAIL, HWND},
    UI::WindowsAndMessaging::{KillTimer, SetTimer},
};

use super::io_to_windows_error;
use crate::app::state::{self, OverlayNotice};
use crate::app::{App, MAGNETIC_NOTICE_DURATION_MS, MAGNETIC_NOTICE_TIMER};
use crate::dialogs::SettingsDialog;
use crate::magnetic::MagneticManager;
use crate::window;

type Result<T> = windows_core::Result<T>;

impl App {
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
                    return Err(io_to_windows_error(error));
                }
                self.magnetic = Some(magnetic);
                self.model.config.magnetic_mode = true;
                if !self.show_overlay_notice(OverlayNotice::SelectMagneticTarget) {
                    if let Some(mut magnetic) = self.magnetic.take() {
                        magnetic.stop();
                    }
                    self.model.config.magnetic_mode = false;
                    return Err(Error::new(
                        windows_core::HRESULT(E_FAIL),
                        "클립보드 감시를 일시 정지할 수 없습니다",
                    ));
                }
            }
            state::MagneticAction::Stop => {
                if let Some(mut magnetic) = self.magnetic.take() {
                    magnetic.stop();
                }
                self.model.config.magnetic_mode = false;
                self.clear_overlay_notice();
            }
        }
        Ok(())
    }

    pub(in crate::app) fn apply_magnetic_request(&mut self, enabled: bool) {
        if let Err(error) = self.set_magnetic_enabled(enabled) {
            tracing::error!("Failed to apply magnetic mode request: {error}");
        }
        self.sync_magnetic_checkbox();
        if let Err(error) = self.model.config.save() {
            tracing::error!("Failed to persist magnetic mode: {error}");
        }
    }

    pub(in crate::app) fn handle_magnetic_target_selected(&mut self, target: HWND) {
        if !self.model.config.magnetic_mode {
            return;
        }
        let result = self
            .magnetic
            .as_mut()
            .ok_or_else(|| {
                Error::new(
                    windows_core::HRESULT(E_FAIL),
                    "자석 선택기가 실행 중이 아닙니다",
                )
            })
            .and_then(|magnetic| magnetic.attach(target).map_err(io_to_windows_error));
        if let Err(error) = result {
            tracing::error!("Failed to attach magnetic target: {error}");
            if let Some(mut magnetic) = self.magnetic.take() {
                magnetic.stop();
            }
            self.model.config.magnetic_mode = false;
            self.clear_overlay_notice();
        } else if !self.show_overlay_notice(OverlayNotice::MagneticTargetAttached) {
            if let Some(mut magnetic) = self.magnetic.take() {
                magnetic.stop();
            }
            self.model.config.magnetic_mode = false;
            self.clear_overlay_notice();
        }
        self.sync_magnetic_checkbox();
        if let Err(error) = self.model.config.save() {
            tracing::error!("Failed to persist selected magnetic target: {error}");
        }
    }

    pub(in crate::app) fn handle_magnetic_notice_timer(&mut self) {
        self.clear_overlay_notice();
    }

    fn show_overlay_notice(&mut self, notice: OverlayNotice) -> bool {
        unsafe {
            let _ = KillTimer(self.hwnd, MAGNETIC_NOTICE_TIMER);
        }
        self.model.runtime.overlay_notice = Some(notice);
        window::set_window_visible(self.hwnd, true);
        if !self.pause_clipboard_capture("자석 대상 선택") {
            self.model.runtime.overlay_notice = None;
            window::set_window_visible(self.hwnd, self.model.config.window_visible);
            return false;
        }
        if let Err(error) = self.paint() {
            tracing::warn!("Failed to paint magnetic notice: {error}");
        }
        if notice == OverlayNotice::MagneticTargetAttached {
            let timer = unsafe {
                SetTimer(
                    self.hwnd,
                    MAGNETIC_NOTICE_TIMER,
                    MAGNETIC_NOTICE_DURATION_MS,
                    None,
                )
            };
            if timer == 0 {
                tracing::warn!("Failed to start magnetic notice timer");
                self.clear_overlay_notice();
            }
        }
        true
    }

    fn clear_overlay_notice(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, MAGNETIC_NOTICE_TIMER);
        }
        if self.model.runtime.overlay_notice.take().is_none() {
            return;
        }
        window::set_window_visible(self.hwnd, self.model.config.window_visible);
        if let Err(error) = self.paint() {
            tracing::warn!("Failed to repaint after magnetic notice: {error}");
        }
        self.resume_clipboard_capture();
    }

    pub(super) fn sync_magnetic_checkbox(&self) {
        if let Some(hwnd) = SettingsDialog::current_hwnd() {
            SettingsDialog::set_magnetic_checked(hwnd, self.model.config.magnetic_mode);
        }
    }
}
