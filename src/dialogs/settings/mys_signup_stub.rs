use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW, WM_APP};

use super::SettingsDialog;

pub(super) const WM_MYS_SIGNUP_RESULT: u32 = WM_APP + 0x32;

const UNAVAILABLE: &str = "이 빌드에는 MyS Translater 엔진이 들어 있지 않습니다.";

pub(super) struct SignupWorker;

impl SignupWorker {
    pub(super) fn shutdown(&self) {}
}

impl SettingsDialog {
    pub(super) fn handle_mys_purchase_button(&self) {
        self.show_mys_unavailable_notice();
    }

    pub(super) fn handle_mys_free_token_button(&mut self) {
        self.show_mys_unavailable_notice();
    }

    pub(super) fn handle_mys_signup_result(&mut self) {}

    fn show_mys_unavailable_notice(&self) {
        // SAFETY: self.hwnd는 살아 있는 설정 대화상자다.
        unsafe {
            let _ = MessageBoxW(
                self.hwnd,
                crate::win32::to_wide(UNAVAILABLE).as_ptr(),
                crate::win32::to_wide("MyS Translater").as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }
}
