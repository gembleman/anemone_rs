//! 후크 설정 대화상자
//!
//! 활성/비활성 후크를 관리하는 대화상자.
//! ListBox를 사용하여 후크를 이동하고 순서를 변경.

use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::{Foundation::*, UI::WindowsAndMessaging::*},
    core::*,
};

use super::helpers::{Dialog, DialogControls};
use crate::config::Config;
use crate::define_dialog_instance;
use crate::util::to_wide;

// 컨트롤 ID
mod ctrl_id {
    pub const ACTIVE_LIST: u16 = 6001;
    pub const INACTIVE_LIST: u16 = 6002;
    pub const BTN_TO_INACTIVE: u16 = 6010; // -> (활성 -> 비활성)
    pub const BTN_TO_ACTIVE: u16 = 6011; // <- (비활성 -> 활성)
    pub const BTN_UP: u16 = 6020;
    pub const BTN_DOWN: u16 = 6021;
    pub const BTN_APPLY: u16 = 6030;
    pub const BTN_CLOSE: u16 = 6031;
}

/// 후크 설정 대화상자
pub struct HookSettingsDialog {
    hwnd: HWND,
    config: Rc<RefCell<Config>>,
    /// 임시 활성 후크 목록 (적용 전까지 Config에 반영하지 않음)
    active_hooks: Vec<String>,
    /// 임시 비활성 후크 목록
    inactive_hooks: Vec<String>,
}

impl DialogControls for HookSettingsDialog {
    fn dialog_hwnd(&self) -> HWND {
        self.hwnd
    }
}

define_dialog_instance!(HOOK_SETTINGS_INSTANCE: HookSettingsDialog);

impl Dialog for HookSettingsDialog {
    type Params = Rc<RefCell<Config>>;

    const CLASS_NAME: PCWSTR = w!("AnemoneHookSettingsClass");
    const TITLE: PCWSTR = w!("후크 설정");
    const WIDTH: i32 = 450;
    const HEIGHT: i32 = 350;
    const EXTRA_STYLE: WINDOW_STYLE = WINDOW_STYLE(0);

    fn instance_slot() -> &'static std::thread::LocalKey<
        std::cell::RefCell<Option<std::rc::Rc<std::cell::RefCell<Self>>>>,
    > {
        &HOOK_SETTINGS_INSTANCE
    }

    fn init(hwnd: HWND, _parent: HWND, config: Self::Params) -> Self {
        let (active, inactive) = {
            let cfg = config.borrow();
            (
                cfg.hook.active_hooks.clone(),
                cfg.hook.inactive_hooks.clone(),
            )
        };
        HookSettingsDialog {
            hwnd,
            config,
            active_hooks: active,
            inactive_hooks: inactive,
        }
    }

    fn create_controls(&mut self) -> Result<()> {
        // SAFETY: self.hwnd is a valid dialog window handle. All helper methods
        // (create_group_box, create_listbox, create_button) use valid parent handle
        // and control IDs.
        unsafe {
            // ====== 활성 후크 그룹 ======
            self.create_group_box(10, 10, 180, 240, "활성 후크")?;
            self.create_listbox(20, 30, 160, 180, ctrl_id::ACTIVE_LIST)?;

            // 위로/아래로 버튼
            self.create_button(20, 215, 75, 25, ctrl_id::BTN_UP, "위로")?;
            self.create_button(105, 215, 75, 25, ctrl_id::BTN_DOWN, "아래로")?;

            // ====== 이동 버튼 (중앙) ======
            self.create_button(200, 80, 50, 30, ctrl_id::BTN_TO_INACTIVE, "->")?;
            self.create_button(200, 120, 50, 30, ctrl_id::BTN_TO_ACTIVE, "<-")?;

            // ====== 비활성 후크 그룹 ======
            // 활성 listbox 와 동일한 가시 영역 (180px) 으로 통일. 비활성 쪽은
            // 위/아래 버튼이 없어 그룹박스 하단에 60px 잉여가 생기지만, 두
            // 그룹의 listbox 시각 영역 일치가 우선.
            self.create_group_box(260, 10, 180, 240, "비활성 후크")?;
            self.create_listbox(270, 30, 160, 180, ctrl_id::INACTIVE_LIST)?;

            // A-2: 다이얼로그 동작 안내. 활성 후크만 순서가 의미 있어 위/아래
            // 버튼이 한쪽에만 있는 이유와 → / ← 의 방향을 한 줄로 설명.
            self.create_label(
                10,
                285,
                430,
                18,
                "→/← 로 후크를 이동, 위/아래로 활성 후크 순서 조정. '적용' 으로 저장.",
            )?;

            // ====== 적용/닫기 버튼 ======
            // X-1 통일 기준: 닫기 100x30 @ (WIDTH-110, HEIGHT-35) = (340, 315).
            // 적용은 닫기 좌측 페어로 (230, 315, 100x30).
            self.create_button(230, 315, 100, 30, ctrl_id::BTN_APPLY, "적용")?;
            self.create_button(340, 315, 100, 30, ctrl_id::BTN_CLOSE, "닫기")?;

            // ListBox 초기화
            self.populate_listboxes();

            Ok(())
        }
    }

