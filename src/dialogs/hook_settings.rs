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
    center_dialog_on_monitor, listbox_add_item, listbox_get_sel, listbox_reset, listbox_set_sel,
    register_resource_dialog, show_dialog_window, unregister_resource_dialog,
};
use super::models::HookListDraft;
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

/// 후크 설정 대화상자
pub struct HookSettingsDialog {
    hwnd: HWND,
    config: Rc<RefCell<Config>>,
    applied_dpi: u32,
    /// 적용 전까지 Config에 반영하지 않는 활성/비활성 목록.
    draft: HookListDraft,
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

            let draft = HookListDraft::from_config(&config.borrow());
            let dialog = Rc::new(RefCell::new(HookSettingsDialog {
                hwnd,
                config,
                applied_dpi: crate::dpi::dpi_for_window(hwnd),
                draft,
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
                self.draft.clone().commit(&mut self.config.borrow_mut());
            }

            BTN_TO_INACTIVE => unsafe {
                let Ok(active_lb) = GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) else {
                    return;
                };
                let Ok(inactive_lb) = GetDlgItem(Some(self.hwnd), INACTIVE_LIST as i32) else {
                    return;
                };
                let sel = listbox_get_sel(active_lb);
                if let Some(change) = usize::try_from(sel)
                    .ok()
                    .and_then(|index| self.draft.move_to_inactive(index))
                {
                    self.refresh_listboxes();
                    if let Some(new_sel) = change.source_selection {
                        listbox_set_sel(active_lb, new_sel as i32);
                    }
                    listbox_set_sel(inactive_lb, change.target_selection as i32);
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
                if let Some(change) = usize::try_from(sel)
                    .ok()
                    .and_then(|index| self.draft.move_to_active(index))
                {
                    self.refresh_listboxes();
                    if let Some(new_sel) = change.source_selection {
                        listbox_set_sel(inactive_lb, new_sel as i32);
                    }
                    listbox_set_sel(active_lb, change.target_selection as i32);
                }
            },

            BTN_UP => unsafe {
                let Ok(active_lb) = GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) else {
                    return;
                };
                let sel = listbox_get_sel(active_lb);
                if let Some(new_sel) = usize::try_from(sel)
                    .ok()
                    .and_then(|index| self.draft.move_active_by(index, -1))
                {
                    self.refresh_listboxes();
                    listbox_set_sel(active_lb, new_sel as i32);
                }
            },

            BTN_DOWN => unsafe {
                let Ok(active_lb) = GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) else {
                    return;
                };
                let sel = listbox_get_sel(active_lb);
                if let Some(new_sel) = usize::try_from(sel)
                    .ok()
                    .and_then(|index| self.draft.move_active_by(index, 1))
                {
                    self.refresh_listboxes();
                    listbox_set_sel(active_lb, new_sel as i32);
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
                for hook in self.draft.active() {
                    listbox_add_item(active_lb, hook);
                }
            }

            if let Ok(inactive_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::INACTIVE_LIST as i32) {
                for hook in self.draft.inactive() {
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
}
