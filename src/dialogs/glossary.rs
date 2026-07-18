//! 글로서리(고정 번역 사전) 편집 대화상자
//!
//! LLM 번역 시 시스템 프롬프트에 주입되는 캐릭터 이름/고유명사 사전을 편집한다.
//! ListBox에 "source → target" 형식으로 표시하고, 두 개의 Edit으로 추가/수정.

use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::{Foundation::*, UI::WindowsAndMessaging::*},
    core::*,
};

use super::helpers::{
    get_window_text, listbox_add_item, listbox_get_sel, listbox_reset, set_window_text,
};
use super::host::{DialogHost, DialogResult, HostedDialog};
use super::models::{GlossaryDraft, SettingsDraft};

mod ctrl_id {
    pub const DIALOG: u16 = 102;
    pub const LIST: u16 = 7001;
    pub const SOURCE_EDIT: u16 = 7002;
    pub const TARGET_EDIT: u16 = 7003;
    pub const BTN_ADD: u16 = 7010;
    pub const BTN_REMOVE: u16 = 7011;
    pub const BTN_APPLY: u16 = 7020;
    pub const BTN_CLOSE: u16 = 7021;
}

/// 글로서리 편집 다이얼로그
pub struct GlossaryDialog {
    hwnd: HWND,
    owner: HWND,
    settings: Rc<RefCell<SettingsDraft>>,
    applied_dpi: u32,
    /// 임시 편집 버퍼 (적용 전까지 Config에 반영하지 않음)
    draft: GlossaryDraft,
}

pub(crate) struct GlossaryInit {
    owner: HWND,
    settings: Rc<RefCell<SettingsDraft>>,
}

impl GlossaryDialog {
    /// `resources/glossary.rc`의 모델리스 DIALOGEX 리소스를 연다.
    pub(crate) fn show(parent: HWND, settings: Rc<RefCell<SettingsDraft>>) -> Result<HWND> {
        DialogHost::<Self>::show(
            parent,
            GlossaryInit {
                owner: parent,
                settings,
            },
        )
    }

    fn initialize_controls(&self) -> Result<()> {
        for id in [
            ctrl_id::LIST,
            ctrl_id::SOURCE_EDIT,
            ctrl_id::TARGET_EDIT,
            ctrl_id::BTN_ADD,
            ctrl_id::BTN_REMOVE,
            ctrl_id::BTN_APPLY,
            ctrl_id::BTN_CLOSE,
        ] {
            // SAFETY: self.hwnd는 WM_INITDIALOG가 전달한 유효한 다이얼로그 핸들이다.
            unsafe { GetDlgItem(Some(self.hwnd), id as i32) }.map_err(|_| {
                Error::new(
                    E_FAIL,
                    format!("글로서리 컨트롤 ID {id}를 찾을 수 없습니다"),
                )
            })?;
        }
        self.populate_listbox();
        Ok(())
    }

