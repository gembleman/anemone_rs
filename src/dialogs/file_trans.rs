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
use crate::impl_dialog;
use crate::util::to_wide;
use super::file_trans_progress::FileTransProgressDialog;
use super::helpers::DialogControls;

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

// SAFETY: FileTransJobData is Send/Sync safe because the HWND is stored as a plain isize
// (not as HWND which is !Send). The isize is only reconstructed to HWND on the target
// thread for PostMessageW, which is safe to call cross-thread. All other fields (Vec,
// WriteType, bool, Arc<AtomicBool>) are inherently Send+Sync.
unsafe impl Send for FileTransJobData {}
unsafe impl Sync for FileTransJobData {}

/// 파일 번역 대화상자
pub struct FileTransDialog {
    hwnd: HWND,
    main_hwnd: HWND,
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

impl DialogControls for FileTransDialog {
    fn dialog_hwnd(&self) -> HWND { self.hwnd }
}

impl_dialog! {
    dialog: FileTransDialog,
    instance: FILE_TRANS_INSTANCE,
    class_name: w!("AnemoneFileTransClass"),
    title: w!("파일 번역"),
    width: 550,
    height: 420,
    extra_style: WINDOW_STYLE::default(),
    params: (parent: HWND, config: Rc<RefCell<Config>>),
    init: |hwnd, parent, config| {
        FileTransDialog {
            hwnd,
            main_hwnd: parent,
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
        }
    },
}

impl FileTransDialog {
    /// 컨트롤 생성
    fn create_controls(&mut self) -> Result<()> {
        // SAFETY: self.hwnd is a valid dialog window handle. GetModuleHandleW(None) returns
        // the current module. All CreateWindowExW calls use valid parent handle, module
        // instance, and control IDs cast to HMENU. SendMessageW WM_SETFONT uses a valid
        // stock font object.
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
                75, 27, 370, 24,
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
                75, 102, 370, 24,
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

            self.save_browser_btn =
                self.create_button(455, 101, 70, 26, ctrl_id::SAVE_BROWSER, "변경...")?;
            let _ = EnableWindow(self.save_browser_btn, false);

            // ====== 미리보기 그룹 ======
            self.create_group_box(10, 155, 525, 130, "미리보기 (처음 7줄)")?;

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
                20, 175, 505, 100,
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

            self.create_radio(20, 310, 80, 20, ctrl_id::OUTPUT_1, "번역만",
                self.write_type == WriteType::TranslationOnly)?;
            self.create_radio(105, 310, 90, 20, ctrl_id::OUTPUT_2, "원문+번역",
                self.write_type == WriteType::OriginalAndTrans)?;
            self.create_radio(200, 310, 120, 20, ctrl_id::OUTPUT_3, "원문+번역+개행",
                self.write_type == WriteType::OriginalTransNewline)?;

            self.create_checkbox(20, 332, 180, 20, ctrl_id::NO_TRANS_LINEFEED,
                "줄바꿈만 있는 라인 번역 안함", self.no_trans_linefeed)?;

            // ====== 버튼 그룹 ======
            self.create_group_box(370, 290, 165, 55, "동작")?;
            self.create_button(385, 310, 65, 28, ctrl_id::BTN_TRANSLATE, "번역 시작")?;
            self.create_button(460, 310, 60, 28, ctrl_id::BTN_CLOSE, "닫기")?;

            Ok(())
        }
    }

    /// 커스텀 메시지 핸들러 (없음)
    fn handle_message(&mut self, _msg: u32, _wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
        None
    }

    /// Edit 컨트롤에 텍스트 설정
    unsafe fn set_edit_text(hwnd: HWND, text: &str) {
        // SAFETY: hwnd is a valid edit control handle. The wide string pointer is valid
        // for the duration of the SetWindowTextW call.
        unsafe {
            let wide = to_wide(text);
            let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
        }
    }

    /// 다중 선택 파일 경로 파싱
    fn parse_multi_select_paths(buffer: &[u16]) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let first_null = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        if first_null == 0 { return paths; }