    /// 커스텀 메시지 핸들러 (없음)
    fn handle_message(&mut self, _msg: u32, _wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
        None
    }

    /// 명령 처리
    fn handle_command(&mut self, cmd: u16, _notify_code: u32) {
        use ctrl_id::*;

        match cmd {
            // SAFETY: self.hwnd is a valid dialog window handle.
            BTN_CLOSE => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },

            BTN_APPLY => {
                self.sync_from_listboxes();
                {
                    let mut cfg = self.config.borrow_mut();
                    cfg.hook.active_hooks = self.active_hooks.clone();
                    cfg.hook.inactive_hooks = self.inactive_hooks.clone();
                }
            }

            BTN_TO_INACTIVE => {
                // SAFETY: self.hwnd is a valid dialog handle. GetDlgItem returns valid
                // listbox handles for known control IDs. All listbox operations use these
                // valid handles with indices verified against LB_ERR before use.
                unsafe {
                    let active_lb = match GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) {
                        Ok(h) => h,
                        Err(_) => return,
                    };
                    let inactive_lb = match GetDlgItem(Some(self.hwnd), INACTIVE_LIST as i32) {
                        Ok(h) => h,
                        Err(_) => return,
                    };
                    let sel = self.listbox_get_sel(active_lb);
                    if sel == LB_ERR {
                        return;
                    }
                    if let Some(text) = self.listbox_get_text(active_lb, sel) {
                        self.listbox_delete_item(active_lb, sel);
                        self.listbox_add_item(inactive_lb, &text);
                        let count = self.listbox_get_count(active_lb);
                        if count > 0 {
                            let new_sel = if sel >= count { count - 1 } else { sel };
                            self.listbox_set_sel(active_lb, new_sel);
                        }
                    }
                }
            }

            BTN_TO_ACTIVE => {
                // SAFETY: self.hwnd is a valid dialog handle. GetDlgItem returns valid
                // listbox handles. All listbox operations use valid handles with indices
                // verified against LB_ERR before use.
                unsafe {
                    let active_lb = match GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) {
                        Ok(h) => h,
                        Err(_) => return,
                    };
                    let inactive_lb = match GetDlgItem(Some(self.hwnd), INACTIVE_LIST as i32) {
                        Ok(h) => h,
                        Err(_) => return,
                    };
                    let sel = self.listbox_get_sel(inactive_lb);
                    if sel == LB_ERR {
                        return;
                    }
                    if let Some(text) = self.listbox_get_text(inactive_lb, sel) {
                        self.listbox_delete_item(inactive_lb, sel);
                        self.listbox_add_item(active_lb, &text);
                        let count = self.listbox_get_count(inactive_lb);
                        if count > 0 {
                            let new_sel = if sel >= count { count - 1 } else { sel };
                            self.listbox_set_sel(inactive_lb, new_sel);
                        }
                    }
                }
            }

            BTN_UP => {
                // SAFETY: self.hwnd is a valid dialog handle. GetDlgItem returns a valid
                // listbox handle. Selection index is verified > 0 before moving up.
                unsafe {
                    let active_lb = match GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) {
                        Ok(h) => h,
                        Err(_) => return,
                    };
                    let sel = self.listbox_get_sel(active_lb);
                    if sel == LB_ERR || sel == 0 {
                        return;
                    }
                    if let Some(text) = self.listbox_get_text(active_lb, sel) {
                        self.listbox_delete_item(active_lb, sel);
                        self.listbox_insert_item(active_lb, sel - 1, &text);
                        self.listbox_set_sel(active_lb, sel - 1);
                    }
                }
            }

            BTN_DOWN => {
                // SAFETY: self.hwnd is a valid dialog handle. GetDlgItem returns a valid
                // listbox handle. Selection index is verified < count-1 before moving down.
                unsafe {
                    let active_lb = match GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) {
                        Ok(h) => h,
                        Err(_) => return,
                    };
                    let sel = self.listbox_get_sel(active_lb);
                    let count = self.listbox_get_count(active_lb);
                    if sel == LB_ERR || sel >= count - 1 {
                        return;
                    }
                    if let Some(text) = self.listbox_get_text(active_lb, sel) {
                        self.listbox_delete_item(active_lb, sel);
                        self.listbox_insert_item(active_lb, sel + 1, &text);
                        self.listbox_set_sel(active_lb, sel + 1);
                    }
                }
            }

            _ => {}
        }
    }
}