    fn handle_dpi_changed(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let new_dpi = (wparam.0 & 0xFFFF) as u32;
        super::helpers::rescale_dialog_children_for_dpi(self.hwnd, self.applied_dpi, new_dpi);
        self.applied_dpi = new_dpi;

        if lparam.0 != 0 {
            // SAFETY: WM_DPICHANGED의 LPARAM은 메시지 처리 동안 유효한 RECT 포인터다.
            unsafe {
                let rect = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }

    fn handle_command(&mut self, cmd: u16, _notify_code: u32) {
        use ctrl_id::*;
        match cmd {
            BTN_APPLY => {
                self.draft.clone().commit(&mut self.settings.borrow_mut());
                let _ = unsafe {
                    PostMessageW(Some(self.owner), WM_GLOSSARY_APPLIED, WPARAM(0), LPARAM(0))
                };
            }
            BTN_ADD => self.add_or_update_entry(),
            BTN_REMOVE => self.remove_selected(),
            LIST => {
                // 리스트 선택 시 Edit에 항목 로드 (편집 흐름 개선)
                let sel = self.listbox_get_sel();
                if sel >= 0 && (sel as usize) < self.draft.entries().len() {
                    let e = &self.draft.entries()[sel as usize];
                    self.set_control_text(SOURCE_EDIT, &e.source);
                    self.set_control_text(TARGET_EDIT, &e.target);
                }
            }
            _ => {}
        }
    }
    fn add_or_update_entry(&mut self) {
        let src = self
            .get_control_text(ctrl_id::SOURCE_EDIT)
            .trim()
            .to_string();
        let tgt = self
            .get_control_text(ctrl_id::TARGET_EDIT)
            .trim()
            .to_string();
        if self.draft.add_or_update(src, tgt).is_none() {
            return;
        }
        self.refresh_listbox();
        self.set_control_text(ctrl_id::SOURCE_EDIT, "");
        self.set_control_text(ctrl_id::TARGET_EDIT, "");
    }

    fn remove_selected(&mut self) {
        let sel = self.listbox_get_sel();
        if sel < 0 || !self.draft.remove(sel as usize) {
            return;
        }
        self.refresh_listbox();
    }

    fn populate_listbox(&self) {
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid listbox handle.
        unsafe {
            let Ok(lb) = GetDlgItem(Some(self.hwnd), ctrl_id::LIST as i32) else {
                return;
            };
            if lb.is_invalid() {
                return;
            }
            for e in self.draft.entries() {
                let line = format!("{} → {}", e.source, e.target);
                listbox_add_item(lb, &line);
            }
        }
    }

    fn refresh_listbox(&self) {
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid listbox handle.
        unsafe {
            let Ok(lb) = GetDlgItem(Some(self.hwnd), ctrl_id::LIST as i32) else {
                return;
            };
            if lb.is_invalid() {
                return;
            }
            listbox_reset(lb);
            for e in self.draft.entries() {
                let line = format!("{} → {}", e.source, e.target);
                listbox_add_item(lb, &line);
            }
        }
    }

    fn listbox_get_sel(&self) -> i32 {
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid listbox handle.
        unsafe {
            let Ok(lb) = GetDlgItem(Some(self.hwnd), ctrl_id::LIST as i32) else {
                return LB_ERR;
            };
            if lb.is_invalid() {
                return LB_ERR;
            }
            listbox_get_sel(lb)
        }
    }

    fn set_control_text(&self, ctrl_id: u16, text: &str) {
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid control handle.
        unsafe {
            if let Ok(ctrl) = GetDlgItem(Some(self.hwnd), ctrl_id as i32)
                && !ctrl.is_invalid()
            {
                let _ = set_window_text(ctrl, text);
            }
        }
    }

    fn get_control_text(&self, ctrl_id: u16) -> String {
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid control handle.
        unsafe {
            let Ok(ctrl) = GetDlgItem(Some(self.hwnd), ctrl_id as i32) else {
                return String::new();
            };
            if ctrl.is_invalid() {
                return String::new();
            }
            get_window_text(ctrl)
        }
    }
}

impl HostedDialog for GlossaryDialog {
    type Init = GlossaryInit;
    const RESOURCE_ID: u16 = ctrl_id::DIALOG;

    fn create(hwnd: HWND, init: Self::Init) -> Result<Self> {
        let draft = GlossaryDraft::from_config(&init.settings.borrow());
        let dialog = Self {
            hwnd,
            owner: init.owner,
            settings: init.settings,
            applied_dpi: crate::dpi::dpi_for_window(hwnd),
            draft,
        };
        dialog.initialize_controls()?;
        Ok(dialog)
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> DialogResult {
        if msg != WM_COMMAND {
            return DialogResult::Unhandled;
        }
        let id = (wparam.0 & 0xFFFF) as u16;
        if id == IDCANCEL.0 as u16 || id == ctrl_id::BTN_CLOSE {
            return DialogResult::Close(LRESULT(1));
        }
        let notify_code = ((wparam.0 >> 16) & 0xFFFF) as u32;
        self.handle_command(id, notify_code);
        DialogResult::Handled(LRESULT(1))
    }

    fn handle_dpi_changed(&mut self, wparam: WPARAM, lparam: LPARAM) {
        GlossaryDialog::handle_dpi_changed(self, wparam, lparam);
    }

    fn can_defer(msg: u32) -> bool {
        msg == WM_COMMAND
    }
}

pub(crate) const WM_GLOSSARY_APPLIED: u32 = WM_APP + 20;
