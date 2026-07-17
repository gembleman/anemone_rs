//! 글로서리(고정 번역 사전) 편집 대화상자
//!
//! LLM 번역 시 시스템 프롬프트에 주입되는 캐릭터 이름/고유명사 사전을 편집한다.
//! ListBox에 "source → target" 형식으로 표시하고, 두 개의 Edit으로 추가/수정.

use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::{Foundation::*, System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*},
    core::*,
};

use super::helpers::{
    center_dialog_on_monitor, register_resource_dialog, show_dialog_window,
    unregister_resource_dialog,
};
use crate::config::{Config, LlmGlossaryEntry};
use crate::define_dialog_instance;
use crate::util::to_wide;

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
    config: Rc<RefCell<Config>>,
    applied_dpi: u32,
    /// 임시 편집 버퍼 (적용 전까지 Config에 반영하지 않음)
    entries: Vec<LlmGlossaryEntry>,
}

define_dialog_instance!(GLOSSARY_INSTANCE: GlossaryDialog);

struct PendingGlossary {
    config: Rc<RefCell<Config>>,
}

thread_local! {
    static GLOSSARY_PENDING: RefCell<Option<PendingGlossary>> = const { RefCell::new(None) };
    static GLOSSARY_INIT_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// `resources/glossary.rc`에서 생성된 모델리스 다이얼로그의 메시지 콜백.
unsafe extern "system" fn glossary_dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    unsafe {
        if msg == WM_INITDIALOG {
            let pending = GLOSSARY_PENDING.with(|slot| slot.borrow_mut().take());
            let Some(PendingGlossary { config }) = pending else {
                GLOSSARY_INIT_ERROR.with(|slot| {
                    *slot.borrow_mut() = Some("글로서리 초기화 인자가 없습니다".to_string());
                });
                return 0;
            };

            let entries = config.borrow().translation.llm.glossary.clone();
            let dialog = Rc::new(RefCell::new(GlossaryDialog {
                hwnd,
                config,
                applied_dpi: crate::dpi::dpi_for_window(hwnd),
                entries,
            }));
            GLOSSARY_INSTANCE.with(|slot| {
                *slot.borrow_mut() = Some(dialog.clone());
            });

            if let Err(error) = dialog.borrow().initialize_controls() {
                GLOSSARY_INSTANCE.with(|slot| {
                    slot.borrow_mut().take();
                });
                GLOSSARY_INIT_ERROR.with(|slot| {
                    *slot.borrow_mut() = Some(error.to_string());
                });
                return 0;
            }
            register_resource_dialog(hwnd);
            return 1;
        }

        let instance = GLOSSARY_INSTANCE.with(|slot| {
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
                GLOSSARY_INSTANCE.with(|slot| {
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

impl GlossaryDialog {
    /// `resources/glossary.rc`의 모델리스 DIALOGEX 리소스를 연다.
    pub fn show(parent: HWND, config: Rc<RefCell<Config>>) -> Result<HWND> {
        let existing = GLOSSARY_INSTANCE
            .with(|slot| slot.borrow().as_ref().map(|dialog| dialog.borrow().hwnd));
        if let Some(hwnd) = existing
            && unsafe { IsWindow(Some(hwnd)).as_bool() }
        {
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
            return Ok(hwnd);
        }

        // SAFETY: None은 현재 프로세스 모듈을 뜻한다.
        let instance = unsafe { GetModuleHandleW(None)? };
        GLOSSARY_INIT_ERROR.with(|slot| {
            slot.borrow_mut().take();
        });
        GLOSSARY_PENDING.with(|slot| {
            *slot.borrow_mut() = Some(PendingGlossary { config });
        });

        // SAFETY: 리소스 ID는 빌드 시 실행 파일에 포함되고, 콜백은 DLGPROC ABI를
        // 따른다. 초기화 인자는 UI 스레드의 pending 슬롯에서 한 번만 꺼낸다.
        let result = unsafe {
            CreateDialogParamW(
                Some(instance.into()),
                PCWSTR(ctrl_id::DIALOG as usize as *const u16),
                Some(parent),
                Some(glossary_dialog_proc),
                LPARAM(0),
            )
        };

        let hwnd = match result {
            Ok(hwnd) => hwnd,
            Err(error) => {
                GLOSSARY_PENDING.with(|slot| {
                    slot.borrow_mut().take();
                });
                GLOSSARY_INIT_ERROR.with(|slot| {
                    slot.borrow_mut().take();
                });
                return Err(error);
            }
        };

        if let Some(message) = GLOSSARY_INIT_ERROR.with(|slot| slot.borrow_mut().take()) {
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
    fn add_or_update_entry(&mut self) {
        let src = self
            .get_control_text(ctrl_id::SOURCE_EDIT)
            .trim()
            .to_string();
        let tgt = self
            .get_control_text(ctrl_id::TARGET_EDIT)
            .trim()
            .to_string();
        if src.is_empty() {
            return;
        }
        // 동일 source가 있으면 target만 갱신, 없으면 추가
        if let Some(existing) = self.entries.iter_mut().find(|e| e.source == src) {
            existing.target = tgt;
        } else {
            self.entries.push(LlmGlossaryEntry {
                source: src,
                target: tgt,
            });
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
            let Ok(lb) = GetDlgItem(Some(self.hwnd), ctrl_id::LIST as i32) else {
                return;
            };
            if lb.is_invalid() {
                return;
            }
            for e in &self.entries {
                let line = format!("{} → {}", e.source, e.target);
                let wide = to_wide(&line);
                let _ = SendMessageW(
                    lb,
                    LB_ADDSTRING,
                    Some(WPARAM(0)),
                    Some(LPARAM(wide.as_ptr() as isize)),
                );
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
            // 전체 삭제: LB_GETCOUNT 만큼 하나씩 지우는 대신 reset
            let count = SendMessageW(lb, LB_GETCOUNT, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32;
            for _ in 0..count {
                let _ = SendMessageW(lb, LB_DELETESTRING, Some(WPARAM(0)), Some(LPARAM(0)));
            }
            for e in &self.entries {
                let line = format!("{} → {}", e.source, e.target);
                let wide = to_wide(&line);
                let _ = SendMessageW(
                    lb,
                    LB_ADDSTRING,
                    Some(WPARAM(0)),
                    Some(LPARAM(wide.as_ptr() as isize)),
                );
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
            let Ok(ctrl) = GetDlgItem(Some(self.hwnd), ctrl_id as i32) else {
                return String::new();
            };
            if ctrl.is_invalid() {
                return String::new();
            }
            let len = GetWindowTextLengthW(ctrl);
            if len == 0 {
                return String::new();
            }
            let mut buffer: Vec<u16> = vec![0; (len + 1) as usize];
            GetWindowTextW(ctrl, &mut buffer);
            String::from_utf16_lossy(&buffer[..len as usize])
        }
    }
}
