//! 백로그 대화상자
//!
//! RichEdit 기반 이력 뷰어.
//! 원문/번역 필터링 및 파일 저장 지원.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;

use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::{GetModuleHandleW, LoadLibraryW},
        UI::Controls::*,
        UI::Controls::Dialogs::{
            GetSaveFileNameW, OPENFILENAMEW, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST,
        },
        UI::WindowsAndMessaging::*,
    },
};

use crate::config::Config;

// 컨트롤 ID
mod ctrl_id {
    pub const RICHEDIT: u16 = 3001;
    pub const CHK_LINEFEED: u16 = 3002;
    pub const RADIO_ORIGINAL: u16 = 3010;
    pub const RADIO_TRANSLATION: u16 = 3011;
    pub const RADIO_ALL: u16 = 3012;
    pub const BTN_CLEAR: u16 = 3020;
    pub const BTN_SAVE: u16 = 3021;
    pub const BTN_FONT: u16 = 3022;
}

const BACKLOG_CLASS_NAME: PCWSTR = w!("AnemoneBacklogClass");
const BACKLOG_WIDTH: i32 = 600;
const BACKLOG_HEIGHT: i32 = 500;

// RichEdit 메시지 (windows crate에서 누락된 것들)
const EM_SETCHARFORMAT: u32 = WM_USER + 68;
const EM_REPLACESEL: u32 = 0x00C2;
const EM_SCROLLCARET: u32 = 0x00B7;
const EM_SETBKGNDCOLOR: u32 = WM_USER + 67;

// CHARFORMAT2W 마스크
const CFM_COLOR: u32 = 0x40000000;
const CFM_BOLD: u32 = 0x00000001;
const CFM_SIZE: u32 = 0x80000000;
const CFM_FACE: u32 = 0x20000000;

// CHARFORMAT2W 효과
const CFE_BOLD: u32 = 0x00000001;

// 선택 범위
const SCF_SELECTION: u32 = 0x0001;

/// 백로그 필터
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum BacklogFilter {
    Original,
    Translation,
    #[default]
    All,
}

/// 로그 항목
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub name: Option<String>,
    pub original: String,
    pub translation: Option<String>,
    pub timestamp: SystemTime,
}

impl LogEntry {
    pub fn new(original: String) -> Self {
        Self {
            name: None,
            original,
            translation: None,
            timestamp: SystemTime::now(),
        }
    }

    pub fn with_name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn with_translation(mut self, translation: String) -> Self {
        self.translation = Some(translation);
        self
    }
}

/// CHARFORMAT2W 구조체 (windows crate에서 직접 사용하기 어려워 수동 정의)
#[repr(C)]
#[derive(Clone, Copy)]
struct CHARFORMAT2W {
    cb_size: u32,
    dw_mask: u32,
    dw_effects: u32,
    y_height: i32,
    y_offset: i32,
    cr_text_color: u32,
    b_char_set: u8,
    b_pitch_and_family: u8,
    sz_face_name: [u16; 32],
    w_weight: u16,
    s_spacing: i16,
    cr_back_color: u32,
    lcid: u32,
    dw_reserved: u32,
    s_style: i16,
    w_kerning: u16,
    b_underline_type: u8,
    b_animation: u8,
    b_rev_author: u8,
    b_reserved1: u8,
}

impl Default for CHARFORMAT2W {
    fn default() -> Self {
        Self {
            cb_size: std::mem::size_of::<CHARFORMAT2W>() as u32,
            dw_mask: 0,
            dw_effects: 0,
            y_height: 0,
            y_offset: 0,
            cr_text_color: 0,
            b_char_set: 0,
            b_pitch_and_family: 0,
            sz_face_name: [0; 32],
            w_weight: 0,
            s_spacing: 0,
            cr_back_color: 0,
            lcid: 0,
            dw_reserved: 0,
            s_style: 0,
            w_kerning: 0,
            b_underline_type: 0,
            b_animation: 0,
            b_rev_author: 0,
            b_reserved1: 0,
        }
    }
}

/// 백로그 대화상자
pub struct BacklogDialog {
    hwnd: HWND,
    #[allow(dead_code)]
    main_hwnd: HWND,
    config: Rc<RefCell<Config>>,
    richedit: HWND,
    filter: BacklogFilter,
    add_linefeed: bool,
    entries: Vec<LogEntry>,
}

thread_local! {
    static BACKLOG_INSTANCE: RefCell<Option<Rc<RefCell<BacklogDialog>>>> = const { RefCell::new(None) };
    static RICHEDIT_LOADED: RefCell<bool> = const { RefCell::new(false) };
}

