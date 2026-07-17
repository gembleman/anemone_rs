//! 후크 설정 대화상자
//!
//! 활성/비활성 후크를 관리하는 대화상자.
//! ListBox를 사용하여 후크를 이동하고 순서를 변경.

use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::{Foundation::*, System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*},
    core::*,
};

use super::helpers::{
    center_dialog_on_monitor, listbox_add_item, listbox_get_count, listbox_get_sel,
    listbox_get_text, listbox_reset, listbox_set_sel, register_resource_dialog, show_dialog_window,
    unregister_resource_dialog,
};
use crate::config::Config;
use crate::define_dialog_instance;

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

fn transfer_item<T>(
    source: &mut Vec<T>,
    target: &mut Vec<T>,
    index: i32,
) -> Option<(Option<i32>, i32)> {
    let index = usize::try_from(index).ok()?;
    if index >= source.len() {
        return None;
    }
    let item = source.remove(index);
    target.push(item);
    let source_selection = if source.is_empty() {
        None
    } else {
        Some(index.min(source.len() - 1) as i32)
    };
    Some((source_selection, (target.len() - 1) as i32))
}

fn move_item<T>(items: &mut [T], index: i32, offset: i32) -> Option<i32> {
    let index = usize::try_from(index).ok()?;
    let target = index.checked_add_signed(offset as isize)?;
    if index >= items.len() || target >= items.len() {
        return None;
    }
    items.swap(index, target);
    Some(target as i32)
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
            register_resource_dialog(hwnd);
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
                unregister_resource_dialog(hwnd);
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
            center_dialog_on_monitor(hwnd, parent);
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

            BTN_TO_INACTIVE => unsafe {
                let Ok(active_lb) = GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) else {
                    return;
                };
                let Ok(inactive_lb) = GetDlgItem(Some(self.hwnd), INACTIVE_LIST as i32) else {
                    return;
                };
                let sel = listbox_get_sel(active_lb);
                self.sync_from_listboxes();
                if let Some((source_sel, target_sel)) =
                    transfer_item(&mut self.active_hooks, &mut self.inactive_hooks, sel)
                {
                    self.refresh_listboxes();
                    if let Some(new_sel) = source_sel {
                        listbox_set_sel(active_lb, new_sel);
                    }
                    listbox_set_sel(inactive_lb, target_sel);
                }
            },

            BTN_TO_ACTIVE => unsafe {
                let Ok(active_lb) = GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) else {
                    return;
                };
                let Ok(inactive_lb) = GetDlgItem(Some(self.hwnd), INACTIVE_LIST as i32) else {
                    return;
                };
                let sel = listbox_get_sel(inactive_lb);
                self.sync_from_listboxes();
                if let Some((source_sel, target_sel)) =
                    transfer_item(&mut self.inactive_hooks, &mut self.active_hooks, sel)
                {
                    self.refresh_listboxes();
                    if let Some(new_sel) = source_sel {
                        listbox_set_sel(inactive_lb, new_sel);
                    }
                    listbox_set_sel(active_lb, target_sel);
                }
            },

            BTN_UP => unsafe {
                let Ok(active_lb) = GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) else {
                    return;
                };
                let sel = listbox_get_sel(active_lb);
                self.sync_from_listboxes();
                if let Some(new_sel) = move_item(&mut self.active_hooks, sel, -1) {
                    self.refresh_listboxes();
                    listbox_set_sel(active_lb, new_sel);
                }
            },

            BTN_DOWN => unsafe {
                let Ok(active_lb) = GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) else {
                    return;
                };
                let sel = listbox_get_sel(active_lb);
                self.sync_from_listboxes();
                if let Some(new_sel) = move_item(&mut self.active_hooks, sel, 1) {
                    self.refresh_listboxes();
                    listbox_set_sel(active_lb, new_sel);
                }
            },

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
                    listbox_add_item(active_lb, hook);
                }
            }

            if let Ok(inactive_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::INACTIVE_LIST as i32) {
                for hook in &self.inactive_hooks {
                    listbox_add_item(inactive_lb, hook);
                }
            }
        }
    }

    fn refresh_listboxes(&self) {
        unsafe {
            if let Ok(active_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::ACTIVE_LIST as i32) {
                listbox_reset(active_lb);
            }
            if let Ok(inactive_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::INACTIVE_LIST as i32) {
                listbox_reset(inactive_lb);
            }
        }
        self.populate_listboxes();
    }

    /// 내부 후크 목록 동기화 (ListBox -> 내부 Vec)
    fn sync_from_listboxes(&mut self) {
        // SAFETY: self.hwnd is a valid dialog window handle. GetDlgItem returns valid
        // listbox handles. All listbox helper calls use these valid handles.
        unsafe {
            if let Ok(active_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::ACTIVE_LIST as i32) {
                self.active_hooks.clear();
                let count = listbox_get_count(active_lb);
                for i in 0..count {
                    if let Some(text) = listbox_get_text(active_lb, i) {
                        self.active_hooks.push(text);
                    }
                }
            }

            if let Ok(inactive_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::INACTIVE_LIST as i32) {
                self.inactive_hooks.clear();
                let count = listbox_get_count(inactive_lb);
                for i in 0..count {
                    if let Some(text) = listbox_get_text(inactive_lb, i) {
                        self.inactive_hooks.push(text);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{move_item, transfer_item};

    #[test]
    fn transfers_items_between_active_and_inactive_lists() {
        let mut active = vec!["A", "B"];
        let mut inactive = vec!["C"];

        let selection = transfer_item(&mut active, &mut inactive, 0);

        assert_eq!(active, ["B"]);
        assert_eq!(inactive, ["C", "A"]);
        assert_eq!(selection, Some((Some(0), 1)));
    }

    #[test]
    fn moves_items_up_and_down_with_bounds_checks() {
        let mut items = vec!["A", "B", "C"];

        assert_eq!(move_item(&mut items, 1, -1), Some(0));
        assert_eq!(items, ["B", "A", "C"]);
        assert_eq!(move_item(&mut items, 0, -1), None);
        assert_eq!(move_item(&mut items, 2, 1), None);
    }
}
