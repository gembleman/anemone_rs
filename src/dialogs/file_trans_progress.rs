//! 파일 번역 진행률 대화상자
//!
//! 번역 진행 상황 표시 및 취소 기능.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW, UI::Controls::*,
        UI::Input::KeyboardAndMouse::EnableWindow, UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::constants::{
    WM_PROGRESS_COMPLETE, WM_PROGRESS_CURRENT, WM_PROGRESS_ERROR, WM_PROGRESS_INDEX,
    WM_PROGRESS_LIST_SIZE, WM_PROGRESS_NAME, WM_PROGRESS_TOTAL_COUNT, WM_PROGRESS_TOTAL_SIZE,
    WM_PROGRESS_UPDATE,
};
use crate::constants::SS_LEFT;
use crate::util::to_wide;
use super::helpers::{self, DialogWindowOptions};

// 컨트롤 ID
mod ctrl_id {
    pub const NAME_TEXT: u16 = 5001; // 현재 파일명
    pub const PROGRESS_BAR: u16 = 5002; // 프로그레스바
    pub const PROGRESS_TEXT: u16 = 5003; // 진행 텍스트
    pub const INDEX_TEXT: u16 = 5004; // 파일 인덱스
    pub const TOTAL_TEXT: u16 = 5005; // 전체 진행
    pub const BTN_CANCEL: u16 = 5010; // 취소 버튼
}

const PROGRESS_CLASS_NAME: PCWSTR = w!("AnemoneFileTransProgressClass");
const PROGRESS_WIDTH: i32 = 450;
const PROGRESS_HEIGHT: i32 = 200;

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
}

thread_local! {
    static PROGRESS_INSTANCE: RefCell<Option<Box<FileTransProgressDialog>>> = const { RefCell::new(None) };
}

impl FileTransProgressDialog {
    /// 진행률 대화상자 생성 및 표시
    pub fn show(parent_hwnd: HWND, cancel_token: Arc<AtomicBool>) -> Result<HWND> {
        unsafe { Self::show_impl(parent_hwnd, cancel_token) }
    }

