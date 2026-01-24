//! 파일 번역 대화상자
//!
//! 다중 파일 선택 및 배치 번역 기능.
//! GetOpenFileNameW + OFN_ALLOWMULTISELECT 사용.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::Controls::BST_CHECKED,
        UI::Controls::Dialogs::{
            GetOpenFileNameW, GetSaveFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER,
            OFN_FILEMUSTEXIST, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
        },
        UI::Input::KeyboardAndMouse::EnableWindow,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::config::Config;

use super::file_trans_progress::FileTransProgressDialog;

// 컨트롤 ID
mod ctrl_id {
    pub const LOAD_EDIT: u16 = 4001;
    pub const SAVE_EDIT: u16 = 4002;
    pub const LOAD_BROWSER: u16 = 4003;
    pub const SAVE_BROWSER: u16 = 4004;
    pub const PREVIEW_EDIT: u16 = 4005;
    pub const OUTPUT_1: u16 = 4010; // 번역만
    pub const OUTPUT_2: u16 = 4011; // 원문+번역
    pub const OUTPUT_3: u16 = 4012; // 원문+번역+개행
    pub const NO_TRANS_LINEFEED: u16 = 4020;
    pub const BTN_TRANSLATE: u16 = 4030;
    pub const BTN_CLOSE: u16 = 4031;
}

const FILE_TRANS_CLASS_NAME: PCWSTR = w!("AnemoneFileTransClass");
const FILE_TRANS_WIDTH: i32 = 550;
const FILE_TRANS_HEIGHT: i32 = 420;

/// 출력 형식
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum WriteType {
    #[default]
    TranslationOnly = 0, // 번역만
    OriginalAndTrans = 1,     // 원문 + 번역
    OriginalTransNewline = 2, // 원문 + 번역 + 개행
}

/// 파일 번역 작업 데이터 (스레드 안전)
pub struct FileTransJobData {
    pub input_files: Vec<PathBuf>,
    pub output_files: Vec<PathBuf>,
    pub write_type: WriteType,
    pub no_trans_linefeed: bool,
    pub progress_hwnd: isize, // HWND를 isize로 저장 (Send 가능)
    pub cancel_token: Arc<AtomicBool>,
}

// FileTransJobData는 Send 안전 (HWND를 isize로 저장)
unsafe impl Send for FileTransJobData {}
unsafe impl Sync for FileTransJobData {}

/// 파일 번역 대화상자
pub struct FileTransDialog {
    hwnd: HWND,
    #[allow(dead_code)]
    main_hwnd: HWND,
    #[allow(dead_code)]
    config: Rc<RefCell<Config>>,
    load_edit: HWND,
    save_edit: HWND,
    save_browser_btn: HWND,
    preview_edit: HWND,
    input_files: Vec<PathBuf>,
    output_files: Vec<PathBuf>,
    write_type: WriteType,
    no_trans_linefeed: bool,
    cancel_token: Arc<AtomicBool>,
}

thread_local! {
    static FILE_TRANS_INSTANCE: RefCell<Option<Rc<RefCell<FileTransDialog>>>> = const { RefCell::new(None) };
}

impl FileTransDialog {
    /// 파일 번역 대화상자 생성 및 표시
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
                lpszClassName: FILE_TRANS_CLASS_NAME,
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
            let x = (cx - FILE_TRANS_WIDTH) / 2;
            let y = (cy - FILE_TRANS_HEIGHT) / 2;

