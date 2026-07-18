//! 파일 번역 진행률 대화상자
//!
//! 번역 진행 상황 표시 및 취소 기능.

use std::cell::RefCell;
use std::rc::Rc;

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
    register_resource_dialog, rescale_dialog_children_for_dpi, set_window_text, show_dialog_window,
    unregister_resource_dialog,
};
use crate::define_dialog_instance;
use crate::file_trans::{FileTransTask, ProgressEvent};

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
#[derive(Debug, Default, PartialEq, Eq)]
struct ProgressState {
    total_files: i32,
    current_file_index: i32,
    total_lines: i32,
    current_line: i32,
    list_size: i32,
    terminal: bool,
}

const PROGRESS_POLL_TIMER: usize = 1;
const WM_PROGRESS_FINISH: u32 = WM_APP + 30;

impl ProgressState {
    fn apply(&mut self, event: &ProgressEvent) {
        if self.terminal {
            return;
        }
        match event {
            ProgressEvent::TotalFiles(value) => self.total_files = *value,
            ProgressEvent::TotalLines(value) => self.total_lines = *value,
            ProgressEvent::FileIndex(value) => self.current_file_index = *value,
            ProgressEvent::FileLines(value) => self.list_size = *value,
            ProgressEvent::FileProgress(_) => {}
            ProgressEvent::TotalProgress(value) => self.current_line = *value,
            ProgressEvent::Complete | ProgressEvent::Cancelled | ProgressEvent::Error(_) => {
                self.terminal = true;
            }
            ProgressEvent::FileName(_) => {}
        }
    }
}

/// 파일 번역 진행률 대화상자
pub struct FileTransProgressDialog {
    hwnd: HWND,
    parent_hwnd: HWND,
    task: FileTransTask,
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
    task: FileTransTask,
}

