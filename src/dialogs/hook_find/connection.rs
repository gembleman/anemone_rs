//! 대상 프로세스 목록, 연결/해제, 연결 상태에 따른 컨트롤 활성화.

use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetDlgItem, LB_RESETCONTENT, LB_SETCURSEL, SendMessageW, SetWindowTextW,
};

use crate::hook::process_list;
use crate::hook::{Arch, HookRequest};

use super::view::{add_list_string, list_selection};
use super::{
    CONNECTING_TEXT, DETACHING_TEXT, DISCONNECTED_TEXT, HookFindDialog, ctrl_id, set_enabled,
};
use crate::dialogs::helpers::show_error_message;

impl HookFindDialog {
    pub(super) fn refresh_targets(&mut self) {
        self.targets = process_list::visible_windows();
        unsafe {
            let _ = SendMessageW(self.targets_list, LB_RESETCONTENT, 0, 0);
        }
        for entry in &self.targets {
            let arch = entry.arch.map_or("?", Arch::label);
            let name = if entry.name.is_empty() {
                "?"
            } else {
                entry.name.as_str()
            };
            add_list_string(
                self.targets_list,
                &format!("{} — {} ({arch})", entry.title, name),
            );
        }
        unsafe {
            set_enabled(self.connect_btn, !self.targets.is_empty());
            if !self.targets.is_empty() {
                let _ = SendMessageW(self.targets_list, LB_SETCURSEL, 0, 0);
            }
        }
    }

    pub(super) fn connect_selected(&mut self) {
        let Some(entry) = list_selection(self.targets_list)
            .and_then(|index| self.targets.get(index))
            .cloned()
        else {
            return;
        };
        tracing::info!(pid = entry.pid, title = %entry.title, "후킹 연결 요청");
        if self
            .hook
            .request(HookRequest::Attach {
                pid: entry.pid,
                process_name: entry.name,
            })
            .is_err()
        {
            show_error_message(
                self.hwnd,
                "후킹 연결 실패",
                "후킹 워커가 이미 종료되었습니다.",
            );
            return;
        }
        self.enter_pending_state(CONNECTING_TEXT);
    }

    pub(super) fn detach(&mut self) {
        if self.hook.request(HookRequest::Detach).is_err() {
            show_error_message(
                self.hwnd,
                "후킹 중지 실패",
                "후킹 워커가 이미 종료되었습니다.",
            );
            return;
        }
        self.enter_pending_state(DETACHING_TEXT);
    }

    pub(super) fn enter_pending_state(&mut self, text: &str) {
        unsafe {
            let text = crate::win32::to_wide(text);
            let _ = SetWindowTextW(self.status, text.as_ptr());
            set_enabled(self.targets_list, false);
            set_enabled(self.refresh_btn, false);
            set_enabled(self.connect_btn, false);
        }
        self.set_management_enabled(false);
    }

    pub(super) fn sync_connection_ui(&mut self) {
        let connected = self.target_label.is_some();
        let label = self.target_label.as_deref().unwrap_or(DISCONNECTED_TEXT);
        unsafe {
            let text = crate::win32::to_wide(label);
            let _ = SetWindowTextW(self.status, text.as_ptr());
            set_enabled(self.targets_list, true);
            set_enabled(self.refresh_btn, true);
            set_enabled(self.connect_btn, !self.targets.is_empty());
        }
        self.set_management_enabled(connected);
    }

    pub(super) fn set_management_enabled(&mut self, enabled: bool) {
        const CONTROLS: [u16; 11] = [
            ctrl_id::GROUP_MANUAL,
            ctrl_id::MANUAL_CODE,
            ctrl_id::BTN_MANUAL_INSTALL,
            ctrl_id::GROUP_STREAMS,
            ctrl_id::STREAMS,
            ctrl_id::PREVIEW,
            ctrl_id::GROUP_SEARCH,
            ctrl_id::SEARCH_TEXT,
            ctrl_id::BTN_TEXT_SEARCH,
            ctrl_id::BTN_AUTO_SEARCH,
            ctrl_id::CANDIDATES,
        ];
        unsafe {
            for id in CONTROLS {
                let control = GetDlgItem(self.hwnd, id as i32);
                if !control.is_null() {
                    set_enabled(control, enabled);
                }
            }
            set_enabled(self.detach_btn, enabled);
            set_enabled(
                self.stream_select_btn,
                enabled && list_selection(self.streams_list).is_some(),
            );
            set_enabled(
                self.candidate_install_btn,
                enabled && !self.candidates.is_empty(),
            );
        }
    }

    pub(super) fn attached(&mut self, target_label: String) {
        self.target_label = Some(target_label);
        self.sync_connection_ui();
    }

    pub(super) fn attach_failed(&mut self) {
        self.target_label = None;
        self.clear_session_view();
        self.sync_connection_ui();
    }

    pub(super) fn detached(&mut self) {
        self.target_label = None;
        self.clear_session_view();
        self.sync_connection_ui();
    }

    pub(super) fn clear_session_view(&mut self) {
        self.streams.clear();
        self.stream_indices.clear();
        self.search_in_flight_until = None;
        self.candidates.clear();
        self.installed_candidate = None;
        self.installed_manual = None;
        unsafe {
            let _ = SendMessageW(self.streams_list, LB_RESETCONTENT, 0, 0);
            let _ = SendMessageW(self.candidates_list, LB_RESETCONTENT, 0, 0);
            let _ = SetWindowTextW(self.preview, crate::win32::to_wide("").as_ptr());
            set_enabled(self.stream_select_btn, false);
            set_enabled(self.candidate_install_btn, false);
        }
    }
}
