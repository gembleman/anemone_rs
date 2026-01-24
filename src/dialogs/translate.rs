//! 번역 대화상자
//!
//! 수동 번역 입력을 위한 대화상자.
//! Edit 컨트롤 서브클래싱으로 Ctrl+A 전체 선택 지원.

use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::DataExchange::*,
        System::LibraryLoader::GetModuleHandleW, System::Memory::*, UI::Controls::*,
        UI::Input::KeyboardAndMouse::*, UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::config::Config;

const CF_UNICODETEXT: u32 = 13;

// 컨트롤 ID
mod ctrl_id {
    pub const SOURCE_EDIT: u16 = 2001;
    pub const DEST_EDIT: u16 = 2002;
    pub const BTN_TRANSLATE: u16 = 2003;
    pub const BTN_COPY: u16 = 2004;
    pub const BTN_CLEAR: u16 = 2005;
    pub const CHK_ONE_GO: u16 = 2006;
    pub const CHK_NO_LINEFEED: u16 = 2007;
    pub const RADIO_OUTPUT_1: u16 = 2010;
    pub const RADIO_OUTPUT_2: u16 = 2011;
    pub const RADIO_OUTPUT_3: u16 = 2012;
}

const TRANSLATE_CLASS_NAME: PCWSTR = w!("AnemoneTranslateClass");
const TRANSLATE_WIDTH: i32 = 500;
const TRANSLATE_HEIGHT: i32 = 450;

/// 출력 형식
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Normal = 0, // 일반
    Brackets = 1,  // 괄호 포함
    NameSplit = 2, // 이름 분리
}

/// 번역 대화상자
pub struct TranslateDialog {
    hwnd: HWND,
    main_hwnd: HWND,
    #[allow(dead_code)]
    config: Rc<RefCell<Config>>,
    source_edit: HWND,
    dest_edit: HWND,
    one_go: bool,
    no_linefeed: bool,
    output_format: OutputFormat,
    original_source_proc: isize,
    original_dest_proc: isize,
}

thread_local! {
    static TRANSLATE_INSTANCE: RefCell<Option<Rc<RefCell<TranslateDialog>>>> = const { RefCell::new(None) };
}

impl TranslateDialog {
    /// 번역 대화상자 생성 및 표시
    pub fn show(main_hwnd: HWND, config: Rc<RefCell<Config>>) -> Result<HWND> {
        unsafe { Self::show_impl(main_hwnd, config) }
    }

    unsafe fn show_impl(main_hwnd: HWND, config: Rc<RefCell<Config>>) -> Result<HWND> {
        unsafe {
            let instance = GetModuleHandleW(None)?;

            // 윈도우 클래스 등록
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(Self::wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance.into(),
                hIcon: LoadIconW(None, IDI_APPLICATION)?,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as *mut _),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: TRANSLATE_CLASS_NAME,
                hIconSm: HICON::default(),
            };

            let atom = RegisterClassExW(&wc);
            if atom == 0 {
                let err = GetLastError();
                if err != ERROR_CLASS_ALREADY_EXISTS {
                    return Err(Error::from_hresult(HRESULT::from_win32(err.0)));
                }
            }

            // 화면 중앙에 위치
            let cx = GetSystemMetrics(SM_CXSCREEN);
            let cy = GetSystemMetrics(SM_CYSCREEN);
            let x = (cx - TRANSLATE_WIDTH) / 2;
            let y = (cy - TRANSLATE_HEIGHT) / 2;

            // 윈도우 생성
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                TRANSLATE_CLASS_NAME,
                w!("번역"),
                WS_POPUP | WS_CAPTION | WS_SYSMENU,
                x,
                y,
                TRANSLATE_WIDTH,
                TRANSLATE_HEIGHT,
                Some(main_hwnd),
                None,
                Some(instance.into()),
                None,
            )?;

            // 인스턴스 생성
            let dialog = Rc::new(RefCell::new(TranslateDialog {
                hwnd,
                main_hwnd,
                config,
                source_edit: HWND::default(),
                dest_edit: HWND::default(),
                one_go: false,
                no_linefeed: false,
                output_format: OutputFormat::Normal,
                original_source_proc: 0,
                original_dest_proc: 0,
            }));

            // 전역 인스턴스 설정
            TRANSLATE_INSTANCE.with(|cell| {
                *cell.borrow_mut() = Some(dialog.clone());
            });