impl HookSettingsDialog {
    /// ListBox에 데이터 채우기
    fn populate_listboxes(&self) {
        // SAFETY: self.hwnd is a valid dialog window handle. GetDlgItem returns valid
        // listbox handles for the known control IDs created in create_controls.
        unsafe {
            if let Ok(active_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::ACTIVE_LIST as i32) {
                for hook in &self.active_hooks {
                    self.listbox_add_item(active_lb, hook);
                }
            }

            if let Ok(inactive_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::INACTIVE_LIST as i32) {
                for hook in &self.inactive_hooks {
                    self.listbox_add_item(inactive_lb, hook);
                }
            }
        }
    }

    // ====== ListBox 헬퍼 함수들 ======

    unsafe fn listbox_add_item(&self, hwnd: HWND, text: &str) {
        // SAFETY: hwnd is a valid listbox handle. The wide string pointer is valid for
        // the duration of SendMessageW. LB_ADDSTRING copies the string internally.
        unsafe {
            let text_wide = to_wide(text);
            let _ = SendMessageW(
                hwnd,
                LB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(text_wide.as_ptr() as isize)),
            );
        }
    }

    unsafe fn listbox_insert_item(&self, hwnd: HWND, index: i32, text: &str) {
        // SAFETY: hwnd is a valid listbox handle. index is a valid position within the
        // listbox. The wide string pointer is valid for the duration of SendMessageW.
        unsafe {
            let text_wide = to_wide(text);
            let _ = SendMessageW(
                hwnd,
                LB_INSERTSTRING,
                Some(WPARAM(index as usize)),
                Some(LPARAM(text_wide.as_ptr() as isize)),
            );
        }
    }

    unsafe fn listbox_delete_item(&self, hwnd: HWND, index: i32) {
        // SAFETY: hwnd is a valid listbox handle. index is a valid item position
        // verified by the caller before deletion.
        unsafe {
            let _ = SendMessageW(
                hwnd,
                LB_DELETESTRING,
                Some(WPARAM(index as usize)),
                Some(LPARAM(0)),
            );
        }
    }

    unsafe fn listbox_get_sel(&self, hwnd: HWND) -> i32 {
        // SAFETY: hwnd is a valid listbox handle. LB_GETCURSEL requires no pointers.
        unsafe { SendMessageW(hwnd, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32 }
    }

    unsafe fn listbox_set_sel(&self, hwnd: HWND, index: i32) {
        // SAFETY: hwnd is a valid listbox handle. index is within the valid range.
        unsafe {
            let _ = SendMessageW(
                hwnd,
                LB_SETCURSEL,
                Some(WPARAM(index as usize)),
                Some(LPARAM(0)),
            );
        }
    }

    unsafe fn listbox_get_count(&self, hwnd: HWND) -> i32 {
        // SAFETY: hwnd is a valid listbox handle. LB_GETCOUNT requires no pointers.
        unsafe { SendMessageW(hwnd, LB_GETCOUNT, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32 }
    }

    unsafe fn listbox_get_text(&self, hwnd: HWND, index: i32) -> Option<String> {
        // SAFETY: hwnd is a valid listbox handle. index is checked via LB_GETTEXTLEN
        // before reading. The buffer is allocated with len+1 capacity based on the
        // reported text length. LB_GETTEXT writes into the buffer up to the reported length.
        unsafe {
            let len = SendMessageW(
                hwnd,
                LB_GETTEXTLEN,
                Some(WPARAM(index as usize)),
                Some(LPARAM(0)),
            )
            .0 as i32;
            if len == LB_ERR || len <= 0 {
                return None;
            }

            let mut buffer: Vec<u16> = vec![0; (len + 1) as usize];
            let result = SendMessageW(
                hwnd,
                LB_GETTEXT,
                Some(WPARAM(index as usize)),
                Some(LPARAM(buffer.as_mut_ptr() as isize)),
            );

            if result.0 as i32 == LB_ERR {
                return None;
            }

            String::from_utf16(&buffer[..len as usize]).ok()
        }
    }

    /// 내부 후크 목록 동기화 (ListBox -> 내부 Vec)
    fn sync_from_listboxes(&mut self) {
        // SAFETY: self.hwnd is a valid dialog window handle. GetDlgItem returns valid
        // listbox handles. All listbox helper calls use these valid handles.
        unsafe {
            if let Ok(active_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::ACTIVE_LIST as i32) {
                self.active_hooks.clear();
                let count = self.listbox_get_count(active_lb);
                for i in 0..count {
                    if let Some(text) = self.listbox_get_text(active_lb, i) {
                        self.active_hooks.push(text);
                    }
                }
            }

            if let Ok(inactive_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::INACTIVE_LIST as i32) {
                self.inactive_hooks.clear();
                let count = self.listbox_get_count(inactive_lb);
                for i in 0..count {
                    if let Some(text) = self.listbox_get_text(inactive_lb, i) {
                        self.inactive_hooks.push(text);
                    }
                }
            }
        }
    }
}
