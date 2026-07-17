//! 파일 번역 진행률 대화상자
//!
//! 번역 진행 상황 표시 및 취소 기능.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::{
    Win32::{
        Foundation::*,
        System::Com::{CLSCTX_ALL, CoCreateInstance},
        System::LibraryLoader::GetModuleHandleW,
        UI::Controls::*,
        UI::Input::KeyboardAndMouse::EnableWindow,
        UI::Shell::{
            ITaskbarList3, TBPF_ERROR, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED, TaskbarList,
        },
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::helpers::{
    register_resource_dialog, rescale_dialog_children_for_dpi, show_dialog_window,
    unregister_resource_dialog,
};
use crate::constants::{
    WM_PROGRESS_COMPLETE, WM_PROGRESS_CURRENT, WM_PROGRESS_ERROR, WM_PROGRESS_INDEX,
    WM_PROGRESS_LIST_SIZE, WM_PROGRESS_NAME, WM_PROGRESS_TOTAL_COUNT, WM_PROGRESS_TOTAL_SIZE,
    WM_PROGRESS_UPDATE,
};
use crate::define_dialog_instance;
use crate::util::to_wide;

// 컨트롤 ID
mod ctrl_id {
    pub const DIALOG: u16 = 103;
    pub const NAME_TEXT: u16 = 5001; // 현재 파일명
    pub const PROGRESS_BAR: u16 = 5002; // 프로그레스바
    pub const PROGRESS_TEXT: u16 = 5003; // 진행 텍스트
    pub const INDEX_TEXT: u16 = 5004; // 파일 인덱스
    pub const TOTAL_TEXT: u16 = 5005; // 전체 진행
    pub const BTN_CANCEL: u16 = 5010; // 취소 버튼
}

/// 진행률 대화상자 상태
struct ProgressState {
    total_files: i32,
    current_file_index: i32,
    total_lines: i32,
    current_line: i32,
    list_size: i32,
}

/// 파일 번역 진행률 대화상자
pub struct FileTransProgressDialog {
    hwnd: HWND,
    parent_hwnd: HWND,
    cancel_token: Arc<AtomicBool>,
    applied_dpi: u32,
    name_text: HWND,
    progress_bar: HWND,
    progress_text: HWND,
    index_text: HWND,
    total_text: HWND,
    cancel_btn: HWND,
    state: ProgressState,
    /// 작업 표시줄 진행률 인터페이스 (Win7+). 실패해도 다이얼로그 자체는 동작해야 하므로 Option.
    taskbar: Option<ITaskbarList3>,
}

define_dialog_instance!(PROGRESS_INSTANCE: FileTransProgressDialog);

struct PendingProgress {
    parent: HWND,
    cancel_token: Arc<AtomicBool>,
}

thread_local! {
    static PROGRESS_PENDING: RefCell<Option<PendingProgress>> = const { RefCell::new(None) };
    static PROGRESS_INIT_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// 인스턴스가 메시지를 처리할 수 없을 때 워커가 넘긴 문자열 버퍼를 회수한다.
fn reclaim_progress_payload(msg: u32, lparam: LPARAM) {
    if (msg == WM_PROGRESS_NAME || msg == WM_PROGRESS_ERROR) && lparam.0 != 0 {
        // SAFETY: 해당 두 메시지의 lparam은 워커가 Box::into_raw로 넘긴 *mut Vec<u16>이다.
        unsafe {
            let _ = Box::from_raw(lparam.0 as *mut Vec<u16>);
        }
    }
}

/// `resources/file_trans_progress.rc`에서 생성된 모델리스 다이얼로그의 메시지 콜백.
unsafe extern "system" fn file_trans_progress_dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    if msg == WM_INITDIALOG {
        let pending = PROGRESS_PENDING.with(|slot| slot.borrow_mut().take());
        let Some(PendingProgress {
            parent,
            cancel_token,
        }) = pending
        else {
            PROGRESS_INIT_ERROR.with(|slot| {
                *slot.borrow_mut() = Some("파일 번역 진행률 창 초기화 인자가 없습니다".into());
            });
            return 0;
        };

        let dialog = Rc::new(RefCell::new(FileTransProgressDialog::new(
            hwnd,
            parent,
            cancel_token,
        )));
        PROGRESS_INSTANCE.with(|slot| {
            *slot.borrow_mut() = Some(dialog.clone());
        });

        let initialization = dialog.borrow_mut().initialize_controls();
        if let Err(error) = initialization {
            dialog.borrow().clear_taskbar_progress();
            PROGRESS_INSTANCE.with(|slot| {
                slot.borrow_mut().take();
            });
            PROGRESS_INIT_ERROR.with(|slot| {
                *slot.borrow_mut() = Some(error.to_string());
            });
            return 0;
        }
        register_resource_dialog(hwnd);
        return 1;
    }

    let instance = PROGRESS_INSTANCE.with(|slot| {
        let Ok(guard) = slot.try_borrow() else {
            return None;
        };
        guard.clone()
    });
    let Some(dialog) = instance else {
        reclaim_progress_payload(msg, lparam);
        return 0;
    };

    if (WM_PROGRESS_TOTAL_SIZE..=WM_PROGRESS_ERROR).contains(&msg) {
        if let Ok(mut dialog) = dialog.try_borrow_mut() {
            dialog.handle_progress_message(msg, wparam, lparam);
        } else {
            reclaim_progress_payload(msg, lparam);
        }
        return 1;
    }

    match msg {
        WM_DPICHANGED => {
            if let Ok(mut dialog) = dialog.try_borrow_mut() {
                dialog.handle_dpi_changed(wparam, lparam);
            }
            1
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u16;
            if (id == ctrl_id::BTN_CANCEL || id == IDCANCEL.0 as u16)
                && let Ok(mut dialog) = dialog.try_borrow_mut()
            {
                dialog.handle_cancel();
            }
            1
        }
        // 닫기 버튼은 무시한다. 취소 버튼/Esc 또는 워커 완료·에러로만 닫힌다.
        WM_CLOSE => 1,
        WM_DESTROY => {
            if let Ok(dialog) = dialog.try_borrow() {
                dialog.clear_taskbar_progress();
            }
            unregister_resource_dialog(hwnd);
            PROGRESS_INSTANCE.with(|slot| {
                if let Ok(mut guard) = slot.try_borrow_mut() {
                    *guard = None;
                }
            });
            1
        }
        _ => 0,
    }
}

impl FileTransProgressDialog {
    fn new(hwnd: HWND, parent: HWND, cancel_token: Arc<AtomicBool>) -> Self {
        // UI 스레드는 main()에서 STA로 초기화된다. 작업 표시줄 초기화 실패는
        // 진행률 창 자체의 실패로 취급하지 않는다.
        let taskbar = unsafe {
            match CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_ALL) {
                Ok(taskbar) => match taskbar.HrInit() {
                    Ok(()) => {
                        let _ = taskbar.SetProgressState(parent, TBPF_NORMAL);
                        Some(taskbar)
                    }
                    Err(error) => {
                        tracing::warn!("ITaskbarList3::HrInit failed: {error}");
                        None
                    }
                },
                Err(error) => {
                    tracing::warn!("CoCreateInstance(TaskbarList) failed: {error}");
                    None
                }
            }
        };

        Self {
            hwnd,
            parent_hwnd: parent,
            cancel_token,
            applied_dpi: crate::dpi::dpi_for_window(hwnd),
            name_text: HWND::default(),
            progress_bar: HWND::default(),
            progress_text: HWND::default(),
            index_text: HWND::default(),
            total_text: HWND::default(),
            cancel_btn: HWND::default(),
            state: ProgressState {
                total_files: 0,
                current_file_index: 0,
                total_lines: 0,
                current_line: 0,
                list_size: 0,
            },
            taskbar,
        }
    }

    /// `resources/file_trans_progress.rc`의 모델리스 DIALOGEX 리소스를 연다.
    pub fn show(parent: HWND, cancel_token: Arc<AtomicBool>) -> Result<HWND> {
        let existing = PROGRESS_INSTANCE
            .with(|slot| slot.borrow().as_ref().map(|dialog| dialog.borrow().hwnd));
        if let Some(hwnd) = existing
            && unsafe { IsWindow(Some(hwnd)).as_bool() }
        {
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
            return Err(Error::new(
                E_FAIL,
                "파일 번역 진행률 창이 이미 열려 있습니다",
            ));
        }

        let instance = unsafe { GetModuleHandleW(None)? };
        PROGRESS_INIT_ERROR.with(|slot| {
            slot.borrow_mut().take();
        });
        PROGRESS_PENDING.with(|slot| {
            *slot.borrow_mut() = Some(PendingProgress {
                parent,
                cancel_token,
            });
        });

        let result = unsafe {
            CreateDialogParamW(
                Some(instance.into()),
                PCWSTR(ctrl_id::DIALOG as usize as *const u16),
                Some(parent),
                Some(file_trans_progress_dialog_proc),
                LPARAM(0),
            )
        };

        let hwnd = match result {
            Ok(hwnd) => hwnd,
            Err(error) => {
                PROGRESS_PENDING.with(|slot| {
                    slot.borrow_mut().take();
                });
                PROGRESS_INIT_ERROR.with(|slot| {
                    slot.borrow_mut().take();
                });
                return Err(error);
            }
        };

        if let Some(message) = PROGRESS_INIT_ERROR.with(|slot| slot.borrow_mut().take()) {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(Error::new(E_FAIL, message));
        }

        unsafe {
            Self::center_on_parent(hwnd, parent);
            show_dialog_window(hwnd);
        }
        Ok(hwnd)
    }

    fn initialize_controls(&mut self) -> Result<()> {
        let get_control = |id| {
            unsafe { GetDlgItem(Some(self.hwnd), id) }.map_err(|_| {
                Error::new(
                    E_FAIL,
                    format!("파일 번역 진행률 컨트롤 ID {id}를 찾을 수 없습니다"),
                )
            })
        };

        self.name_text = get_control(ctrl_id::NAME_TEXT as i32)?;
        self.progress_bar = get_control(ctrl_id::PROGRESS_BAR as i32)?;
        self.progress_text = get_control(ctrl_id::PROGRESS_TEXT as i32)?;
        self.index_text = get_control(ctrl_id::INDEX_TEXT as i32)?;
        self.total_text = get_control(ctrl_id::TOTAL_TEXT as i32)?;
        self.cancel_btn = get_control(ctrl_id::BTN_CANCEL as i32)?;
        Ok(())
    }

    unsafe fn center_on_parent(hwnd: HWND, parent: HWND) {
        unsafe {
            let mut dialog_rect = RECT::default();
            let mut parent_rect = RECT::default();
            if GetWindowRect(hwnd, &mut dialog_rect).is_err()
                || GetWindowRect(parent, &mut parent_rect).is_err()
            {
                return;
            }
            let width = dialog_rect.right - dialog_rect.left;
            let height = dialog_rect.bottom - dialog_rect.top;
            let x = parent_rect.left + (parent_rect.right - parent_rect.left - width) / 2;
            let y = parent_rect.top + (parent_rect.bottom - parent_rect.top - height) / 2;
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

    fn handle_dpi_changed(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let new_dpi = (wparam.0 & 0xFFFF) as u32;
        rescale_dialog_children_for_dpi(self.hwnd, self.applied_dpi, new_dpi);
        self.applied_dpi = new_dpi;

        if lparam.0 != 0 {
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

    fn clear_taskbar_progress(&self) {
        if let Some(taskbar) = &self.taskbar {
            unsafe {
                let _ = taskbar.SetProgressState(self.parent_hwnd, TBPF_NOPROGRESS);
            }
        }
    }

    /// 텍스트 설정
    unsafe fn set_text(hwnd: HWND, text: &str) {
        // SAFETY: hwnd is a valid control handle. wide string is valid for the call duration.
        unsafe {
            let wide = to_wide(text);
            let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
        }
    }

    /// 진행률 메시지 처리
    fn handle_progress_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) {
        // SAFETY: All control handles were loaded from the RC template and are valid.
        // lparam for WM_PROGRESS_NAME/WM_PROGRESS_ERROR is a *mut Vec<u16> leaked by
        // file_trans_thread::post_wide_string; we reclaim ownership via Box::from_raw below.
        unsafe {
            match msg {
                WM_PROGRESS_TOTAL_SIZE => {
                    self.state.total_lines = lparam.0 as i32;
                    let text = format!("전체: 0/{}", self.state.total_lines);
                    Self::set_text(self.total_text, &text);
                }
                WM_PROGRESS_TOTAL_COUNT => {
                    self.state.total_files = lparam.0 as i32;
                    let text = format!("파일: 0/{}", self.state.total_files);
                    Self::set_text(self.index_text, &text);
                }
                WM_PROGRESS_INDEX => {
                    self.state.current_file_index = lparam.0 as i32;
                    let text = format!(
                        "파일: {}/{}",
                        self.state.current_file_index, self.state.total_files
                    );
                    Self::set_text(self.index_text, &text);
                }
                WM_PROGRESS_NAME => {
                    // lparam 은 워커가 Box::into_raw 로 넘긴 *mut Vec<u16> — Box::from_raw 로 회수.
                    if lparam.0 != 0 {
                        let boxed: Box<Vec<u16>> = Box::from_raw(lparam.0 as *mut Vec<u16>);
                        let end = boxed.iter().position(|&c| c == 0).unwrap_or(boxed.len());
                        let filename = String::from_utf16_lossy(&boxed[..end]);
                        let text = format!(
                            "{} ({}/{})",
                            filename, self.state.current_file_index, self.state.total_files
                        );
                        Self::set_text(self.name_text, &text);
                    }
                    // 프로그레스바 초기화
                    let _ = SendMessageW(
                        self.progress_bar,
                        PBM_SETPOS,
                        Some(WPARAM(0)),
                        Some(LPARAM(0)),
                    );
                }
                WM_PROGRESS_LIST_SIZE => {
                    self.state.list_size = lparam.0 as i32;
                    // 프로그레스바 범위 설정
                    let _ = SendMessageW(
                        self.progress_bar,
                        PBM_SETRANGE32,
                        Some(WPARAM(0)),
                        Some(LPARAM(self.state.list_size as isize)),
                    );
                    let _ = SendMessageW(
                        self.progress_bar,
                        PBM_SETSTEP,
                        Some(WPARAM(1)),
                        Some(LPARAM(0)),
                    );
                }
                WM_PROGRESS_UPDATE => {
                    let current = wparam.0 as i32;
                    // 프로그레스바 업데이트
                    let _ = SendMessageW(
                        self.progress_bar,
                        PBM_SETPOS,
                        Some(WPARAM(current as usize)),
                        Some(LPARAM(0)),
                    );
                    let text = format!("{}/{}", current, self.state.list_size);
                    Self::set_text(self.progress_text, &text);
                }
                WM_PROGRESS_CURRENT => {
                    self.state.current_line = lparam.0 as i32;
                    let text = format!(
                        "전체: {}/{}",
                        self.state.current_line, self.state.total_lines
                    );
                    Self::set_text(self.total_text, &text);

                    // 작업 표시줄 진행률 갱신 (전체 라인 기준 — 가장 안정적인 단조 증가 신호).
                    if let Some(ref tb) = self.taskbar
                        && self.state.total_lines > 0
                    {
                        let _ = tb.SetProgressValue(
                            self.parent_hwnd,
                            self.state.current_line.max(0) as u64,
                            self.state.total_lines as u64,
                        );
                    }
                }
                WM_PROGRESS_COMPLETE => {
                    Self::set_text(self.name_text, "완료!");
                    Self::set_text(self.progress_text, "번역 완료");
                    let _ = EnableWindow(self.cancel_btn, false);

                    // 작업 표시줄 진행률 해제
                    if let Some(ref tb) = self.taskbar {
                        let _ = tb.SetProgressState(self.parent_hwnd, TBPF_NOPROGRESS);
                    }

                    // 완료 메시지
                    let _ = MessageBoxW(
                        Some(self.hwnd),
                        w!("번역을 완료했습니다."),
                        w!("알림"),
                        MB_ICONINFORMATION,
                    );

                    // 창 닫기
                    let _ = DestroyWindow(self.hwnd);
                }
                WM_PROGRESS_ERROR => {
                    // lparam 은 워커가 Box::into_raw 로 넘긴 *mut Vec<u16> — Box::from_raw 로 회수.
                    let error_msg = if lparam.0 != 0 {
                        let boxed: Box<Vec<u16>> = Box::from_raw(lparam.0 as *mut Vec<u16>);
                        let end = boxed.iter().position(|&c| c == 0).unwrap_or(boxed.len());
                        String::from_utf16_lossy(&boxed[..end])
                    } else {
                        "알 수 없는 오류가 발생했습니다.".to_string()
                    };

                    Self::set_text(self.name_text, "오류 발생");
                    let _ = EnableWindow(self.cancel_btn, false);

                    // 작업 표시줄 진행률을 ERROR 상태로 잠깐 표시한 뒤
                    // MessageBox 가 닫히고 다이얼로그가 파괴되며 WM_DESTROY 에서 정리한다.
                    if let Some(ref tb) = self.taskbar {
                        let _ = tb.SetProgressState(self.parent_hwnd, TBPF_ERROR);
                    }

                    let msg_wide = to_wide(&error_msg);
                    let _ = MessageBoxW(
                        Some(self.hwnd),
                        PCWSTR(msg_wide.as_ptr()),
                        w!("오류"),
                        MB_ICONERROR,
                    );

                    self.clear_taskbar_progress();
                    let _ = DestroyWindow(self.hwnd);
                }
                _ => {}
            }
        }
    }

    /// 취소 처리
    fn handle_cancel(&mut self) {
        self.cancel_token.store(true, Ordering::SeqCst);
        // SAFETY: self.cancel_btn is a valid control handle from the RC template.
        unsafe {
            let _ = EnableWindow(self.cancel_btn, false);
            Self::set_text(self.progress_text, "취소 중...");
            // 작업 표시줄을 PAUSED 로 표시 — 실제 종료/정리는 워커 스레드가
            // 취소 토큰을 감지해 WM_PROGRESS_ERROR 를 보낼 때 마무리된다.
            if let Some(ref tb) = self.taskbar {
                let _ = tb.SetProgressState(self.parent_hwnd, TBPF_PAUSED);
            }
        }
    }
}