            // 컨트롤 생성
            dialog.borrow_mut().create_controls()?;

            // 윈도우 표시
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);

            Ok(hwnd)
        }
    }

    /// 컨트롤 생성
    fn create_controls(&mut self) -> Result<()> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let hfont = GetStockObject(DEFAULT_GUI_FONT);

            // ====== 원문 입력 그룹 ======
            self.create_group_box(10, 5, 475, 150, "원문 입력")?;

            // 원문 Edit (multiline)
            self.source_edit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("EDIT"),
                w!(""),
                WINDOW_STYLE(
                    WS_CHILD.0
                        | WS_VISIBLE.0
                        | WS_VSCROLL.0
                        | ES_MULTILINE as u32
                        | ES_AUTOVSCROLL as u32
                        | ES_WANTRETURN as u32,
                ),
                20,
                25,
                455,
                120,
                Some(self.hwnd),
                Some(HMENU(ctrl_id::SOURCE_EDIT as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;
            let _ = SendMessageW(
                self.source_edit,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );
            // 텍스트 길이 무제한
            let _ = SendMessageW(
                self.source_edit,
                EM_SETLIMITTEXT,
                Some(WPARAM(0)),
                Some(LPARAM(0)),
            );
            // 서브클래싱
            self.original_source_proc = SetWindowLongPtrW(
                self.source_edit,
                GWLP_WNDPROC,
                Self::edit_subclass_proc as isize,
            );

            // ====== 번역 결과 그룹 ======
            self.create_group_box(10, 160, 475, 150, "번역 결과")?;

            // 번역 Edit (readonly, multiline)
            self.dest_edit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("EDIT"),
                w!(""),
                WINDOW_STYLE(
                    WS_CHILD.0
                        | WS_VISIBLE.0
                        | WS_VSCROLL.0
                        | ES_MULTILINE as u32
                        | ES_AUTOVSCROLL as u32
                        | ES_READONLY as u32,
                ),
                20,
                180,
                455,
                120,
                Some(self.hwnd),
                Some(HMENU(ctrl_id::DEST_EDIT as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;
            let _ = SendMessageW(
                self.dest_edit,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );
            let _ = SendMessageW(
                self.dest_edit,
                EM_SETLIMITTEXT,
                Some(WPARAM(0)),
                Some(LPARAM(0)),
            );
            // 서브클래싱
            self.original_dest_proc = SetWindowLongPtrW(
                self.dest_edit,
                GWLP_WNDPROC,
                Self::edit_subclass_proc as isize,
            );

            // ====== 옵션 그룹 ======
            self.create_group_box(10, 315, 230, 90, "옵션")?;

            // 체크박스들
            self.create_checkbox(
                20,
                335,
                100,
                20,
                ctrl_id::CHK_ONE_GO,
                "자동 번역",
                self.one_go,
            )?;
            self.create_checkbox(
                125,
                335,
                110,
                20,
                ctrl_id::CHK_NO_LINEFEED,
                "줄바꿈 제거",
                self.no_linefeed,
            )?;

            // 출력 형식 라디오 버튼
            self.create_label(20, 360, 70, 18, "출력 형식:")?;
            self.create_radio(
                95,
                358,
                50,
                20,
                ctrl_id::RADIO_OUTPUT_1,
                "일반",
                self.output_format == OutputFormat::Normal,
            )?;
            self.create_radio(
                150,
                358,
                50,
                20,
                ctrl_id::RADIO_OUTPUT_2,
                "괄호",
                self.output_format == OutputFormat::Brackets,
            )?;
            self.create_radio(
                205,
                358,
                50,
                20,
                ctrl_id::RADIO_OUTPUT_3,
                "분리",
                self.output_format == OutputFormat::NameSplit,
            )?;

            // ====== 버튼 그룹 ======
            self.create_group_box(250, 315, 235, 90, "동작")?;

            // 버튼들
            self.create_button(265, 340, 65, 28, ctrl_id::BTN_TRANSLATE, "번역")?;
            self.create_button(340, 340, 65, 28, ctrl_id::BTN_COPY, "복사")?;
            self.create_button(415, 340, 60, 28, ctrl_id::BTN_CLEAR, "초기화")?;

            Ok(())
        }
    }

    // 헬퍼 함수들 (settings.rs와 동일한 패턴)
    unsafe fn create_group_box(&self, x: i32, y: i32, w: i32, h: i32, text: &str) -> Result<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(text_wide.as_ptr()),
                WINDOW_STYLE(BS_GROUPBOX as u32 | WS_CHILD.0 | WS_VISIBLE.0),
                x,
                y,
                w,
                h,
                Some(self.hwnd),
                None,
                Some(hinst.into()),
                None,
            )?;

            let hfont = GetStockObject(DEFAULT_GUI_FONT);
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            Ok(hwnd)
        }
    }

    unsafe fn create_label(&self, x: i32, y: i32, w: i32, h: i32, text: &str) -> Result<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                PCWSTR(text_wide.as_ptr()),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
                x,
                y,
                w,
                h,
                Some(self.hwnd),
                None,
                Some(hinst.into()),
                None,
            )?;

            let hfont = GetStockObject(DEFAULT_GUI_FONT);
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            Ok(hwnd)
        }
    }

    unsafe fn create_button(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: u16,
        text: &str,
    ) -> Result<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(text_wide.as_ptr()),
                WINDOW_STYLE(BS_PUSHBUTTON as u32 | WS_CHILD.0 | WS_VISIBLE.0),
                x,
                y,
                w,
                h,
                Some(self.hwnd),
                Some(HMENU(id as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;

            let hfont = GetStockObject(DEFAULT_GUI_FONT);
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            Ok(hwnd)
        }
    }

    unsafe fn create_checkbox(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: u16,
        text: &str,
        checked: bool,
    ) -> Result<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(text_wide.as_ptr()),
                WINDOW_STYLE(BS_AUTOCHECKBOX as u32 | WS_CHILD.0 | WS_VISIBLE.0),
                x,
                y,
                w,
                h,
                Some(self.hwnd),
                Some(HMENU(id as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;

            let hfont = GetStockObject(DEFAULT_GUI_FONT);
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            if checked {
                let _ = SendMessageW(
                    hwnd,
                    BM_SETCHECK,
                    Some(WPARAM(BST_CHECKED.0 as usize)),
                    Some(LPARAM(0)),
                );
            }

            Ok(hwnd)
        }
    }

    unsafe fn create_radio(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: u16,
        text: &str,
        checked: bool,
    ) -> Result<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(text_wide.as_ptr()),
                WINDOW_STYLE(BS_AUTORADIOBUTTON as u32 | WS_CHILD.0 | WS_VISIBLE.0),
                x,
                y,
                w,
                h,
                Some(self.hwnd),
                Some(HMENU(id as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;

            let hfont = GetStockObject(DEFAULT_GUI_FONT);
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            if checked {
                let _ = SendMessageW(
                    hwnd,
                    BM_SETCHECK,
                    Some(WPARAM(BST_CHECKED.0 as usize)),
                    Some(LPARAM(0)),
                );
            }

            Ok(hwnd)
        }
    }

    /// Edit 서브클래스 프로시저 (Ctrl+A 지원)
    unsafe extern "system" fn edit_subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe {
            if msg == WM_KEYDOWN {
                // Ctrl+A 처리
                if wparam.0 == 'A' as usize {
                    let ctrl_pressed = (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;
                    if ctrl_pressed {
                        // 전체 선택
                        let _ = SendMessageW(hwnd, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                        return LRESULT(0);
                    }
                }
            }

            // 원래 프로시저 호출
            let instance = TRANSLATE_INSTANCE.with(|cell| cell.borrow().clone());
            if let Some(dialog) = instance {
                let dialog_ref = dialog.borrow();
                let original_proc = if hwnd == dialog_ref.source_edit {
                    dialog_ref.original_source_proc
                } else if hwnd == dialog_ref.dest_edit {
                    dialog_ref.original_dest_proc
                } else {
                    0
                };

                if original_proc != 0 {
                    let proc: WNDPROC = std::mem::transmute(original_proc);
                    return CallWindowProcW(proc, hwnd, msg, wparam, lparam);
                }
            }

            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }

    /// 원문 텍스트 가져오기
    fn get_source_text(&self) -> String {
        unsafe { Self::get_edit_text(self.source_edit) }
    }

    /// Edit 컨트롤에서 텍스트 가져오기
    unsafe fn get_edit_text(hwnd: HWND) -> String {
        unsafe {
            let len = GetWindowTextLengthW(hwnd);
            if len == 0 {
                return String::new();
            }

            let mut buffer: Vec<u16> = vec![0; (len + 1) as usize];
            GetWindowTextW(hwnd, &mut buffer);
            String::from_utf16_lossy(&buffer[..len as usize])
        }
    }

    /// Edit 컨트롤에 텍스트 설정
    unsafe fn set_edit_text(hwnd: HWND, text: &str) {
        unsafe {
            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
        }
    }

    /// 번역 결과 설정
    fn set_dest_text(&self, text: &str) {
        unsafe { Self::set_edit_text(self.dest_edit, text) }
    }

    /// 번역 수행 (현재는 원문을 그대로 복사 - 추후 번역 엔진 연동)
    fn do_translate(&mut self) {
        let source = self.get_source_text();
        if source.is_empty() {
            return;
        }

        // 줄바꿈 제거 옵션 처리
        let text = if self.no_linefeed {
            source.replace("\r\n", " ").replace('\n', " ")
        } else {
            source
        };

        // TODO: 실제 번역 엔진 연동
        // 현재는 원문을 그대로 표시 (플레이스홀더)
        let result = format!("[번역 결과]\n{}", text);
        self.set_dest_text(&result);
    }

    /// 번역 결과를 클립보드에 복사
    fn copy_to_clipboard(&self) {
        unsafe {
            let text = Self::get_edit_text(self.dest_edit);
            if text.is_empty() {
                return;
            }

            Self::set_clipboard_text(&text, self.hwnd);
        }
    }

    /// 클립보드에 텍스트 설정
    unsafe fn set_clipboard_text(text: &str, hwnd: HWND) {
        unsafe {
            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let byte_len = wide.len() * 2;

            if OpenClipboard(Some(hwnd)).is_err() {
                return;
            }

            let _ = EmptyClipboard();

            let hmem = GlobalAlloc(GMEM_MOVEABLE, byte_len).ok();
            if let Some(hmem) = hmem {
                let ptr = GlobalLock(hmem) as *mut u16;
                if !ptr.is_null() {
                    std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                    let _ = GlobalUnlock(hmem);

                    let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hmem.0)));
                }
            }

            let _ = CloseClipboard();
        }
    }

    /// 텍스트 초기화
    fn clear_text(&self) {
        unsafe {
            Self::set_edit_text(self.source_edit, "");
            Self::set_edit_text(self.dest_edit, "");
            let _ = SetFocus(Some(self.source_edit));
        }
    }

    /// 명령 처리
    fn handle_command(&mut self, cmd: u16) {
        use ctrl_id::*;

        match cmd {
            BTN_TRANSLATE => {
                self.do_translate();
            }
            BTN_COPY => {
                self.copy_to_clipboard();
            }
            BTN_CLEAR => {
                self.clear_text();
            }
            CHK_ONE_GO => {
                self.one_go = !self.one_go;
            }
            CHK_NO_LINEFEED => {
                self.no_linefeed = !self.no_linefeed;
            }
            RADIO_OUTPUT_1 => {
                self.output_format = OutputFormat::Normal;
            }
            RADIO_OUTPUT_2 => {
                self.output_format = OutputFormat::Brackets;
            }
            RADIO_OUTPUT_3 => {
                self.output_format = OutputFormat::NameSplit;
            }
            _ => {}
        }
    }

    /// WndProc
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe {
            let instance = TRANSLATE_INSTANCE.with(|cell| cell.borrow().clone());

            if let Some(dialog) = instance {
                match msg {
                    WM_COMMAND => {
                        let id = (wparam.0 & 0xFFFF) as u16;
                        let notify_code = ((wparam.0 >> 16) & 0xFFFF) as u32;

                        // Edit 변경 알림 (자동 번역용)
                        if notify_code == EN_CHANGE as u32 && id == ctrl_id::SOURCE_EDIT {
                            if dialog.borrow().one_go {
                                dialog.borrow_mut().do_translate();
                            }
                        } else {
                            dialog.borrow_mut().handle_command(id);
                        }
                        return LRESULT(0);
                    }

                    WM_CLOSE => {
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }

                    WM_DESTROY => {
                        // 인스턴스 정리
                        TRANSLATE_INSTANCE.with(|cell| {
                            *cell.borrow_mut() = None;
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
            }

            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}