impl BacklogDialog {
    /// 백로그 대화상자 생성 및 표시
    pub fn show(main_hwnd: HWND, config: Rc<RefCell<Config>>) -> Result<HWND> {
        // RichEdit DLL 로드
        Self::ensure_richedit_loaded();
        unsafe { Self::show_impl(main_hwnd, config) }
    }

    /// RichEdit DLL 로드 확인
    fn ensure_richedit_loaded() {
        RICHEDIT_LOADED.with(|loaded| {
            if !*loaded.borrow() {
                unsafe {
                    let _ = LoadLibraryW(w!("Riched20.dll"));
                }
                *loaded.borrow_mut() = true;
            }
        });
    }

    unsafe fn show_impl(main_hwnd: HWND, config: Rc<RefCell<Config>>) -> Result<HWND> { unsafe {
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
            lpszClassName: BACKLOG_CLASS_NAME,
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
        let x = (cx - BACKLOG_WIDTH) / 2;
        let y = (cy - BACKLOG_HEIGHT) / 2;

        // 윈도우 생성
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            BACKLOG_CLASS_NAME,
            w!("백로그"),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_SIZEBOX,
            x,
            y,
            BACKLOG_WIDTH,
            BACKLOG_HEIGHT,
            Some(main_hwnd),
            None,
            Some(instance.into()),
            None,
        )?;

        // 인스턴스 생성
        let dialog = Rc::new(RefCell::new(BacklogDialog {
            hwnd,
            main_hwnd,
            config,
            richedit: HWND::default(),
            filter: BacklogFilter::All,
            add_linefeed: true,
            entries: Vec::new(),
        }));

        // 전역 인스턴스 설정
        BACKLOG_INSTANCE.with(|cell| {
            *cell.borrow_mut() = Some(dialog.clone());
        });

        // 컨트롤 생성
        dialog.borrow_mut().create_controls()?;

        // 윈도우 표시
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);

