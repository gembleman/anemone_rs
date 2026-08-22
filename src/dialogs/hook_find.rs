//! 후크 찾기 대화상자 — 자동 탐색과 텍스트 검색, 후보 설치.
//!
//! DLL의 FIND_HOOK 파이프라인을 트리거하고 `HookFoundNotif`로 흘러오는
//! 후보를 나열한다. 후보는 UI 스레드에서만 다루므로 thread_local 슬롯으로
//! App(`hook_text.rs`)과 주고받는다.

use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::Input::KeyboardAndMouse::EnableWindow,
        UI::WindowsAndMessaging::{
            GetDlgItem, GetWindowTextLengthW, GetWindowTextW, LB_ADDSTRING, LB_GETCURSEL,
            LB_GETTEXT, LB_GETTEXTLEN, SendMessageW, WM_CLOSE, WM_COMMAND,
        },
    },
    core::*,
};

use super::host::{DialogHost, DialogResult, HostedDialog};
use crate::hook::pipe_client::FoundHook;
use crate::hook::{HookRequest, HookWorker};

mod ctrl_id {
    pub const DIALOG: u16 = 108;
    pub const SEARCH_TEXT: u16 = 6101;
    pub const CANDIDATES: u16 = 6102;
    pub const BTN_TEXT_SEARCH: u16 = 6110;
    pub const BTN_INSTALL: u16 = 6111;
    pub const BTN_AUTO_SEARCH: u16 = 6112;
}

