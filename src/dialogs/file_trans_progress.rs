//! 파일 번역 진행률 대화상자
//!
//! 번역 진행 상황 표시 및 취소 기능.

use windows_core::{Error, HRESULT};
use windows_sys::Win32::{
    Foundation::*, UI::Controls::*, UI::Input::KeyboardAndMouse::EnableWindow,
    UI::WindowsAndMessaging::*,
};

use super::helpers::set_window_text;
use super::host::{DialogHost, DialogPlacement, DialogResult, HostedDialog, ReopenPolicy};
use crate::file_trans::{FileTranslationProgress, FileTranslationTask};
type Result<T> = windows_core::Result<T>;

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
    fn apply(&mut self, event: &FileTranslationProgress) {
        if self.terminal {
            return;
        }
        match event {
            FileTranslationProgress::TotalFiles(value) => self.total_files = *value,
            FileTranslationProgress::TotalLines(value) => self.total_lines = *value,
            FileTranslationProgress::FileIndex(value) => self.current_file_index = *value,
            FileTranslationProgress::FileLines(value) => self.list_size = *value,
            FileTranslationProgress::FileProgress(_) => {}
            FileTranslationProgress::TotalProgress(value) => self.current_line = *value,
            FileTranslationProgress::Finished(_) => {
                self.terminal = true;
            }
            FileTranslationProgress::FileName(_) => {}
        }
    }
}

/// 파일 번역 진행률 대화상자
pub struct FileTransProgressDialog {
    hwnd: HWND,
    task: FileTranslationTask,
    applied_dpi: u32,
    name_text: HWND,
    progress_bar: HWND,
    progress_text: HWND,
    index_text: HWND,
    total_text: HWND,
    cancel_btn: HWND,
    state: ProgressState,
}

pub(crate) struct ProgressInit {
    task: FileTranslationTask,
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
        let mut dialog = Self::new(hwnd, init.task);
        if let Err(error) = dialog.initialize_controls() {
            dialog.clear_taskbar_progress();
            return Err(error);
        }
        Ok(dialog)
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> DialogResult {
        match msg {
            WM_TIMER if wparam == PROGRESS_POLL_TIMER => {
                self.drain_progress_events();
                DialogResult::Handled(1)
            }
            WM_COMMAND => {
                let id = (wparam & 0xFFFF) as u16;
                if id == ctrl_id::BTN_CANCEL || id == IDCANCEL as u16 {
                    self.handle_cancel();
                }
                DialogResult::Handled(1)
            }
            // 닫기 요청도 취소와 동일하게 처리해 워커가 임시 파일을 정리하게 한다.
            // 창은 워커가 종료 이벤트를 보낼 때 WM_PROGRESS_FINISH로 닫는다.
            WM_CLOSE => {
                self.handle_cancel();
                DialogResult::Handled(1)
            }
            WM_PROGRESS_FINISH => DialogResult::Close(1),
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
            let _ = KillTimer(self.hwnd, PROGRESS_POLL_TIMER);
        }
    }

    fn can_defer(msg: u32) -> bool {
        // WM_CLOSE는 host가 공통으로 재예약해 현재 handler가 끝난 뒤 취소로 처리한다.
        msg == WM_COMMAND
    }
}

impl FileTransProgressDialog {
    fn new(hwnd: HWND, task: FileTranslationTask) -> Self {
        // UI 스레드는 main()에서 STA로 초기화된다. 작업 표시줄 초기화 실패는
        // 진행률 창 자체의 실패로 취급하지 않는다.
        Self {
            hwnd,
            task,
            applied_dpi: crate::dpi::dpi_for_window(hwnd),
            name_text: std::ptr::null_mut(),
            progress_bar: std::ptr::null_mut(),
            progress_text: std::ptr::null_mut(),
            index_text: std::ptr::null_mut(),
            total_text: std::ptr::null_mut(),
            cancel_btn: std::ptr::null_mut(),
            state: ProgressState::default(),
        }
    }

    /// `resources/file_trans_progress.rc`의 모델리스 DIALOGEX 리소스를 연다.
    pub(crate) fn show(parent: HWND, task: FileTranslationTask) -> Result<HWND> {
        DialogHost::<Self>::show(parent, ProgressInit { task })
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
            let handle = unsafe { GetDlgItem(self.hwnd, id) };
            if handle.is_null() {
                Err(Error::new(
                    HRESULT(0x80004005u32 as i32),
                    format!("파일 번역 진행률 컨트롤 ID {id}를 찾을 수 없습니다"),
                ))
            } else {
                Ok(handle)
            }
        };

