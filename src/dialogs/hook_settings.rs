//! 후크 설정 대화상자
//!
//! 활성/비활성 후크를 관리하는 대화상자.
//! ListBox를 사용하여 후크를 이동하고 순서를 변경.

use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::helpers::show_dialog_window;
use crate::config::Config;
use crate::define_dialog_instance;
use crate::util::to_wide;

// 컨트롤 ID
mod ctrl_id {
    pub const DIALOG: u16 = 101;
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
    applied_dpi: u32,
    /// 임시 활성 후크 목록 (적용 전까지 Config에 반영하지 않음)
    active_hooks: Vec<String>,
    /// 임시 비활성 후크 목록
    inactive_hooks: Vec<String>,
}

define_dialog_instance!(HOOK_SETTINGS_INSTANCE: HookSettingsDialog);

struct PendingHookSettings {
    config: Rc<RefCell<Config>>,
}

thread_local! {
    static HOOK_SETTINGS_PENDING: RefCell<Option<PendingHookSettings>> = const { RefCell::new(None) };
    static HOOK_SETTINGS_INIT_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// 부모 윈도우가 있는 모니터의 작업 영역 중앙에 후크 설정창을 배치한다.
unsafe fn center_on_monitor(hwnd: HWND, parent: HWND) {
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return;
        }
        let monitor = MonitorFromWindow(parent, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let work = if GetMonitorInfoW(monitor, &mut info).as_bool() {
            info.rcWork
        } else {
            RECT {
                left: 0,
                top: 0,
                right: GetSystemMetrics(SM_CXSCREEN),
                bottom: GetSystemMetrics(SM_CYSCREEN),
            }
        };
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let x = work.left + (work.right - work.left - width) / 2;
        let y = work.top + (work.bottom - work.top - height) / 2;
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// `resources/hook_settings.rc`에서 생성된 모델리스 다이얼로그의 메시지 콜백.
unsafe extern "system" fn hook_settings_dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    unsafe {
        if msg == WM_INITDIALOG {
            let pending = HOOK_SETTINGS_PENDING.with(|slot| slot.borrow_mut().take());
            let Some(PendingHookSettings { config }) = pending else {
                HOOK_SETTINGS_INIT_ERROR.with(|slot| {
                    *slot.borrow_mut() = Some("후크 설정창 초기화 인자가 없습니다".to_string());
                });
                return 0;
            };

            let (active_hooks, inactive_hooks) = {
                let cfg = config.borrow();
                (
                    cfg.hook.active_hooks.clone(),
                    cfg.hook.inactive_hooks.clone(),
                )
            };
            let dialog = Rc::new(RefCell::new(HookSettingsDialog {
                hwnd,
                config,
                applied_dpi: crate::dpi::dpi_for_window(hwnd),
                active_hooks,
                inactive_hooks,
            }));
            HOOK_SETTINGS_INSTANCE.with(|slot| {
                *slot.borrow_mut() = Some(dialog.clone());
            });

            if let Err(error) = dialog.borrow().initialize_controls() {
                HOOK_SETTINGS_INSTANCE.with(|slot| {
                    slot.borrow_mut().take();
                });
                HOOK_SETTINGS_INIT_ERROR.with(|slot| {
                    *slot.borrow_mut() = Some(error.to_string());
                });
                return 0;
            }
            return 1;
        }

        let instance = HOOK_SETTINGS_INSTANCE.with(|slot| {
            let Ok(guard) = slot.try_borrow() else {
                return None;
            };
            guard.clone()
        });
        let Some(dialog) = instance else {
            return 0;
        };

        match msg {
            WM_DPICHANGED => {
                if let Ok(mut dialog) = dialog.try_borrow_mut() {
                    dialog.handle_dpi_changed(wparam, lparam);
                }
                1
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as u16;
                let notify_code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                if id == IDCANCEL.0 as u16 {
                    let _ = DestroyWindow(hwnd);
                } else if let Ok(mut dialog) = dialog.try_borrow_mut() {
                    dialog.handle_command(id, notify_code);
                }
                1
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                1
            }
            WM_DESTROY => {
                HOOK_SETTINGS_INSTANCE.with(|slot| {
                    if let Ok(mut guard) = slot.try_borrow_mut() {
                        *guard = None;
                    }
                });
                1
            }
            _ => 0,
        }
    }
}

impl HookSettingsDialog {
    /// `resources/hook_settings.rc`의 모델리스 DIALOGEX 리소스를 연다.
    pub fn show(parent: HWND, config: Rc<RefCell<Config>>) -> Result<HWND> {
        // SAFETY: None은 현재 프로세스 모듈을 뜻한다.
        let instance = unsafe { GetModuleHandleW(None)? };
        HOOK_SETTINGS_INIT_ERROR.with(|slot| {
            slot.borrow_mut().take();
        });
        HOOK_SETTINGS_PENDING.with(|slot| {
            *slot.borrow_mut() = Some(PendingHookSettings { config });
        });

        // SAFETY: 리소스 ID는 빌드 시 실행 파일에 포함되고, 콜백은 DLGPROC ABI를
        // 따른다. 초기화 인자는 UI 스레드의 pending 슬롯에서 한 번만 꺼낸다.
        let result = unsafe {
            CreateDialogParamW(
                Some(instance.into()),
                PCWSTR(ctrl_id::DIALOG as usize as *const u16),
                Some(parent),
                Some(hook_settings_dialog_proc),
                LPARAM(0),
            )
        };

        let hwnd = match result {
            Ok(hwnd) => hwnd,
            Err(error) => {
                HOOK_SETTINGS_PENDING.with(|slot| {
                    slot.borrow_mut().take();
                });
                HOOK_SETTINGS_INIT_ERROR.with(|slot| {
                    slot.borrow_mut().take();
                });
                return Err(error);
            }
        };

        if let Some(message) = HOOK_SETTINGS_INIT_ERROR.with(|slot| slot.borrow_mut().take()) {
            // SAFETY: CreateDialogParamW가 반환한 유효한 모델리스 다이얼로그.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(Error::new(E_FAIL, message));
        }

        unsafe {
            center_on_monitor(hwnd, parent);
            show_dialog_window(hwnd);
        }
        Ok(hwnd)
    }

    fn initialize_controls(&self) -> Result<()> {
        for id in [
            ctrl_id::ACTIVE_LIST,
            ctrl_id::INACTIVE_LIST,
            ctrl_id::BTN_TO_INACTIVE,
            ctrl_id::BTN_TO_ACTIVE,
            ctrl_id::BTN_UP,
            ctrl_id::BTN_DOWN,
            ctrl_id::BTN_APPLY,
            ctrl_id::BTN_CLOSE,
        ] {
            // SAFETY: self.hwnd는 WM_INITDIALOG가 전달한 유효한 다이얼로그 핸들이다.
            unsafe { GetDlgItem(Some(self.hwnd), id as i32) }.map_err(|_| {
                Error::new(
                    E_FAIL,
                    format!("후크 설정 컨트롤 ID {id}를 찾을 수 없습니다"),
                )
            })?;
        }
        self.populate_listboxes();
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