        let directory = String::from_utf16_lossy(&buffer[..first_null]);
        let second_start = first_null + 1;
        if second_start >= buffer.len() || buffer[second_start] == 0 {
            paths.push(PathBuf::from(&directory));
            return paths;
        }

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
            if pos >= buffer.len() || buffer[pos] == 0 { break; }
        }
        paths
    }

    /// 입력 파일 선택 (다중 선택)
    fn browse_input_files(&mut self) {
        // SAFETY: self.hwnd is a valid dialog window handle used as owner. OPENFILENAMEW is
        // initialized with correct lStructSize, valid filter/file buffer pointers with
        // sufficient sizes. The file buffer lives for the duration of GetOpenFileNameW.
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

                for input in &self.input_files {
                    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
                    let parent = input.parent().unwrap_or(std::path::Path::new(""));
                    let output = parent.join(format!("{}_번역.txt", stem));
                    self.output_files.push(output);
                }

                let input_display: Vec<String> = self.input_files.iter()
                    .map(|p| p.to_string_lossy().to_string()).collect();
                let output_display: Vec<String> = self.output_files.iter()
                    .map(|p| p.to_string_lossy().to_string()).collect();

                Self::set_edit_text(self.load_edit, &input_display.join(", "));
                Self::set_edit_text(self.save_edit, &output_display.join(", "));

                let _ = EnableWindow(self.save_browser_btn, self.input_files.len() == 1);

                if let Some(first_file) = self.input_files.first() {
                    self.show_preview(first_file);
                }
            }
        }
    }

    /// 출력 파일 위치 변경 (단일 파일만)
    fn browse_output_file(&mut self) {
        if self.input_files.len() != 1 { return; }

        // SAFETY: self.hwnd is a valid dialog window handle used as owner. OPENFILENAMEW is
        // initialized with correct lStructSize, valid filter/file buffer pointers. The
        // file buffer is stack-allocated with sufficient size for a single file path.
        unsafe {
            let mut file_buffer: [u16; 260] = [0; 260];
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
                let len = file_buffer.iter().position(|&c| c == 0).unwrap_or(file_buffer.len());
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

        // SAFETY: self.preview_edit is a valid edit control handle created in create_controls.
        unsafe { Self::set_edit_text(self.preview_edit, &content); }
    }

    /// 번역 시작
    fn start_translation(&mut self) {
        if self.input_files.is_empty() {
            // SAFETY: self.hwnd is a valid dialog window handle used as the message box owner.
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

        self.cancel_token.store(false, Ordering::SeqCst);

        let progress_hwnd =
            match FileTransProgressDialog::show(self.hwnd, self.cancel_token.clone()) {
                Ok(hwnd) => hwnd,
                Err(e) => {
                    tracing::error!("Failed to create progress dialog: {:?}", e);
                    return;
                }
            };

        let job_data = Arc::new(FileTransJobData {
            input_files: self.input_files.clone(),
            output_files: self.output_files.clone(),
            write_type: self.write_type,
            no_trans_linefeed: self.no_trans_linefeed,
            progress_hwnd: progress_hwnd.0 as isize,
            cancel_token: self.cancel_token.clone(),
        });

        std::thread::spawn(move || {
            super::file_trans_thread::file_trans_thread(job_data);
        });
    }

    /// 명령 처리
    fn handle_command(&mut self, cmd: u16, _notify_code: u32) {
        use ctrl_id::*;

        match cmd {
            LOAD_BROWSER => self.browse_input_files(),
            SAVE_BROWSER => self.browse_output_file(),
            OUTPUT_1 => self.write_type = WriteType::TranslationOnly,
            OUTPUT_2 => self.write_type = WriteType::OriginalAndTrans,
            OUTPUT_3 => self.write_type = WriteType::OriginalTransNewline,
            NO_TRANS_LINEFEED => self.no_trans_linefeed = !self.no_trans_linefeed,
            BTN_TRANSLATE => self.start_translation(),
            // SAFETY: self.hwnd is a valid dialog window handle.
            BTN_CLOSE => unsafe { let _ = DestroyWindow(self.hwnd); },
            _ => {}
        }
    }
}