            // 윈도우 생성
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                FILE_TRANS_CLASS_NAME,
                w!("파일 번역"),
                WS_POPUP | WS_CAPTION | WS_SYSMENU,
                x,
                y,
                FILE_TRANS_WIDTH,
                FILE_TRANS_HEIGHT,
                Some(main_hwnd),
                None,
                Some(instance.into()),
                None,
            )?;

            // 인스턴스 생성
            let dialog = Rc::new(RefCell::new(FileTransDialog {
                hwnd,
                main_hwnd,
                config,
                load_edit: HWND::default(),
                save_edit: HWND::default(),
                save_browser_btn: HWND::default(),
                preview_edit: HWND::default(),
                input_files: Vec::new(),
                output_files: Vec::new(),
                write_type: WriteType::TranslationOnly,
                no_trans_linefeed: false,
                cancel_token: Arc::new(AtomicBool::new(false)),
            }));

            // 전역 인스턴스 설정
            FILE_TRANS_INSTANCE.with(|cell| {
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

            // ====== 입력 파일 그룹 ======
            self.create_group_box(10, 5, 525, 70, "입력 파일")?;

            self.create_label(20, 30, 50, 18, "파일:")?;

            // 입력 파일 Edit
            self.load_edit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("EDIT"),
                w!(""),
                WINDOW_STYLE(
                    WS_CHILD.0 | WS_VISIBLE.0 | ES_AUTOHSCROLL as u32 | ES_READONLY as u32,
                ),
                75,
                27,
                370,
                24,
                Some(self.hwnd),
                Some(HMENU(ctrl_id::LOAD_EDIT as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;
            let _ = SendMessageW(
                self.load_edit,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            // 찾기 버튼
            self.create_button(455, 26, 70, 26, ctrl_id::LOAD_BROWSER, "찾아보기...")?;

            // ====== 출력 파일 그룹 ======
            self.create_group_box(10, 80, 525, 70, "출력 파일")?;

            self.create_label(20, 105, 50, 18, "파일:")?;

            // 출력 파일 Edit
            self.save_edit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("EDIT"),
                w!(""),
                WINDOW_STYLE(
                    WS_CHILD.0 | WS_VISIBLE.0 | ES_AUTOHSCROLL as u32 | ES_READONLY as u32,
                ),
                75,
                102,
                370,
                24,
                Some(self.hwnd),
                Some(HMENU(ctrl_id::SAVE_EDIT as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;
            let _ = SendMessageW(
                self.save_edit,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            // 저장 위치 버튼 (단일 파일 선택 시만 활성화)
            self.save_browser_btn =
                self.create_button(455, 101, 70, 26, ctrl_id::SAVE_BROWSER, "변경...")?;
            let _ = EnableWindow(self.save_browser_btn, false);

            // ====== 미리보기 그룹 ======
            self.create_group_box(10, 155, 525, 130, "미리보기 (처음 7줄)")?;

            // 미리보기 Edit (readonly, multiline)
            self.preview_edit = CreateWindowExW(
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
                175,
                505,
                100,
                Some(self.hwnd),
                Some(HMENU(ctrl_id::PREVIEW_EDIT as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;
            let _ = SendMessageW(
                self.preview_edit,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            // ====== 출력 형식 그룹 ======
            self.create_group_box(10, 290, 350, 55, "출력 형식")?;

            // 라디오 버튼
            self.create_radio(
                20,
                310,
                80,
                20,
                ctrl_id::OUTPUT_1,
                "번역만",
                self.write_type == WriteType::TranslationOnly,
            )?;
            self.create_radio(
                105,
                310,
                90,
                20,
                ctrl_id::OUTPUT_2,
                "원문+번역",
                self.write_type == WriteType::OriginalAndTrans,
            )?;
            self.create_radio(
                200,
                310,
                120,
                20,
                ctrl_id::OUTPUT_3,
                "원문+번역+개행",
                self.write_type == WriteType::OriginalTransNewline,
            )?;

            // 줄바꿈 제외 체크박스
            self.create_checkbox(
                20,
                332,
                180,
                20,
                ctrl_id::NO_TRANS_LINEFEED,
                "줄바꿈만 있는 라인 번역 안함",
                self.no_trans_linefeed,
            )?;

            // ====== 버튼 그룹 ======
            self.create_group_box(370, 290, 165, 55, "동작")?;

            // 버튼들
            self.create_button(385, 310, 65, 28, ctrl_id::BTN_TRANSLATE, "번역 시작")?;
            self.create_button(460, 310, 60, 28, ctrl_id::BTN_CLOSE, "닫기")?;

            Ok(())
        }
    }

    // 헬퍼 함수들
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

    /// Edit 컨트롤에 텍스트 설정
    unsafe fn set_edit_text(hwnd: HWND, text: &str) {
        unsafe {
            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
        }
    }

    /// 다중 선택 파일 경로 파싱
    fn parse_multi_select_paths(buffer: &[u16]) -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // null 종료자 찾기
        let first_null = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        if first_null == 0 {
            return paths;
        }

        let directory = String::from_utf16_lossy(&buffer[..first_null]);

        // 두 번째 문자열 시작 위치
        let second_start = first_null + 1;
        if second_start >= buffer.len() || buffer[second_start] == 0 {
            // 단일 파일 선택
            paths.push(PathBuf::from(&directory));
            return paths;
        }

        // 다중 파일 선택
        let dir = PathBuf::from(&directory);
        let mut pos = second_start;

        while pos < buffer.len() && buffer[pos] != 0 {
            let end = buffer[pos..]
                .iter()
                .position(|&c| c == 0)
                .map(|i| pos + i)
                .unwrap_or(buffer.len());

            let filename = String::from_utf16_lossy(&buffer[pos..end]);
            paths.push(dir.join(&filename));

            pos = end + 1;
            if pos >= buffer.len() || buffer[pos] == 0 {
                break;
            }
        }

        paths
    }

    /// 입력 파일 선택 (다중 선택)
    fn browse_input_files(&mut self) {
        unsafe {
            let mut file_buffer: Vec<u16> = vec![0; 32767];

            let filter: Vec<u16> = "텍스트 파일 (*.txt)\0*.txt\0모든 파일 (*.*)\0*.*\0\0"
                .encode_utf16()
                .collect();

            let mut ofn = OPENFILENAMEW {
                lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
                hwndOwner: self.hwnd,
                lpstrFilter: PCWSTR(filter.as_ptr()),
                lpstrFile: PWSTR(file_buffer.as_mut_ptr()),
                nMaxFile: file_buffer.len() as u32,
                Flags: OFN_ALLOWMULTISELECT | OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_FILEMUSTEXIST,
                ..Default::default()
            };

            if GetOpenFileNameW(&mut ofn).as_bool() {
                self.input_files = Self::parse_multi_select_paths(&file_buffer);
                self.output_files.clear();

                // 출력 파일명 생성 (입력파일명_번역.txt)
                for input in &self.input_files {
                    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
                    let parent = input.parent().unwrap_or(std::path::Path::new(""));
                    let output = parent.join(format!("{}_번역.txt", stem));
                    self.output_files.push(output);
                }

                // UI 표시
                let input_display: Vec<String> = self
                    .input_files
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                let output_display: Vec<String> = self
                    .output_files
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();

                Self::set_edit_text(self.load_edit, &input_display.join(", "));
                Self::set_edit_text(self.save_edit, &output_display.join(", "));

                // 단일 파일 선택 시만 저장 위치 변경 버튼 활성화
                let _ = EnableWindow(self.save_browser_btn, self.input_files.len() == 1);

                // 첫 파일 미리보기
                if let Some(first_file) = self.input_files.first() {
                    self.show_preview(first_file);
                }
            }
        }
    }

    /// 출력 파일 위치 변경 (단일 파일만)
    fn browse_output_file(&mut self) {
        if self.input_files.len() != 1 {
            return;
        }

        unsafe {
            let mut file_buffer: [u16; 260] = [0; 260];

            // 현재 출력 경로 복사
            if let Some(current) = self.output_files.first() {
                let current_wide: Vec<u16> = current
                    .to_string_lossy()
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                let copy_len = current_wide.len().min(file_buffer.len() - 1);
                file_buffer[..copy_len].copy_from_slice(&current_wide[..copy_len]);
            }

            let filter: Vec<u16> = "텍스트 파일 (*.txt)\0*.txt\0모든 파일 (*.*)\0*.*\0\0"
                .encode_utf16()
                .collect();

            let mut ofn = OPENFILENAMEW {
                lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
                hwndOwner: self.hwnd,
                lpstrFilter: PCWSTR(filter.as_ptr()),
                lpstrFile: PWSTR(file_buffer.as_mut_ptr()),
                nMaxFile: 260,
                Flags: OFN_PATHMUSTEXIST | OFN_OVERWRITEPROMPT,
                lpstrDefExt: w!("txt"),
                ..Default::default()
            };

            if GetSaveFileNameW(&mut ofn).as_bool() {
                let len = file_buffer
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(file_buffer.len());
                let path = String::from_utf16_lossy(&file_buffer[..len]);
                self.output_files[0] = PathBuf::from(&path);
                Self::set_edit_text(self.save_edit, &path);
            }
        }
    }

    /// 파일 미리보기 (처음 7줄)
    fn show_preview(&self, path: &PathBuf) {
        use std::fs::File;
        use std::io::{BufRead, BufReader};

        let content = match File::open(path) {
            Ok(file) => {
                let reader = BufReader::new(file);
                let lines: Vec<String> = reader.lines().take(7).filter_map(|l| l.ok()).collect();
                lines.join("\r\n")
            }
            Err(_) => "! 파일을 열 수 없습니다.".to_string(),
        };

        unsafe {
            Self::set_edit_text(self.preview_edit, &content);
        }
    }

    /// 번역 시작
    fn start_translation(&mut self) {
        if self.input_files.is_empty() {
            unsafe {
                let _ = MessageBoxW(
                    Some(self.hwnd),
                    w!("파일을 먼저 선택해주세요."),
                    w!("알림"),
                    MB_ICONINFORMATION,
                );
            }
            return;
        }

        // 취소 토큰 초기화
        self.cancel_token.store(false, Ordering::SeqCst);

        // 진행률 대화상자 생성
        let progress_hwnd =
            match FileTransProgressDialog::show(self.hwnd, self.cancel_token.clone()) {
                Ok(hwnd) => hwnd,
                Err(e) => {
                    eprintln!("Failed to create progress dialog: {:?}", e);
                    return;
                }
            };

        // 작업 데이터 생성 (HWND를 isize로 변환)
        let job_data = Arc::new(FileTransJobData {
            input_files: self.input_files.clone(),
            output_files: self.output_files.clone(),
            write_type: self.write_type,
            no_trans_linefeed: self.no_trans_linefeed,
            progress_hwnd: progress_hwnd.0 as isize,
            cancel_token: self.cancel_token.clone(),
        });

        // 백그라운드 스레드 시작
        std::thread::spawn(move || {
            super::file_trans_thread::file_trans_thread(job_data);
        });
    }

    /// 명령 처리
    fn handle_command(&mut self, cmd: u16) {
        use ctrl_id::*;

        match cmd {
            LOAD_BROWSER => {
                self.browse_input_files();
            }
            SAVE_BROWSER => {
                self.browse_output_file();
            }
            OUTPUT_1 => {
                self.write_type = WriteType::TranslationOnly;
            }
            OUTPUT_2 => {
                self.write_type = WriteType::OriginalAndTrans;
            }
            OUTPUT_3 => {
                self.write_type = WriteType::OriginalTransNewline;
            }
            NO_TRANS_LINEFEED => {
                self.no_trans_linefeed = !self.no_trans_linefeed;
            }
            BTN_TRANSLATE => {
                self.start_translation();
            }
            BTN_CLOSE => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
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
            let instance = FILE_TRANS_INSTANCE.with(|cell| cell.borrow().clone());

            if let Some(dialog) = instance {
                match msg {
                    WM_COMMAND => {
                        let id = (wparam.0 & 0xFFFF) as u16;
                        dialog.borrow_mut().handle_command(id);
                        return LRESULT(0);
                    }

                    WM_CLOSE => {
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }

                    WM_DESTROY => {
                        FILE_TRANS_INSTANCE.with(|cell| {
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
