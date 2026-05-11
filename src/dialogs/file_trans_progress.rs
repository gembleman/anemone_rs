//! 파일 번역 진행률 대화상자
//!
//! 번역 진행 상황 표시 및 취소 기능.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

use crate::constants::{
    WM_PROGRESS_COMPLETE, WM_PROGRESS_CURRENT, WM_PROGRESS_ERROR, WM_PROGRESS_INDEX,
    WM_PROGRESS_LIST_SIZE, WM_PROGRESS_NAME, WM_PROGRESS_TOTAL_COUNT, WM_PROGRESS_TOTAL_SIZE,
    WM_PROGRESS_UPDATE,
};
use crate::define_dialog_instance;
use crate::util::to_wide;
use super::helpers::{Dialog, DialogControls};

// 컨트롤 ID
mod ctrl_id {
    pub const NAME_TEXT: u16 = 5001; // 현재 파일명
    pub const PROGRESS_BAR: u16 = 5002; // 프로그레스바
    pub const PROGRESS_TEXT: u16 = 5003; // 진행 텍스트
    pub const INDEX_TEXT: u16 = 5004; // 파일 인덱스
    pub const TOTAL_TEXT: u16 = 5005; // 전체 진행
    pub const BTN_CANCEL: u16 = 5010; // 취소 버튼
}

const PROGRESS_WIDTH: i32 = 450;
// WIDTH / HEIGHT 는 캡션·테두리를 포함한 전체 윈도우 크기다. 캡션(~24~30px)+테두리(~2px)
// 가 클라이언트에서 차감되므로, 취소 버튼 하단 Y=180 이 잘리지 않으려면 캡션 여유까지
// 합쳐 220 이상이 필요. 200 일 때는 일부 윈도우 테마에서 버튼 하단 ~8px 가 잘릴 수 있다.
const PROGRESS_HEIGHT: i32 = 220;

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

impl DialogControls for FileTransProgressDialog {
    fn dialog_hwnd(&self) -> HWND {
        self.hwnd
    }
}

define_dialog_instance!(PROGRESS_INSTANCE: FileTransProgressDialog);

impl Dialog for FileTransProgressDialog {
    type Params = Arc<AtomicBool>;

    const CLASS_NAME: PCWSTR = w!("AnemoneFileTransProgressClass");
    const TITLE: PCWSTR = w!("파일 번역 진행 중");
    const WIDTH: i32 = PROGRESS_WIDTH;
    const HEIGHT: i32 = PROGRESS_HEIGHT;
    const EXTRA_STYLE: WINDOW_STYLE = WINDOW_STYLE(0);

    fn instance_slot()
        -> &'static std::thread::LocalKey<
            std::cell::RefCell<Option<std::rc::Rc<std::cell::RefCell<Self>>>>,
        > {
        &PROGRESS_INSTANCE
    }

    fn use_parent_centered() -> bool {
        true
    }

    fn init(hwnd: HWND, parent: HWND, cancel_token: Self::Params) -> Self {
        // 작업 표시줄 진행률 인터페이스 초기화.
        // UI 스레드는 main()에서 STA 로 1회 초기화되어 있다. CoCreateInstance / HrInit
        // 실패는 모두 Option 으로 흡수해 작업표시줄 진행률 없이 동작하도록 한다.
        // SAFETY: STA 초기화는 main()에서 보장. parent 는 호출자가 제공한 유효 핸들.
        let taskbar: Option<ITaskbarList3> = unsafe {
            match CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_ALL) {
                Ok(t) => match t.HrInit() {
                    Ok(()) => {
                        // 부모 윈도우에 진행 중 상태 표시 시작
                        let _ = t.SetProgressState(parent, TBPF_NORMAL);
                        Some(t)
                    }
                    Err(e) => {
                        tracing::warn!("ITaskbarList3::HrInit failed: {e}");
                        None
                    }
                },
                Err(e) => {
                    tracing::warn!("CoCreateInstance(TaskbarList) failed: {e}");
                    None
                }
            }
        };

