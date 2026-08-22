//! 후킹 대상 프로세스 선택 대화상자.
//!
//! 보이는 top-level 창을 나열하고 선택한 pid로 attach 요청을 보낸다. attach
//! 자체는 워커 스레드가 수행하며(수 초 걸릴 수 있음) 이 창은 즉시 닫힌다.
//! 결과는 WM_APP_HOOK_STATE 이벤트로 안내된다.

use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::Input::KeyboardAndMouse::EnableWindow,
        UI::WindowsAndMessaging::{
            GetDlgItem, LB_ADDSTRING, LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL, SendMessageW,
            WM_CLOSE, WM_COMMAND,
        },
    },
    core::*,
};

use super::host::{DialogHost, DialogResult, HostedDialog};
use crate::hook::process_list::{self, ProcessEntry};
use crate::hook::{Arch, HookRequest, HookWorker};

mod ctrl_id {
    pub const DIALOG: u16 = 107;
    pub const LIST: u16 = 6001;
    pub const BTN_REFRESH: u16 = 6010;
    pub const BTN_CONNECT: u16 = 6011;
}

pub struct HookSelectDialog {
    hwnd: HWND,
    list: HWND,
    connect_btn: HWND,
    entries: Vec<ProcessEntry>,
    hook: Rc<HookWorker>,
}

pub(crate) struct HookSelectInit {
    hook: Rc<HookWorker>,
}

impl HostedDialog for HookSelectDialog {
    type Init = HookSelectInit;
    const RESOURCE_ID: u16 = ctrl_id::DIALOG;

    fn create(hwnd: HWND, init: Self::Init) -> Result<Self> {
        let mut dialog = Self {
            hwnd,
            list: unsafe { GetDlgItem(Some(hwnd), ctrl_id::LIST as i32) }?,
            connect_btn: unsafe { GetDlgItem(Some(hwnd), ctrl_id::BTN_CONNECT as i32) }?,
            entries: Vec::new(),
            hook: init.hook,
        };
        dialog.refresh_list();
        Ok(dialog)
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> DialogResult {
        match msg {
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as u16;
                match id {
                    ctrl_id::BTN_REFRESH => self.refresh_list(),
                    ctrl_id::BTN_CONNECT => self.connect_selected(),
                    _ => {}
                }
                DialogResult::Handled(LRESULT(1))
            }
            WM_CLOSE => DialogResult::Close(LRESULT(1)),
            _ => DialogResult::Unhandled,
        }
    }

    fn destroy(&mut self) {}
}

impl HookSelectDialog {
    pub(crate) fn show(parent: HWND, hook: Rc<HookWorker>) -> Result<HWND> {
        DialogHost::<Self>::show(parent, HookSelectInit { hook })
    }

    fn refresh_list(&mut self) {
        self.entries = process_list::visible_windows();
        // SAFETY: self.list는 RC 템플릿의 유효한 컨트롤 핸들이다.
        unsafe {
            let _ = SendMessageW(self.list, LB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
            for entry in &self.entries {
                let arch = entry.arch.map_or("?", Arch::label);
                let name = if entry.name.is_empty() {
                    "?"
                } else {
                    entry.name.as_str()
                };
                let line = format!("{} — {} ({arch})", entry.title, name);
                let wide: Vec<u16> = line.encode_utf16().chain([0]).collect();
                let _ = SendMessageW(
                    self.list,
                    LB_ADDSTRING,
                    Some(WPARAM(0)),
                    Some(LPARAM(wide.as_ptr() as isize)),
                );
            }
            let has_items = !self.entries.is_empty();
            let _ = EnableWindow(self.connect_btn, has_items);
            if has_items {
                let _ = SendMessageW(self.list, LB_SETCURSEL, Some(WPARAM(0)), Some(LPARAM(0)));
            }
        }
    }

    fn connect_selected(&mut self) {
        // SAFETY: list는 유효한 핸들이고 반환값은 목록 인덱스다.
        let selection =
            unsafe { SendMessageW(self.list, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 };
        if selection < 0 {
            return;
        }
        let Some(entry) = self.entries.get(selection as usize) else {
            return;
        };
        tracing::info!(pid = entry.pid, title = %entry.title, "후킹 연결 요청");
        if let Err(error) = self.hook.request(HookRequest::Attach {
            pid: entry.pid,
            process_name: entry.name.clone(),
        }) {
            tracing::error!("후킹 워커가 종료되어 attach 요청을 보낼 수 없습니다: {error:?}");
            return;
        }

        // SAFETY: hwnd는 아직 살아 있는 모덜리스 dialog 창이다.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(self.hwnd),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}