    unsafe fn show_impl(parent_hwnd: HWND, cancel_token: Arc<AtomicBool>) -> Result<HWND> {
        // SAFETY: parent_hwnd is a valid window handle from the caller. Window class
        // registration and window creation use valid Win32 parameters.
        unsafe {
            helpers::register_dialog_class(PROGRESS_CLASS_NAME, Self::wndproc)?;

            let hwnd = helpers::create_dialog_window_centered_on_parent(&DialogWindowOptions {
                class_name: PROGRESS_CLASS_NAME,
                title: w!("파일 번역 진행 중"),
                width: PROGRESS_WIDTH,
                height: PROGRESS_HEIGHT,
                parent: parent_hwnd,
                extra_style: WINDOW_STYLE::default(),
            })?;

            // 인스턴스 생성
            let mut dialog = Box::new(FileTransProgressDialog {
                hwnd,
                parent_hwnd,
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
            });

            // 컨트롤 생성
            dialog.create_controls()?;

            // 전역 인스턴스 설정
            PROGRESS_INSTANCE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    *guard = Some(dialog);
                }
            });

            helpers::show_dialog_window(hwnd);

            Ok(hwnd)
        }
    }

    /// 컨트롤 생성
    fn create_controls(&mut self) -> Result<()> {
        // SAFETY: self.hwnd is a valid window handle from show_impl. All CreateWindowExW
        // calls use valid parent handle and instance. SendMessageW uses valid control handles.
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let hfont = GetStockObject(DEFAULT_GUI_FONT);

            // ====== 현재 파일 정보 그룹 ======
            self.create_group_box(10, 5, 425, 50, "현재 파일")?;

            // 파일명 텍스트
            self.name_text = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("대기 중..."),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | SS_LEFT as u32),
                20,
                25,
                405,
                20,
                Some(self.hwnd),
                Some(HMENU(ctrl_id::NAME_TEXT as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;
            let _ = SendMessageW(
                self.name_text,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            // ====== 진행률 그룹 ======
            self.create_group_box(10, 60, 425, 85, "진행률")?;

            // 프로그레스바
            self.progress_bar = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("msctls_progress32"),
                w!(""),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
                20,
                80,
                405,
                20,
                Some(self.hwnd),
                Some(HMENU(ctrl_id::PROGRESS_BAR as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;

            // 진행 텍스트
            self.progress_text = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("0/0"),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | SS_LEFT as u32),
                20,
                105,
                200,
                20,
                Some(self.hwnd),
                Some(HMENU(ctrl_id::PROGRESS_TEXT as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;
            let _ = SendMessageW(
                self.progress_text,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            // 파일 인덱스 텍스트
            self.index_text = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("파일: 0/0"),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | SS_LEFT as u32),
                230,
                105,
                90,
                20,
                Some(self.hwnd),
                Some(HMENU(ctrl_id::INDEX_TEXT as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;
            let _ = SendMessageW(
                self.index_text,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            // 전체 진행 텍스트
            self.total_text = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("전체: 0/0"),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | SS_LEFT as u32),
                330,
                105,
                95,
                20,
                Some(self.hwnd),
                Some(HMENU(ctrl_id::TOTAL_TEXT as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;
            let _ = SendMessageW(
                self.total_text,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            // ====== 취소 버튼 ======
            self.cancel_btn = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!("취소"),
                WINDOW_STYLE(BS_PUSHBUTTON as u32 | WS_CHILD.0 | WS_VISIBLE.0),
                175,
                150,
                100,
                30,
                Some(self.hwnd),
                Some(HMENU(ctrl_id::BTN_CANCEL as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;
            let _ = SendMessageW(
                self.cancel_btn,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            Ok(())
        }
    }

    unsafe fn create_group_box(&self, x: i32, y: i32, w: i32, h: i32, text: &str) -> Result<HWND> {
        // SAFETY: self.hwnd is a valid window handle; delegating to helpers with valid params.
        unsafe { helpers::create_group_box(self.hwnd, x, y, w, h, text) }
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
        // lparam pointers for WM_PROGRESS_NAME/WM_PROGRESS_ERROR point to static buffers
        // in file_trans_thread that outlive the message processing.
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
                    // lparam은 파일명 문자열 포인터
                    if lparam.0 != 0 {
                        let ptr = lparam.0 as *const u16;
                        let mut len = 0;
                        while *ptr.add(len) != 0 {
                            len += 1;
                        }
                        let slice = std::slice::from_raw_parts(ptr, len);
                        let filename = String::from_utf16_lossy(slice);
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
                }
                WM_PROGRESS_COMPLETE => {
                    Self::set_text(self.name_text, "완료!");
                    Self::set_text(self.progress_text, "번역 완료");
                    let _ = EnableWindow(self.cancel_btn, false);

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
                    // lparam은 에러 메시지 문자열 포인터
                    let error_msg = if lparam.0 != 0 {
                        let ptr = lparam.0 as *const u16;
                        let mut len = 0;
                        while *ptr.add(len) != 0 {
                            len += 1;
                        }
                        let slice = std::slice::from_raw_parts(ptr, len);
                        String::from_utf16_lossy(slice)
                    } else {
                        "알 수 없는 오류가 발생했습니다.".to_string()
                    };

                    Self::set_text(self.name_text, "오류 발생");
                    let _ = EnableWindow(self.cancel_btn, false);

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
        }
    }

    /// WndProc
    // SAFETY: This is a Win32 window procedure callback. The system guarantees valid parameters.
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: All Win32 API calls use valid system-provided parameters.
        unsafe {
            // 진행률 메시지 범위 체크
            if msg >= WM_PROGRESS_TOTAL_SIZE && msg <= WM_PROGRESS_ERROR {
                PROGRESS_INSTANCE.with(|cell| {
                    if let Ok(mut guard) = cell.try_borrow_mut() {
                        if let Some(ref mut dialog) = *guard {
                            dialog.handle_progress_message(msg, wparam, lparam);
                        }
                    }
                });
                return LRESULT(0);
            }

            match msg {
                WM_COMMAND => {
                    let id = (wparam.0 & 0xFFFF) as u16;
                    if id == ctrl_id::BTN_CANCEL {
                        PROGRESS_INSTANCE.with(|cell| {
                            if let Ok(mut guard) = cell.try_borrow_mut() {
                                if let Some(ref mut dialog) = *guard {
                                    dialog.handle_cancel();
                                }
                            }
                        });
                    }
                    return LRESULT(0);
                }

                WM_CLOSE => {
                    // 닫기 버튼 무시 (취소 버튼으로만 닫을 수 있음)
                    return LRESULT(0);
                }

                WM_DESTROY => {
                    PROGRESS_INSTANCE.with(|cell| {
                        if let Ok(mut guard) = cell.try_borrow_mut() {
                            *guard = None;
                        }
                    });
                    return LRESULT(0);
                }

                WM_LBUTTONDOWN => {
                    // 창 드래그
                    let _ = SendMessageW(
                        hwnd,
                        WM_NCLBUTTONDOWN,
                        Some(WPARAM(HTCAPTION as usize)),
                        Some(LPARAM(0)),
                    );
                    return LRESULT(0);
                }

                _ => {}
            }

            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}