        FileTransProgressDialog {
            hwnd,
            parent_hwnd: parent,
            cancel_token,
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

    fn create_controls(&mut self) -> Result<()> {
        // SAFETY: self.hwnd 는 show() 단계에서 만든 유효 핸들. DialogControls 의 모든 헬퍼는
        // 그 hwnd 위에서 자식 컨트롤만 생성한다.
        unsafe {
            // ====== 현재 파일 정보 그룹 ======
            self.create_group_box(10, 5, 425, 50, "현재 파일")?;
            self.name_text = self.create_label_with_id(20, 25, 405, 20, ctrl_id::NAME_TEXT, "대기 중...")?;

            // ====== 진행률 그룹 ======
            self.create_group_box(10, 60, 425, 85, "진행률")?;

            // 프로그레스바 (msctls_progress32) — DialogControls 에 헬퍼가 없어 직접 생성.
            self.progress_bar = self.create_progress_bar(20, 80, 405, 20, ctrl_id::PROGRESS_BAR)?;

            self.progress_text = self.create_label_with_id(20, 105, 200, 20, ctrl_id::PROGRESS_TEXT, "0/0")?;
            self.index_text = self.create_label_with_id(230, 105, 90, 20, ctrl_id::INDEX_TEXT, "파일: 0/0")?;
            self.total_text = self.create_label_with_id(330, 105, 95, 20, ctrl_id::TOTAL_TEXT, "전체: 0/0")?;

            // ====== 취소 버튼 ======
            self.cancel_btn = self.create_button(175, 150, 100, 30, ctrl_id::BTN_CANCEL, "취소")?;

            Ok(())
        }
    }

    fn handle_command(&mut self, cmd: u16, _notify_code: u32) {
        if cmd == ctrl_id::BTN_CANCEL {
            self.handle_cancel();
        }
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        // 진행률 메시지는 trait 의 기본 분기보다 먼저 잡아서 처리.
        if (WM_PROGRESS_TOTAL_SIZE..=WM_PROGRESS_ERROR).contains(&msg) {
            self.handle_progress_message(msg, wparam, lparam);
            return Some(LRESULT(0));
        }

        match msg {
            // 닫기 버튼은 무시 — 취소 버튼 또는 워커 완료/에러로만 닫힌다.
            WM_CLOSE => Some(LRESULT(0)),
            // WM_DESTROY 시점에 작업 표시줄 진행률을 비운 뒤, 기본 cleanup
            // (인스턴스 슬롯 해제) 으로 흘려보낸다.
            WM_DESTROY => {
                if let Some(ref tb) = self.taskbar {
                    // SAFETY: self.parent_hwnd 는 init() 에서 받은 유효 핸들.
                    unsafe {
                        let _ = tb.SetProgressState(self.parent_hwnd, TBPF_NOPROGRESS);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// 다이얼로그 인스턴스가 사라진 뒤 도착한 진행률 메시지의 lparam 박스를 회수.
    fn on_orphan_message(msg: u32, _w: WPARAM, lparam: LPARAM) {
        if (msg == WM_PROGRESS_NAME || msg == WM_PROGRESS_ERROR) && lparam.0 != 0 {
            // SAFETY: lparam 은 워커가 Box::into_raw 로 넘긴 *mut Vec<u16> 이다.
            unsafe {
                let _ = Box::from_raw(lparam.0 as *mut Vec<u16>);
            }
        }
    }
}

impl FileTransProgressDialog {
    /// 프로그레스바(msctls_progress32) 자식 컨트롤을 생성한다.
    unsafe fn create_progress_bar(&self, x: i32, y: i32, w: i32, h: i32, id: u16) -> Result<HWND> {
        // SAFETY: self.hwnd 는 유효 핸들. msctls_progress32 는 표준 공통 컨트롤 클래스.
        unsafe {
            use windows::Win32::System::LibraryLoader::GetModuleHandleW;

            let hinst = GetModuleHandleW(None)?;
            let dpi = crate::dpi::dpi_for_window(self.hwnd);
            let s = |v: i32| crate::dpi::scale(v, dpi);

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("msctls_progress32"),
                w!(""),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
                s(x),
                s(y),
                s(w),
                s(h),
                Some(self.hwnd),
                Some(HMENU(id as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;

            Ok(hwnd)
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
        // SAFETY: All control handles were created in create_controls and are valid.
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
                    if let Some(ref tb) = self.taskbar {
                        if self.state.total_lines > 0 {
                            let _ = tb.SetProgressValue(
                                self.parent_hwnd,
                                self.state.current_line.max(0) as u64,
                                self.state.total_lines as u64,
                            );
                        }
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

                    let _ = DestroyWindow(self.hwnd);
                }
                _ => {}
            }
        }
    }

    /// 취소 처리
    fn handle_cancel(&mut self) {
        self.cancel_token.store(true, Ordering::SeqCst);
        // SAFETY: self.cancel_btn is a valid control handle from create_controls.
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