thread_local! {
    /// 아직 설치하지 않은 후보들. App 이벤트 루프가 채우고 dialog가 소비한다.
    static CANDIDATES: std::cell::RefCell<Vec<FoundHook>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// (App 스레드) 발견된 후보를 적립한다. 창이 열려 있으면 즉시 목록에 반영한다.
pub(crate) fn add_candidate(found: FoundHook) {
    let should_refresh = DialogHost::<HookFindDialog>::current_hwnd().is_some();
    CANDIDATES.with(|slot| slot.borrow_mut().push(found));
    if should_refresh {
        DialogHost::<HookFindDialog>::with_state_mut(HookFindDialog::drain_into_list);
    }
}

pub struct HookFindDialog {
    hwnd: HWND,
    candidates_list: HWND,
    install_btn: HWND,
    hook: Rc<HookWorker>,
    /// 이 창에서 마지막으로 설치한 후보 주소. 새 후보를 설치하면 먼저 제거해
    /// 중복 텍스트를 막는다.
    installed: Option<u64>,
}

pub(crate) struct HookFindInit {
    hook: Rc<HookWorker>,
}

impl HostedDialog for HookFindDialog {
    type Init = HookFindInit;
    const RESOURCE_ID: u16 = ctrl_id::DIALOG;

    fn create(hwnd: HWND, init: Self::Init) -> Result<Self> {
        let mut dialog = Self {
            hwnd,
            candidates_list: unsafe { GetDlgItem(Some(hwnd), ctrl_id::CANDIDATES as i32) }?,
            install_btn: unsafe { GetDlgItem(Some(hwnd), ctrl_id::BTN_INSTALL as i32) }?,
            hook: init.hook,
            installed: None,
        };
        dialog.drain_into_list();
        Ok(dialog)
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> DialogResult {
        match msg {
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as u16;
                match id {
                    ctrl_id::BTN_TEXT_SEARCH => self.start_text_search(),
                    ctrl_id::BTN_AUTO_SEARCH => self.start_auto_search(),
                    ctrl_id::BTN_INSTALL => self.install_selected(),
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

impl HookFindDialog {
    pub(crate) fn show(parent: HWND, hook: Rc<HookWorker>) -> Result<HWND> {
        DialogHost::<Self>::show(parent, HookFindInit { hook })
    }

    /// 적립된 후보를 목록에 옮긴다.
    fn drain_into_list(&mut self) {
        let taken = CANDIDATES.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
        if taken.is_empty() {
            return;
        }
        // SAFETY: 모든 컨트롤 핸들은 RC 템플릿에서 읽은 유효한 핸들이다.
        unsafe {
            for found in taken {
                let line = format!(
                    "[{:x}] flags={:x} \"{}\"",
                    found.hook_address, found.hook_type_flags, found.text
                );
                let wide: Vec<u16> = line.encode_utf16().chain([0]).collect();
                let _ = SendMessageW(
                    self.candidates_list,
                    LB_ADDSTRING,
                    Some(WPARAM(0)),
                    Some(LPARAM(wide.as_ptr() as isize)),
                );
            }
            let _ = EnableWindow(self.install_btn, true);
        }
    }

    /// 화면에 보이는 문장으로 텍스트 검색. 비었으면 일반 범용 탐색으로 폴백.
    fn start_text_search(&mut self) {
        let text = read_control_text(self.hwnd, ctrl_id::SEARCH_TEXT);
        if text.trim().is_empty() {
            self.start_auto_search();
            return;
        }
        tracing::info!(%text, "텍스트 검색 시작");
        self.send_search(build_text_search_param(&text));
    }
    /// 후보 수집형 범용 탐색 (텍스트 불필요).
    fn start_auto_search(&mut self) {
        tracing::info!("범용 후크 탐색 시작");
        self.send_search(build_general_search_param());
    }

    fn send_search(&mut self, sp: lunahook_rs::protocol::SearchParam) {
        if let Err(error) = self.hook.request(HookRequest::FindHook(Box::new(sp))) {
            tracing::error!("탐색 요청 실패: {error:?}");
        }
    }

    fn install_selected(&mut self) {
        // SAFETY: candidates_list는 유효한 핸들이고 반환값은 인덱스다.
        let selection = unsafe {
            SendMessageW(
                self.candidates_list,
                LB_GETCURSEL,
                Some(WPARAM(0)),
                Some(LPARAM(0)),
            )
            .0
        };
        if selection < 0 {
            return;
        }
        // 목록 항목에서 주소를 되찾는다 — FoundHook 전체를 보관하는 대신
        // 표시 문자열의 address만 파싱해도 NEW_HOOK에 충분하다.
        // LB_GETTEXT로 정확한 항목을 읽는다.
        let text_len = unsafe {
            SendMessageW(
                self.candidates_list,
                LB_GETTEXTLEN,
                Some(WPARAM(selection as usize)),
                Some(LPARAM(0)),
            )
            .0 as usize
        };
        if text_len == 0 || text_len > 4096 {
            return;
        }
        let mut buffer = vec![0u16; text_len + 1];
        unsafe {
            SendMessageW(
                self.candidates_list,
                LB_GETTEXT,
                Some(WPARAM(selection as usize)),
                Some(LPARAM(buffer.as_mut_ptr() as isize)),
            );
        }
        let line = String::from_utf16_lossy(&buffer);
        let Some(address_hex) = line
            .strip_prefix('[')
            .and_then(|rest| rest.split_once(']'))
            .map(|(address, _)| address)
        else {
            return;
        };
        let Ok(address) = u64::from_str_radix(address_hex, 16) else {
            return;
        };

        // 후보의 원래 플래그는 CANDIDATES 슬롯 순서와 목록 순서가 같으므로
        // 인덱스로 찾을 수 없다(이전 drain으로 소비됨). 대신 표시된 flags를
        // 파싱한다.
        let flags = line
            .split("flags=")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|hex| u64::from_str_radix(hex.trim_start_matches("0x"), 16).ok())
            .unwrap_or(default_candidate_flags());

        let mut hp = lunahook_rs::params::RawHookParam {
            address,
            hook_type: flags,
            ..lunahook_rs::params::RawHookParam::default()
        };
        hp.name[..5].copy_from_slice(b"UserH");

        // 이전 사용자 후크가 있으면 먼저 제거해 중복 텍스트를 막는다.
        if let Some(previous) = self.installed.replace(address)
            && previous != address
        {
            let _ = self.hook.request(HookRequest::RemoveHook(previous));
        }

        tracing::info!(address, flags, "후보 후크 설치");
        let _ = self.hook.request(HookRequest::NewHook(Box::new(hp)));
    }
}

/// 플래그를 파싱하지 못했을 때의 안전한 기본값: UTF-16 문자열 후크.
fn default_candidate_flags() -> u64 {
    (lunahook_rs::params::HookType::USING_STRING | lunahook_rs::params::HookType::CODEC_UTF16)
        .bits()
}

fn build_text_search_param(text: &str) -> lunahook_rs::protocol::SearchParam {
    crate::hook::pipe_client::build_text_search_param(text)
}

fn build_general_search_param() -> lunahook_rs::protocol::SearchParam {
    crate::hook::pipe_client::build_general_search_param()
}

fn read_control_text(dialog: HWND, id: u16) -> String {
    // SAFETY: 유효한 dialog/컨트롤 ID다.
    unsafe {
        let Ok(edit) = GetDlgItem(Some(dialog), id as i32) else {
            return String::new();
        };
        let len = GetWindowTextLengthW(edit) as usize;
        if len == 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; len + 1];
        let written = GetWindowTextW(edit, &mut buffer);
        String::from_utf16_lossy(&buffer[..written.max(0) as usize])
    }
}
