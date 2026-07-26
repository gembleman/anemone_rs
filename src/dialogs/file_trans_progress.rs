//! 파일 번역 진행률 대화상자
//!
//! 번역 진행 상황 표시 및 취소 기능.

use windows::{
    Win32::{
        Foundation::*,
        System::Com::{CLSCTX_ALL, CoCreateInstance},
        UI::Controls::*,
        UI::Input::KeyboardAndMouse::EnableWindow,
        UI::Shell::{
            ITaskbarList3, TBPF_ERROR, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED, TaskbarList,
        },
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::helpers::set_window_text;
use super::host::{DialogHost, DialogPlacement, DialogResult, HostedDialog, ReopenPolicy};
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
            ProgressEvent::Finished(_) => {
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

pub(crate) struct ProgressInit {
    parent: HWND,
    task: FileTransTask,
}

impl HostedDialog for FileTransProgressDialog {
    type Init = ProgressInit;
    const RESOURCE_ID: u16 = ctrl_id::DIALOG;
    /// 번역이 도는 동안 부모를 가리므로 부모 사각형 기준으로 중앙에 놓는다.
    const PLACEMENT: DialogPlacement = DialogPlacement::ParentCenter;
    /// 진행 중에는 주 창 입력을 막는다. host가 WM_DESTROY에서 되살린다.
    const DISABLE_PARENT: bool = true;
    /// 같은 작업을 두 번 시작하지 않도록 호출자에게 오류를 돌려준다.
    const REOPEN: ReopenPolicy = ReopenPolicy::Reject("파일 번역 진행률 창이 이미 열려 있습니다");

    fn create(hwnd: HWND, init: Self::Init) -> Result<Self> {
        let mut dialog = Self::new(hwnd, init.parent, init.task);
        if let Err(error) = dialog.initialize_controls() {
            dialog.clear_taskbar_progress();
            return Err(error);
        }
        Ok(dialog)
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> DialogResult {
        match msg {
            WM_TIMER if wparam.0 == PROGRESS_POLL_TIMER => {
                self.drain_progress_events();
                DialogResult::Handled(LRESULT(1))
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as u16;
                if id == ctrl_id::BTN_CANCEL || id == IDCANCEL.0 as u16 {
                    self.handle_cancel();
                }
                DialogResult::Handled(LRESULT(1))
            }
            // 닫기 요청도 취소와 동일하게 처리해 워커가 임시 파일을 정리하게 한다.
            // 창은 워커가 종료 이벤트를 보낼 때 WM_PROGRESS_FINISH로 닫는다.
            WM_CLOSE => {
                self.handle_cancel();
                DialogResult::Handled(LRESULT(1))
            }
            WM_PROGRESS_FINISH => DialogResult::Close(LRESULT(1)),
            _ => DialogResult::Unhandled,
        }
    }

    fn applied_dpi(&mut self) -> Option<&mut u32> {
        Some(&mut self.applied_dpi)
    }

    fn destroy(&mut self) {
        self.clear_taskbar_progress();
        // SAFETY: self.hwnd는 아직 파괴 중인 유효한 창이다.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), PROGRESS_POLL_TIMER);
        }
    }

    fn can_defer(msg: u32) -> bool {
        // WM_CLOSE는 host가 공통으로 재예약해 현재 handler가 끝난 뒤 취소로 처리한다.
        msg == WM_COMMAND
    }
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
        DialogHost::<Self>::show(parent, ProgressInit { parent, task })
    }

    /// 파일 번역이 진행 중인지만 판정한다. 창을 앞으로 가져오지 않는다.
    ///
    /// [`Self::activate_existing`]은 판정과 동시에 `SetForegroundWindow`를
    /// 호출하므로, 단순히 "진행 중인가"를 묻는 곳에서 쓰면 사용자가 보던 창이
    /// 뜻밖에 뒤로 밀린다. 업데이트 적용 거부 판정처럼 부작용이 없어야 하는
    /// 곳에서는 이 함수를 쓴다.
    pub(crate) fn is_running() -> bool {
        DialogHost::<Self>::current_hwnd().is_some()
    }

    /// 이미 진행 중인 작업이 있으면 그 진행창을 앞으로 가져온다.
    pub(crate) fn activate_existing() -> bool {
        let Some(hwnd) = DialogHost::<Self>::current_hwnd() else {
            return false;
        };
        // SAFETY: current_hwnd는 IsWindow로 검증된 핸들만 돌려준다.
        unsafe {
            let _ = SetForegroundWindow(hwnd);
        }
        true
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
                ProgressEvent::Finished(Ok(_)) => {
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
                ProgressEvent::Finished(Err(
                    crate::file_trans::FileTranslationError::Cancelled,
                )) => {
                    let _ = set_window_text(self.name_text, "취소됨");
                    let _ = set_window_text(self.progress_text, "번역을 취소했습니다");
                    let _ = EnableWindow(self.cancel_btn, false);
                    self.clear_taskbar_progress();
                    let _ = PostMessageW(Some(self.hwnd), WM_PROGRESS_FINISH, WPARAM(0), LPARAM(0));
                }
                ProgressEvent::Finished(Err(error_msg)) => {
                    let _ = set_window_text(self.name_text, "오류 발생");
                    let _ = EnableWindow(self.cancel_btn, false);

                    // 작업 표시줄 진행률을 ERROR 상태로 잠깐 표시한 뒤
                    // MessageBox 가 닫히고 다이얼로그가 파괴되며 WM_DESTROY 에서 정리한다.
                    if let Some(ref tb) = self.taskbar {
                        let _ = tb.SetProgressState(self.parent_hwnd, TBPF_ERROR);
                    }

                    let message = HSTRING::from(error_msg.to_string());
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