thread_local! {
    static PROGRESS_PENDING: RefCell<Option<PendingProgress>> = const { RefCell::new(None) };
    static PROGRESS_INIT_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
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
        let Some(PendingProgress { parent, task }) = pending else {
            PROGRESS_INIT_ERROR.with(|slot| {
                *slot.borrow_mut() = Some("파일 번역 진행률 창 초기화 인자가 없습니다".into());
            });
            return 0;
        };

        let dialog = Rc::new(RefCell::new(FileTransProgressDialog::new(
            hwnd, parent, task,
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
        return 0;
    };

    if msg == WM_TIMER && wparam.0 == PROGRESS_POLL_TIMER {
        let mut can_flush = false;
        if let Ok(mut dialog) = dialog.try_borrow_mut() {
            dialog.drain_progress_events();
            can_flush = true;
        } else {
            unsafe {
                super::helpers::defer_dialog_message(hwnd, msg, wparam, LPARAM(0));
            }
        }
        if can_flush {
            super::helpers::flush_deferred_dialog_messages(hwnd);
        }
        return 1;
    }

    let mut can_flush = false;
    let result = match msg {
        WM_DPICHANGED => {
            if let Ok(mut dialog) = dialog.try_borrow_mut() {
                dialog.handle_dpi_changed(wparam, lparam);
                can_flush = true;
            } else {
                unsafe {
                    super::helpers::defer_dialog_dpi_change(hwnd, wparam, lparam);
                }
            }
            1
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u16;
            if (id == ctrl_id::BTN_CANCEL || id == IDCANCEL.0 as u16)
                && let Ok(mut dialog) = dialog.try_borrow_mut()
            {
                dialog.handle_cancel();
                can_flush = true;
            } else if id == ctrl_id::BTN_CANCEL || id == IDCANCEL.0 as u16 {
                unsafe {
                    super::helpers::defer_dialog_message(hwnd, msg, wparam, lparam);
                }
            }
            1
        }
        // 닫기 요청도 취소와 동일하게 처리해 워커가 임시 파일을 정리하게 한다.
        WM_CLOSE => {
            if let Ok(mut dialog) = dialog.try_borrow_mut() {
                dialog.handle_cancel();
                can_flush = true;
            } else {
                unsafe {
                    super::helpers::defer_dialog_message(hwnd, msg, wparam, lparam);
                }
            }
            1
        }
        WM_DESTROY => {
            if let Ok(dialog) = dialog.try_borrow() {
                dialog.clear_taskbar_progress();
                unsafe {
                    let _ = EnableWindow(dialog.parent_hwnd, true);
                    let _ = SetForegroundWindow(dialog.parent_hwnd);
                }
            }
            unsafe {
                let _ = KillTimer(Some(hwnd), PROGRESS_POLL_TIMER);
            }
            unregister_resource_dialog(hwnd);
            PROGRESS_INSTANCE.with(|slot| {
                if let Ok(mut guard) = slot.try_borrow_mut() {
                    *guard = None;
                }
            });
            1
        }
        WM_PROGRESS_FINISH => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            1
        }
        _ => 0,
    };
    if can_flush {
        super::helpers::flush_deferred_dialog_messages(hwnd);
    }
    result
}

impl FileTransProgressDialog {
    fn new(hwnd: HWND, parent: HWND, task: FileTransTask) -> Self {
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
            task,
            applied_dpi: crate::dpi::dpi_for_window(hwnd),
            name_text: HWND::default(),
            progress_bar: HWND::default(),
            progress_text: HWND::default(),
            index_text: HWND::default(),
            total_text: HWND::default(),
            cancel_btn: HWND::default(),
            state: ProgressState::default(),
            taskbar,
        }
    }

    /// `resources/file_trans_progress.rc`의 모델리스 DIALOGEX 리소스를 연다.
    pub(crate) fn show(parent: HWND, task: FileTransTask) -> Result<HWND> {
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
            *slot.borrow_mut() = Some(PendingProgress { parent, task });
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
            let _ = EnableWindow(parent, false);
            show_dialog_window(hwnd);
        }
        Ok(hwnd)
    }

    /// 이미 진행 중인 작업이 있으면 그 진행창을 앞으로 가져온다.
    pub(crate) fn activate_existing() -> bool {
        let existing = PROGRESS_INSTANCE
            .with(|slot| slot.borrow().as_ref().map(|dialog| dialog.borrow().hwnd));
        if let Some(hwnd) = existing
            && unsafe { IsWindow(Some(hwnd)).as_bool() }
        {
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
            true
        } else {
            false
        }
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
        let timer = unsafe { SetTimer(Some(self.hwnd), PROGRESS_POLL_TIMER, 50, None) };
        if timer == 0 {
            return Err(Error::new(
                E_FAIL,
                "파일 번역 진행률 timer를 만들 수 없습니다",
            ));
        }
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

    fn drain_progress_events(&mut self) {
        let events = self.task.drain_events();
        for event in events {
            self.handle_progress_event(event);
        }
    }

    /// 공유 큐에서 꺼낸 진행률 이벤트를 화면에 반영한다.
    fn handle_progress_event(&mut self, event: ProgressEvent) {
        self.state.apply(&event);
        // SAFETY: 모든 컨트롤 핸들은 RC 템플릿에서 읽은 유효한 핸들이다.
        unsafe {
            match event {
                ProgressEvent::TotalLines(_) => {
                    let text = format!("전체: 0/{}", self.state.total_lines);
                    let _ = set_window_text(self.total_text, &text);
                }
                ProgressEvent::TotalFiles(_) => {
                    let text = format!("파일: 0/{}", self.state.total_files);
                    let _ = set_window_text(self.index_text, &text);
                }
                ProgressEvent::FileIndex(_) => {
                    let text = format!(
                        "파일: {}/{}",
                        self.state.current_file_index, self.state.total_files
                    );
                    let _ = set_window_text(self.index_text, &text);
                }
                ProgressEvent::FileName(filename) => {
                    let text = format!(
                        "{} ({}/{})",
                        filename, self.state.current_file_index, self.state.total_files
                    );
                    let _ = set_window_text(self.name_text, &text);
                    // 프로그레스바 초기화
                    let _ = SendMessageW(
                        self.progress_bar,
                        PBM_SETPOS,
                        Some(WPARAM(0)),
                        Some(LPARAM(0)),
                    );
                }
                ProgressEvent::FileLines(_) => {
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
                ProgressEvent::FileProgress(current) => {
                    // 프로그레스바 업데이트
                    let _ = SendMessageW(
                        self.progress_bar,
                        PBM_SETPOS,
                        Some(WPARAM(current as usize)),
                        Some(LPARAM(0)),
                    );
                    let text = format!("{}/{}", current, self.state.list_size);
                    let _ = set_window_text(self.progress_text, &text);
                }
                ProgressEvent::TotalProgress(_) => {
                    let text = format!(
                        "전체: {}/{}",
                        self.state.current_line, self.state.total_lines
                    );
                    let _ = set_window_text(self.total_text, &text);

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
                ProgressEvent::Complete => {
                    let _ = set_window_text(self.name_text, "완료!");
                    let _ = set_window_text(self.progress_text, "번역 완료");
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
                    let _ = PostMessageW(Some(self.hwnd), WM_PROGRESS_FINISH, WPARAM(0), LPARAM(0));
                }
                ProgressEvent::Cancelled => {
                    let _ = set_window_text(self.name_text, "취소됨");
                    let _ = set_window_text(self.progress_text, "번역을 취소했습니다");
                    let _ = EnableWindow(self.cancel_btn, false);
                    self.clear_taskbar_progress();
                    let _ = PostMessageW(Some(self.hwnd), WM_PROGRESS_FINISH, WPARAM(0), LPARAM(0));
                }
                ProgressEvent::Error(error_msg) => {
                    let _ = set_window_text(self.name_text, "오류 발생");
                    let _ = EnableWindow(self.cancel_btn, false);

                    // 작업 표시줄 진행률을 ERROR 상태로 잠깐 표시한 뒤
                    // MessageBox 가 닫히고 다이얼로그가 파괴되며 WM_DESTROY 에서 정리한다.
                    if let Some(ref tb) = self.taskbar {
                        let _ = tb.SetProgressState(self.parent_hwnd, TBPF_ERROR);
                    }

                    let message = HSTRING::from(error_msg);
                    let _ = MessageBoxW(Some(self.hwnd), &message, w!("오류"), MB_ICONERROR);

                    self.clear_taskbar_progress();
                    let _ = PostMessageW(Some(self.hwnd), WM_PROGRESS_FINISH, WPARAM(0), LPARAM(0));
                }
            }
        }
    }

    /// 취소 처리
    fn handle_cancel(&mut self) {
        self.task.cancel();
        // SAFETY: self.cancel_btn is a valid control handle from the RC template.
        unsafe {
            let _ = EnableWindow(self.cancel_btn, false);
            let _ = set_window_text(self.progress_text, "취소 중...");
            // 작업 표시줄을 PAUSED 로 표시 — 실제 종료/정리는 워커 스레드가
            // 취소 토큰을 감지해 오류 이벤트를 보낼 때 마무리된다.
            if let Some(ref tb) = self.taskbar {
                let _ = tb.SetProgressState(self.parent_hwnd, TBPF_PAUSED);
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/dialogs/file_trans_progress.rs"]
mod tests;