        self.name_text = get_control(ctrl_id::NAME_TEXT as i32)?;
        self.progress_bar = get_control(ctrl_id::PROGRESS_BAR as i32)?;
        self.progress_text = get_control(ctrl_id::PROGRESS_TEXT as i32)?;
        self.index_text = get_control(ctrl_id::INDEX_TEXT as i32)?;
        self.total_text = get_control(ctrl_id::TOTAL_TEXT as i32)?;
        self.cancel_btn = get_control(ctrl_id::BTN_CANCEL as i32)?;
        let timer = unsafe { SetTimer(self.hwnd, PROGRESS_POLL_TIMER, 50, None) };
        if timer == 0 {
            return Err(Error::new(
                HRESULT(0x80004005u32 as i32),
                "파일 번역 진행률 timer를 만들 수 없습니다",
            ));
        }
        Ok(())
    }

    fn clear_taskbar_progress(&self) {
        // windows-sys exposes no projected ITaskbarList3; the dialog control
        // remains the authoritative progress indicator.
    }

    fn drain_progress_events(&mut self) {
        const EVENT_BUDGET: usize = 128;
        let events = self.task.drain_events_budgeted(EVENT_BUDGET);
        for event in events {
            self.handle_progress_event(event);
        }
    }

    /// 공유 큐에서 꺼낸 진행률 이벤트를 화면에 반영한다.
    fn handle_progress_event(&mut self, event: FileTranslationProgress) {
        self.state.apply(&event);
        // SAFETY: 모든 컨트롤 핸들은 RC 템플릿에서 읽은 유효한 핸들이다.
        unsafe {
            match event {
                FileTranslationProgress::TotalLines(_) => {
                    let text = format!("전체: 0/{}", self.state.total_lines);
                    let _ = set_window_text(self.total_text, &text);
                }
                FileTranslationProgress::TotalFiles(_) => {
                    let text = format!("파일: 0/{}", self.state.total_files);
                    let _ = set_window_text(self.index_text, &text);
                }
                FileTranslationProgress::FileIndex(_) => {
                    let text = format!(
                        "파일: {}/{}",
                        self.state.current_file_index, self.state.total_files
                    );
                    let _ = set_window_text(self.index_text, &text);
                }
                FileTranslationProgress::FileName(filename) => {
                    let text = format!(
                        "{} ({}/{})",
                        filename, self.state.current_file_index, self.state.total_files
                    );
                    let _ = set_window_text(self.name_text, &text);
                    // 프로그레스바 초기화
                    let _ = SendMessageW(self.progress_bar, PBM_SETPOS, 0, 0);
                }
                FileTranslationProgress::FileLines(_) => {
                    // 프로그레스바 범위 설정
                    let _ = SendMessageW(
                        self.progress_bar,
                        PBM_SETRANGE32,
                        0,
                        self.state.list_size as isize,
                    );
                    let _ = SendMessageW(self.progress_bar, PBM_SETSTEP, 1, 0);
                }
                FileTranslationProgress::FileProgress(current) => {
                    // 프로그레스바 업데이트
                    let _ = SendMessageW(self.progress_bar, PBM_SETPOS, current as usize, 0);
                    let text = format!("{}/{}", current, self.state.list_size);
                    let _ = set_window_text(self.progress_text, &text);
                }
                FileTranslationProgress::TotalProgress(_) => {
                    let text = format!(
                        "전체: {}/{}",
                        self.state.current_line, self.state.total_lines
                    );
                    let _ = set_window_text(self.total_text, &text);

                    // 작업 표시줄 진행률 갱신 (전체 라인 기준 — 가장 안정적인 단조 증가 신호).
                }
                FileTranslationProgress::Finished(Ok(_)) => {
                    let _ = set_window_text(self.name_text, "완료!");
                    let _ = set_window_text(self.progress_text, "번역 완료");
                    let _ = EnableWindow(self.cancel_btn, 0);

                    // 작업 표시줄 진행률 해제
                    // 완료 메시지
                    let _ = MessageBoxW(
                        self.hwnd,
                        crate::win32::to_wide("번역을 완료했습니다.").as_ptr(),
                        crate::win32::to_wide("알림").as_ptr(),
                        MB_ICONINFORMATION,
                    );

                    // 창 닫기
                    let _ = PostMessageW(self.hwnd, WM_PROGRESS_FINISH, 0, 0);
                }
                FileTranslationProgress::Finished(Err(
                    crate::file_trans::FileTranslationError::Cancelled,
                )) => {
                    let _ = set_window_text(self.name_text, "취소됨");
                    let _ = set_window_text(self.progress_text, "번역을 취소했습니다");
                    let _ = EnableWindow(self.cancel_btn, 0);
                    self.clear_taskbar_progress();
                    let _ = PostMessageW(self.hwnd, WM_PROGRESS_FINISH, 0, 0);
                }
                FileTranslationProgress::Finished(Err(error_msg)) => {
                    let _ = set_window_text(self.name_text, "오류 발생");
                    let _ = EnableWindow(self.cancel_btn, 0);

                    // 작업 표시줄 진행률을 ERROR 상태로 잠깐 표시한 뒤
                    // MessageBox 가 닫히고 다이얼로그가 파괴되며 WM_DESTROY 에서 정리한다.
                    let message = crate::win32::to_wide(&error_msg.to_string());
                    let _ = MessageBoxW(
                        self.hwnd,
                        message.as_ptr(),
                        crate::win32::to_wide("오류").as_ptr(),
                        MB_ICONERROR,
                    );

                    self.clear_taskbar_progress();
                    let _ = PostMessageW(self.hwnd, WM_PROGRESS_FINISH, 0, 0);
                }
            }
        }
    }

    /// 취소 처리
    fn handle_cancel(&mut self) {
        self.task.cancel();
        // SAFETY: self.cancel_btn is a valid control handle from the RC template.
        unsafe {
            let _ = EnableWindow(self.cancel_btn, 0);
            let _ = set_window_text(self.progress_text, "취소 중...");
            // 작업 표시줄을 PAUSED 로 표시 — 실제 종료/정리는 워커 스레드가
            // 취소 토큰을 감지해 오류 이벤트를 보낼 때 마무리된다.
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/dialogs/file_trans_progress.rs"]
mod tests;
