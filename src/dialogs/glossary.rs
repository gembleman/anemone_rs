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

use crate::config::{Config, LlmGlossaryEntry};
use crate::define_dialog_instance;
use crate::util::to_wide;
use super::helpers::{Dialog, DialogControls};

mod ctrl_id {
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
    config: Rc<RefCell<Config>>,
    /// 임시 편집 버퍼 (적용 전까지 Config에 반영하지 않음)
    entries: Vec<LlmGlossaryEntry>,
}

impl DialogControls for GlossaryDialog {
    fn dialog_hwnd(&self) -> HWND { self.hwnd }
}

define_dialog_instance!(GLOSSARY_INSTANCE: GlossaryDialog);

impl Dialog for GlossaryDialog {
    type Params = Rc<RefCell<Config>>;

    const CLASS_NAME: PCWSTR = w!("AnemoneGlossaryClass");
    const TITLE: PCWSTR = w!("LLM 글로서리 편집");
    const WIDTH: i32 = 480;
    const HEIGHT: i32 = 380;
    const EXTRA_STYLE: WINDOW_STYLE = WINDOW_STYLE(0);

    fn instance_slot()
        -> &'static std::thread::LocalKey<
            std::cell::RefCell<Option<std::rc::Rc<std::cell::RefCell<Self>>>>,
        > {
        &GLOSSARY_INSTANCE
    }

    fn init(hwnd: HWND, _parent: HWND, config: Self::Params) -> Self {
        let entries = config.borrow().translation.llm.glossary.clone();
        GlossaryDialog { hwnd, config, entries }
    }

    fn create_controls(&mut self) -> Result<()> {
        // SAFETY: self.hwnd is a valid dialog window. All helper methods use this handle.
        unsafe {
            self.create_group_box(10, 10, 460, 220, "사전 항목 (source → target)")?;
            self.create_listbox(20, 30, 440, 190, ctrl_id::LIST)?;

            self.create_label(20, 240, 50, 18, "원문:")?;
            self.create_edit(70, 238, 160, 22, ctrl_id::SOURCE_EDIT, "")?;
            self.create_label(240, 240, 50, 18, "번역:")?;
            self.create_edit(290, 238, 170, 22, ctrl_id::TARGET_EDIT, "")?;

            self.create_button(20, 270, 100, 26, ctrl_id::BTN_ADD, "추가/수정")?;
            self.create_button(130, 270, 80, 26, ctrl_id::BTN_REMOVE, "삭제")?;

            self.create_button(280, 308, 90, 30, ctrl_id::BTN_APPLY, "적용")?;
            self.create_button(380, 308, 80, 30, ctrl_id::BTN_CLOSE, "닫기")?;

            self.populate_listbox();
            Ok(())
        }
    }

    fn handle_command(&mut self, cmd: u16, _notify_code: u32) {
        use ctrl_id::*;
        match cmd {
            BTN_CLOSE => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
            BTN_APPLY => {
                self.config.borrow_mut().translation.llm.glossary = self.entries.clone();
                if let Err(e) = self.config.borrow().save() {
                    tracing::error!("글로서리 저장 실패: {}", e);
                }
            }
            BTN_ADD => self.add_or_update_entry(),
            BTN_REMOVE => self.remove_selected(),
            LIST => {
                // 리스트 선택 시 Edit에 항목 로드 (편집 흐름 개선)
                let sel = self.listbox_get_sel();
                if sel >= 0 && (sel as usize) < self.entries.len() {
                    let e = &self.entries[sel as usize];
                    self.set_control_text(SOURCE_EDIT, &e.source);
                    self.set_control_text(TARGET_EDIT, &e.target);
                }
            }
            _ => {}
        }
    }
}

impl GlossaryDialog {
    fn add_or_update_entry(&mut self) {
        let src = self.get_control_text(ctrl_id::SOURCE_EDIT).trim().to_string();
        let tgt = self.get_control_text(ctrl_id::TARGET_EDIT).trim().to_string();
        if src.is_empty() {
            return;
        }
        // 동일 source가 있으면 target만 갱신, 없으면 추가
        if let Some(existing) = self.entries.iter_mut().find(|e| e.source == src) {
            existing.target = tgt;
        } else {
            self.entries.push(LlmGlossaryEntry { source: src, target: tgt });
        }
        self.refresh_listbox();
        self.set_control_text(ctrl_id::SOURCE_EDIT, "");
        self.set_control_text(ctrl_id::TARGET_EDIT, "");
    }

    fn remove_selected(&mut self) {
        let sel = self.listbox_get_sel();
        if sel < 0 || (sel as usize) >= self.entries.len() {
            return;
        }
        self.entries.remove(sel as usize);
        self.refresh_listbox();
    }

    fn populate_listbox(&self) {
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid listbox handle.
        unsafe {
            let Ok(lb) = GetDlgItem(Some(self.hwnd), ctrl_id::LIST as i32) else { return; };
            if lb.is_invalid() { return; }
            for e in &self.entries {
                let line = format!("{} → {}", e.source, e.target);
                let wide = to_wide(&line);
                let _ = SendMessageW(lb, LB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(wide.as_ptr() as isize)));
            }
        }
    }

    fn refresh_listbox(&self) {
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid listbox handle.
        unsafe {
            let Ok(lb) = GetDlgItem(Some(self.hwnd), ctrl_id::LIST as i32) else { return; };
            if lb.is_invalid() { return; }
            // 전체 삭제: LB_GETCOUNT 만큼 하나씩 지우는 대신 reset
            let count = SendMessageW(lb, LB_GETCOUNT, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32;
            for _ in 0..count {
                let _ = SendMessageW(lb, LB_DELETESTRING, Some(WPARAM(0)), Some(LPARAM(0)));
            }
            for e in &self.entries {
                let line = format!("{} → {}", e.source, e.target);
                let wide = to_wide(&line);
                let _ = SendMessageW(lb, LB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(wide.as_ptr() as isize)));
            }
        }
    }

    fn listbox_get_sel(&self) -> i32 {
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid listbox handle.
        unsafe {
            let Ok(lb) = GetDlgItem(Some(self.hwnd), ctrl_id::LIST as i32) else { return LB_ERR; };
            if lb.is_invalid() { return LB_ERR; }
            SendMessageW(lb, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32
        }
    }

    fn set_control_text(&self, ctrl_id: u16, text: &str) {
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid control handle.
        unsafe {
            if let Ok(ctrl) = GetDlgItem(Some(self.hwnd), ctrl_id as i32)
                && !ctrl.is_invalid()
            {
                let text_wide = to_wide(text);
                let _ = SetWindowTextW(ctrl, PCWSTR(text_wide.as_ptr()));
            }
        }
    }

    fn get_control_text(&self, ctrl_id: u16) -> String {
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid control handle.
        unsafe {
            let Ok(ctrl) = GetDlgItem(Some(self.hwnd), ctrl_id as i32) else { return String::new(); };
            if ctrl.is_invalid() { return String::new(); }
            let len = GetWindowTextLengthW(ctrl);
            if len == 0 { return String::new(); }
            let mut buffer: Vec<u16> = vec![0; (len + 1) as usize];
            GetWindowTextW(ctrl, &mut buffer);
            String::from_utf16_lossy(&buffer[..len as usize])
        }
    }
}

