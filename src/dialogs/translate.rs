//! 번역 대화상자
//!
//! 수동 번역 입력을 위한 대화상자.
//! Edit 컨트롤 서브클래싱으로 Ctrl+A 전체 선택 지원.
//! 번역 엔진 선택 (EzTrans, Google, DeepL) 및 언어 선택 지원.

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
use crate::translation::{
    get_translation_manager, Language, TranslationEngine, TranslationResult,
    TranslationWorker, WM_TRANSLATION_COMPLETE, take_all_responses,
};

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
    // 번역 엔진 선택
    pub const COMBO_ENGINE: u16 = 2020;
    pub const COMBO_SOURCE_LANG: u16 = 2021;
    pub const COMBO_TARGET_LANG: u16 = 2022;
}

const TRANSLATE_CLASS_NAME: PCWSTR = w!("AnemoneTranslateClass");
const TRANSLATE_WIDTH: i32 = 500;
const TRANSLATE_HEIGHT: i32 = 520;

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
    config: Rc<RefCell<Config>>,
    source_edit: HWND,
    dest_edit: HWND,
    engine_combo: HWND,
    source_lang_combo: HWND,
    target_lang_combo: HWND,
    one_go: bool,
    no_linefeed: bool,
    output_format: OutputFormat,
    original_source_proc: isize,
    original_dest_proc: isize,
    engine_initialized: bool,
    /// 비동기 번역 워커
    translation_worker: Option<TranslationWorker>,
    /// 번역 진행 중 여부
    translating: bool,
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

            // 번역 워커 생성
            let translation_worker = TranslationWorker::spawn(hwnd);

            // 인스턴스 생성
            let dialog = Rc::new(RefCell::new(TranslateDialog {
                hwnd,
                main_hwnd,
                config,
                source_edit: HWND::default(),
                dest_edit: HWND::default(),
                engine_combo: HWND::default(),
                source_lang_combo: HWND::default(),
                target_lang_combo: HWND::default(),
                one_go: false,
                no_linefeed: false,
                output_format: OutputFormat::Normal,
                original_source_proc: 0,
                original_dest_proc: 0,
                engine_initialized: false,
                translation_worker: Some(translation_worker),
                translating: false,
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

            // ====== 번역 엔진 선택 그룹 ======
            self.create_group_box(10, 5, 475, 55, "번역 설정")?;

            // 엔진 선택
            self.create_label(20, 28, 40, 18, "엔진:")?;
            self.engine_combo = self.create_combobox(65, 25, 100, 150, ctrl_id::COMBO_ENGINE)?;
            self.add_combobox_item(self.engine_combo, "EzTrans");
            self.add_combobox_item(self.engine_combo, "Google");
            self.add_combobox_item(self.engine_combo, "DeepL");

            // 소스 언어
            self.create_label(180, 28, 40, 18, "소스:")?;
            self.source_lang_combo =
                self.create_combobox(220, 25, 100, 150, ctrl_id::COMBO_SOURCE_LANG)?;
            self.add_combobox_item(self.source_lang_combo, "일본어");
            self.add_combobox_item(self.source_lang_combo, "한국어");
            self.add_combobox_item(self.source_lang_combo, "영어");
            self.add_combobox_item(self.source_lang_combo, "중국어(간체)");
            self.add_combobox_item(self.source_lang_combo, "중국어(번체)");

            // 타겟 언어
            self.create_label(335, 28, 40, 18, "타겟:")?;
            self.target_lang_combo =
                self.create_combobox(375, 25, 100, 150, ctrl_id::COMBO_TARGET_LANG)?;
            self.add_combobox_item(self.target_lang_combo, "일본어");
            self.add_combobox_item(self.target_lang_combo, "한국어");
            self.add_combobox_item(self.target_lang_combo, "영어");
            self.add_combobox_item(self.target_lang_combo, "중국어(간체)");
            self.add_combobox_item(self.target_lang_combo, "중국어(번체)");

            // 설정에서 초기값 로드
            {
                let config = self.config.borrow();
                let engine = config.translation.get_engine();
                let _ = SendMessageW(
                    self.engine_combo,
                    CB_SETCURSEL,
                    Some(WPARAM(config.translation.engine_as_u8() as usize)),
                    None,
                );
                let _ = SendMessageW(
                    self.source_lang_combo,
                    CB_SETCURSEL,
                    Some(WPARAM(config.translation.source_lang_index(engine))),
                    None,
                );
                let _ = SendMessageW(
                    self.target_lang_combo,
                    CB_SETCURSEL,
                    Some(WPARAM(config.translation.target_lang_index(engine))),
                    None,
                );
            }

            // ====== 원문 입력 그룹 ======
            self.create_group_box(10, 65, 475, 130, "원문 입력")?;

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
                85,
                455,
                100,
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
            #[cfg(target_pointer_width = "64")]
            {
                self.original_source_proc = SetWindowLongPtrW(
                    self.source_edit,
                    GWLP_WNDPROC,
                    Self::edit_subclass_proc as isize,
                );
            }
            #[cfg(target_pointer_width = "32")]
            {
                self.original_source_proc = SetWindowLongW(
                    self.source_edit,
                    GWLP_WNDPROC,
                    Self::edit_subclass_proc as i32,
                ) as isize;
            }

            // ====== 번역 결과 그룹 ======
            self.create_group_box(10, 200, 475, 130, "번역 결과")?;

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
                220,
                455,
                100,
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
            #[cfg(target_pointer_width = "64")]
            {
                self.original_dest_proc = SetWindowLongPtrW(
                    self.dest_edit,
                    GWLP_WNDPROC,
                    Self::edit_subclass_proc as isize,
                );
            }
            #[cfg(target_pointer_width = "32")]
            {
                self.original_dest_proc = SetWindowLongW(
                    self.dest_edit,
                    GWLP_WNDPROC,
                    Self::edit_subclass_proc as i32,
                ) as isize;
            }

            // ====== 옵션 그룹 ======
            self.create_group_box(10, 335, 230, 90, "옵션")?;

            // 체크박스들
            self.create_checkbox(
                20,
                355,
                100,
                20,
                ctrl_id::CHK_ONE_GO,
                "자동 번역",
                self.one_go,
            )?;
            self.create_checkbox(
                125,
                355,
                110,
                20,
                ctrl_id::CHK_NO_LINEFEED,
                "줄바꿈 제거",
                self.no_linefeed,
            )?;

            // 출력 형식 라디오 버튼
            self.create_label(20, 380, 70, 18, "출력 형식:")?;
            self.create_radio(
                95,
                378,
                50,
                20,
                ctrl_id::RADIO_OUTPUT_1,
                "일반",
                self.output_format == OutputFormat::Normal,
            )?;
            self.create_radio(
                150,
                378,
                50,
                20,
                ctrl_id::RADIO_OUTPUT_2,
                "괄호",
                self.output_format == OutputFormat::Brackets,
            )?;
            self.create_radio(
                205,
                378,
                50,
                20,
                ctrl_id::RADIO_OUTPUT_3,
                "분리",
                self.output_format == OutputFormat::NameSplit,
            )?;

            // ====== 버튼 그룹 ======
            self.create_group_box(250, 335, 235, 90, "동작")?;

            // 버튼들
            self.create_button(265, 360, 65, 28, ctrl_id::BTN_TRANSLATE, "번역")?;
            self.create_button(340, 360, 65, 28, ctrl_id::BTN_COPY, "복사")?;
            self.create_button(415, 360, 60, 28, ctrl_id::BTN_CLEAR, "초기화")?;

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

    unsafe fn create_combobox(&self, x: i32, y: i32, w: i32, h: i32, id: u16) -> Result<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("COMBOBOX"),
                w!(""),
                WINDOW_STYLE(
                    WS_CHILD.0
                        | WS_VISIBLE.0
                        | CBS_DROPDOWNLIST as u32
                        | CBS_HASSTRINGS as u32
                        | WS_VSCROLL.0,
                ),
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

    fn add_combobox_item(&self, combo: HWND, text: &str) {
        unsafe {
            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = SendMessageW(
                combo,
                CB_ADDSTRING,
                None,
                Some(LPARAM(wide.as_ptr() as isize)),
            );
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

            // 원래 프로시저를 borrow 범위 밖에서 가져옴 (재귀 호출 시 borrow 충돌 방지)
            let original_proc = TRANSLATE_INSTANCE.with(|cell| {
                if let Some(ref dialog) = *cell.borrow() {
                    // try_borrow를 사용하여 이미 borrow된 경우 0 반환
                    if let Ok(dialog_ref) = dialog.try_borrow() {
                        if hwnd == dialog_ref.source_edit {
                            return dialog_ref.original_source_proc;
                        } else if hwnd == dialog_ref.dest_edit {
                            return dialog_ref.original_dest_proc;
                        }
                    }
                }
                0
            });

            if original_proc != 0 {
                let proc: WNDPROC = std::mem::transmute(original_proc);
                return CallWindowProcW(proc, hwnd, msg, wparam, lparam);
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

    /// 번역 엔진 초기화
    fn init_translation_engine(&mut self) -> std::result::Result<(), String> {
        if self.engine_initialized {
            return Ok(());
        }

        let config = self.config.borrow();
        let engine = config.translation.get_engine();
        let manager = get_translation_manager();

        if let Ok(mut mgr) = manager.lock() {
            // EzTrans 초기화
            if engine == TranslationEngine::EzTrans {
                if config.translation.eztrans_dll_path.is_empty()
                    || config.translation.eztrans_dat_path.is_empty()
                {
                    return Err(
                        "EzTrans 경로가 설정되지 않았습니다. 번역 설정에서 경로를 지정하세요."
                            .to_string(),
                    );
                }
                if let Err(e) = mgr.init_eztrans(
                    &config.translation.eztrans_dll_path,
                    &config.translation.eztrans_dat_path,
                ) {
                    return Err(format!("EzTrans 초기화 실패: {}", e));
                }
            }

            // DeepL API 키 설정
            if !config.translation.deepl_api_key.is_empty() {
                mgr.set_deepl_api_key(config.translation.deepl_api_key.clone());
            }

            // 엔진 설정
            mgr.set_engine(engine);
            mgr.set_source_language(config.translation.get_source_language());
            mgr.set_target_language(config.translation.get_target_language());
        } else {
            return Err("번역 매니저 잠금 실패".to_string());
        }

        self.engine_initialized = true;
        Ok(())
    }

    /// 현재 선택된 엔진/언어를 매니저에 적용
    fn apply_current_settings(&self) {
        unsafe {
            let engine_idx = SendMessageW(self.engine_combo, CB_GETCURSEL, None, None).0 as usize;
            let source_idx =
                SendMessageW(self.source_lang_combo, CB_GETCURSEL, None, None).0 as usize;
            let target_idx =
                SendMessageW(self.target_lang_combo, CB_GETCURSEL, None, None).0 as usize;

            let engine = TranslationEngine::from_u8(engine_idx as u8);
            let supported_source = engine.supported_source_languages();
            let supported_target = engine.supported_target_languages();

            let source_lang = supported_source.get(source_idx).copied().unwrap_or(Language::Jpn);
            let target_lang = supported_target.get(target_idx).copied().unwrap_or(Language::Kor);

            let manager = get_translation_manager();
            if let Ok(mut mgr) = manager.lock() {
                mgr.set_engine(engine);
                mgr.set_source_language(source_lang);
                mgr.set_target_language(target_lang);
            }

            // 설정 저장
            {
                let mut config = self.config.borrow_mut();
                config.translation.set_engine(engine);
                config.translation.set_source_language(source_lang);
                config.translation.set_target_language(target_lang);
            }
        }
    }

    /// 번역 수행 (비동기)
    fn do_translate(&mut self) {
        let source = self.get_source_text();
        if source.is_empty() {
            return;
        }

        // 이미 번역 중이면 무시
        if self.translating {
            return;
        }

        // 엔진 초기화
        if let Err(e) = self.init_translation_engine() {
            self.set_dest_text(&format!("[오류] {}", e));
            return;
        }

        // 현재 설정 적용
        self.apply_current_settings();

        // 줄바꿈 제거 옵션 처리
        let text = if self.no_linefeed {
            source.replace("\r\n", " ").replace('\n', " ")
        } else {
            source
        };

        // 현재 엔진 설정 가져오기
        let (engine, source_lang, target_lang, deepl_api_key) = {
            let config = self.config.borrow();
            let engine = config.translation.get_engine();
            let source_lang = config.translation.get_source_language();
            let target_lang = config.translation.get_target_language();
            let deepl_api_key = if engine == TranslationEngine::DeepL {
                Some(config.translation.deepl_api_key.clone())
            } else {
                None
            };
            (engine, source_lang, target_lang, deepl_api_key)
        };

        // 번역 중 표시
        self.set_dest_text("[번역 중...]");
        self.translating = true;

        // 워커에 번역 요청
        if let Some(ref mut worker) = self.translation_worker {
            worker.translate(
                text,
                engine,
                source_lang,
                target_lang,
                deepl_api_key,
            );
        }
    }

    /// 번역 완료 처리
    fn handle_translation_complete(&mut self) {
        self.translating = false;

        // 모든 완료된 번역 결과 가져오기
        let responses = take_all_responses();

        for response in responses {
            let result = match response.result {
                TranslationResult::Success(translated) => {
                    // 출력 형식 적용
                    match self.output_format {
                        OutputFormat::Normal => translated,
                        OutputFormat::Brackets => format!("「{}」", translated),
                        OutputFormat::NameSplit => {
                            // 이름 분리 처리 (간단 구현)
                            if let Some((name, rest)) = translated.split_once([':', '：']) {
                                format!("{}\n{}", name.trim(), rest.trim())
                            } else {
                                translated
                            }
                        }
                    }
                }
                TranslationResult::Error(err) => {
                    format!("[오류] {}", err)
                }
            };

            self.set_dest_text(&result);
        }
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
    fn handle_command(&mut self, cmd: u16, notify_code: u32) {
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
            COMBO_ENGINE | COMBO_SOURCE_LANG | COMBO_TARGET_LANG => {
                // CBN_SELCHANGE
                if notify_code == 1 {
                    self.apply_current_settings();
                }
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
                            dialog.borrow_mut().handle_command(id, notify_code);
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

                    // 번역 완료 메시지
                    msg if msg == WM_TRANSLATION_COMPLETE => {
                        dialog.borrow_mut().handle_translation_complete();
                        return LRESULT(0);
                    }

                    _ => {}
                }
            }

            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}