        Ok(hwnd)
    }}

    /// 컨트롤 생성
    fn create_controls(&mut self) -> Result<()> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let _hfont = GetStockObject(DEFAULT_GUI_FONT);

            // RichEdit 컨트롤 생성
            self.richedit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("RichEdit20W"),
                w!(""),
                WINDOW_STYLE(
                    WS_CHILD.0
                        | WS_VISIBLE.0
                        | WS_VSCROLL.0
                        | WS_HSCROLL.0
                        | ES_MULTILINE as u32
                        | ES_AUTOVSCROLL as u32
                        | ES_AUTOHSCROLL as u32
                        | ES_READONLY as u32,
                ),
                10,
                10,
                BACKLOG_WIDTH - 30,
                BACKLOG_HEIGHT - 120,
                Some(self.hwnd),
                Some(HMENU(ctrl_id::RICHEDIT as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;

            // 배경색 설정 (어두운 색상)
            let _ = SendMessageW(
                self.richedit,
                EM_SETBKGNDCOLOR,
                Some(WPARAM(0)),
                Some(LPARAM(0x282828)), // BGR
            );

            // ====== 옵션 그룹 ======
            self.create_group_box(10, BACKLOG_HEIGHT - 100, 350, 60, "필터")?;

            // 라디오 버튼
            self.create_radio(
                20,
                BACKLOG_HEIGHT - 80,
                80,
                20,
                ctrl_id::RADIO_ORIGINAL,
                "원문만",
                self.filter == BacklogFilter::Original,
            )?;
            self.create_radio(
                105,
                BACKLOG_HEIGHT - 80,
                80,
                20,
                ctrl_id::RADIO_TRANSLATION,
                "번역만",
                self.filter == BacklogFilter::Translation,
            )?;
            self.create_radio(
                190,
                BACKLOG_HEIGHT - 80,
                60,
                20,
                ctrl_id::RADIO_ALL,
                "전체",
                self.filter == BacklogFilter::All,
            )?;

            // 줄바꿈 체크박스
            self.create_checkbox(
                260,
                BACKLOG_HEIGHT - 80,
                90,
                20,
                ctrl_id::CHK_LINEFEED,
                "줄바꿈 추가",
                self.add_linefeed,
            )?;

            // ====== 버튼 그룹 ======
            self.create_group_box(370, BACKLOG_HEIGHT - 100, 200, 60, "동작")?;

            // 버튼들
            self.create_button(
                380,
                BACKLOG_HEIGHT - 78,
                55,
                28,
                ctrl_id::BTN_CLEAR,
                "초기화",
            )?;
            self.create_button(
                445,
                BACKLOG_HEIGHT - 78,
                55,
                28,
                ctrl_id::BTN_SAVE,
                "저장",
            )?;
            self.create_button(
                510,
                BACKLOG_HEIGHT - 78,
                55,
                28,
                ctrl_id::BTN_FONT,
                "폰트",
            )?;

            Ok(())
        }
    }

    // 헬퍼 함수들
    unsafe fn create_group_box(&self, x: i32, y: i32, w: i32, h: i32, text: &str) -> Result<HWND> { unsafe {
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
        let _ = SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));

        Ok(hwnd)
    }}

    unsafe fn create_button(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str) -> Result<HWND> { unsafe {
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
        let _ = SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));

        Ok(hwnd)
    }}

    unsafe fn create_checkbox(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: u16,
        text: &str,
        checked: bool,
    ) -> Result<HWND> { unsafe {
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
        let _ = SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));

        if checked {
            let _ = SendMessageW(hwnd, BM_SETCHECK, Some(WPARAM(BST_CHECKED.0 as usize)), Some(LPARAM(0)));
        }

        Ok(hwnd)
    }}

    unsafe fn create_radio(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: u16,
        text: &str,
        checked: bool,
    ) -> Result<HWND> { unsafe {
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
        let _ = SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));

        if checked {
            let _ = SendMessageW(hwnd, BM_SETCHECK, Some(WPARAM(BST_CHECKED.0 as usize)), Some(LPARAM(0)));
        }

        Ok(hwnd)
    }}

    /// 로그 항목 추가
    pub fn add_entry(&mut self, entry: LogEntry) {
        self.entries.push(entry.clone());
        self.append_entry_to_richedit(&entry);
    }

    /// RichEdit에 항목 추가
    fn append_entry_to_richedit(&self, entry: &LogEntry) {
        unsafe {
            // 끝으로 이동
            let _ = SendMessageW(self.richedit, EM_SETSEL, Some(WPARAM(usize::MAX)), Some(LPARAM(-1)));

            // 이름 출력 (노란색, 굵게)
            if let Some(ref name) = entry.name {
                if self.filter != BacklogFilter::Translation {
                    self.append_styled_text(&format!("[{}] ", name), 0x00FFFF, true); // 노란색 (BGR)
                }
            }

            // 원문 출력 (흰색)
            if self.filter != BacklogFilter::Translation {
                self.append_styled_text(&entry.original, 0xFFFFFF, false);
                if self.add_linefeed {
                    self.append_styled_text("\r\n", 0xFFFFFF, false);
                }
            }

            // 번역문 출력 (밝은 초록색)
            if let Some(ref translation) = entry.translation {
                if self.filter != BacklogFilter::Original {
                    self.append_styled_text(translation, 0x90EE90, false); // 밝은 초록 (BGR)
                    if self.add_linefeed {
                        self.append_styled_text("\r\n", 0x90EE90, false);
                    }
                }
            }

            // 항목 구분선
            if self.filter == BacklogFilter::All && self.add_linefeed {
                self.append_styled_text("\r\n", 0x808080, false);
            }

            // 자동 스크롤
            let _ = SendMessageW(self.richedit, EM_SCROLLCARET, Some(WPARAM(0)), Some(LPARAM(0)));
        }
    }

    /// 스타일 텍스트 추가
    unsafe fn append_styled_text(&self, text: &str, color: u32, bold: bool) { unsafe {
        // CHARFORMAT2W 설정
        let mut cf = CHARFORMAT2W::default();
        cf.dw_mask = CFM_COLOR | CFM_SIZE;
        cf.cr_text_color = color;
        cf.y_height = 200; // 10pt (1pt = 20 트윕)

        if bold {
            cf.dw_mask |= CFM_BOLD;
            cf.dw_effects |= CFE_BOLD;
        }

        // 포맷 적용
        let _ = SendMessageW(
            self.richedit,
            EM_SETCHARFORMAT,
            Some(WPARAM(SCF_SELECTION as usize)),
            Some(LPARAM(&cf as *const _ as isize)),
        );

        // 텍스트 삽입
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let _ = SendMessageW(
            self.richedit,
            EM_REPLACESEL,
            Some(WPARAM(0)),
            Some(LPARAM(wide.as_ptr() as isize)),
        );
    }}

    /// RichEdit 내용 지우기
    fn clear_richedit(&mut self) {
        self.entries.clear();
        unsafe {
            let _ = SetWindowTextW(self.richedit, w!(""));
        }
    }

    /// RichEdit 다시 그리기 (필터 변경 시)
    fn refresh_richedit(&self) {
        unsafe {
            let _ = SetWindowTextW(self.richedit, w!(""));
        }
        for entry in &self.entries.clone() {
            self.append_entry_to_richedit(entry);
        }
    }

    /// 파일로 저장
    fn save_to_file(&self) {
        unsafe {
            use std::io::Write;

            // 파일 저장 대화상자
            let mut filename: [u16; 260] = [0; 260];
            let filter: Vec<u16> = "텍스트 파일 (*.txt)\0*.txt\0모든 파일 (*.*)\0*.*\0\0"
                .encode_utf16()
                .collect();

            let mut ofn = OPENFILENAMEW {
                lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
                hwndOwner: self.hwnd,
                lpstrFilter: PCWSTR(filter.as_ptr()),
                lpstrFile: PWSTR(filename.as_mut_ptr()),
                nMaxFile: 260,
                Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST,
                lpstrDefExt: w!("txt"),
                ..Default::default()
            };

            if !GetSaveFileNameW(&mut ofn).as_bool() {
                return;
            }

            // 파일명 가져오기
            let len = filename.iter().position(|&c| c == 0).unwrap_or(filename.len());
            let path = String::from_utf16_lossy(&filename[..len]);

            // 내용 생성
            let mut content = String::new();
            for entry in &self.entries {
                if let Some(ref name) = entry.name {
                    content.push_str(&format!("[{}] ", name));
                }
                content.push_str(&entry.original);
                content.push_str("\r\n");
                if let Some(ref trans) = entry.translation {
                    content.push_str(trans);
                    content.push_str("\r\n");
                }
                content.push_str("\r\n");
            }

            // 파일 쓰기
            if let Ok(mut file) = std::fs::File::create(&path) {
                // UTF-8 BOM 추가
                let _ = file.write_all(&[0xEF, 0xBB, 0xBF]);
                let _ = file.write_all(content.as_bytes());
            }
        }
    }

    /// 명령 처리
    fn handle_command(&mut self, cmd: u16) {
        use ctrl_id::*;

        match cmd {
            CHK_LINEFEED => {
                self.add_linefeed = !self.add_linefeed;
                self.refresh_richedit();
            }
            RADIO_ORIGINAL => {
                self.filter = BacklogFilter::Original;
                self.refresh_richedit();
            }
            RADIO_TRANSLATION => {
                self.filter = BacklogFilter::Translation;
                self.refresh_richedit();
            }
            RADIO_ALL => {
                self.filter = BacklogFilter::All;
                self.refresh_richedit();
            }
            BTN_CLEAR => {
                self.clear_richedit();
            }
            BTN_SAVE => {
                self.save_to_file();
            }
            BTN_FONT => {
                // TODO: 폰트 선택 대화상자
            }
            _ => {}
        }
    }

    /// 윈도우 크기 변경 시 컨트롤 재배치
    fn on_size(&self, width: i32, height: i32) {
        unsafe {
            // RichEdit 크기 조정
            let _ = SetWindowPos(
                self.richedit,
                None,
                10,
                10,
                width - 30,
                height - 120,
                SWP_NOZORDER,
            );
        }
    }

    /// WndProc
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT { unsafe {
        let instance = BACKLOG_INSTANCE.with(|cell| cell.borrow().clone());

        if let Some(dialog) = instance {
            match msg {
                WM_COMMAND => {
                    let id = (wparam.0 & 0xFFFF) as u16;
                    dialog.borrow_mut().handle_command(id);
                    return LRESULT(0);
                }

                WM_SIZE => {
                    let width = (lparam.0 & 0xFFFF) as i32;
                    let height = ((lparam.0 >> 16) & 0xFFFF) as i32;
                    dialog.borrow().on_size(width, height);
                    return LRESULT(0);
                }

                WM_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }

                WM_DESTROY => {
                    BACKLOG_INSTANCE.with(|cell| {
                        *cell.borrow_mut() = None;
                    });
                    return LRESULT(0);
                }

                WM_LBUTTONDOWN => {
                    // 창 드래그
                    let _ = SendMessageW(hwnd, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(LPARAM(0)));
                    return LRESULT(0);
                }

                _ => {}
            }
        }

        DefWindowProcW(hwnd, msg, wparam, lparam)
    }}
}

/// 공개 인터페이스: 기존 백로그에 항목 추가
pub fn add_to_backlog(entry: LogEntry) {
    BACKLOG_INSTANCE.with(|cell| {
        if let Some(ref dialog) = *cell.borrow() {
            dialog.borrow_mut().add_entry(entry);
        }
    });
}
