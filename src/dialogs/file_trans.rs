//! 파일 번역 대화상자
//!
//! 다중 파일 선택 및 배치 번역 기능.
//! Common Item Dialog (IFileOpenDialog / IFileSaveDialog) 사용.

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
        UI::Input::KeyboardAndMouse::EnableWindow,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::config::Config;
use crate::impl_dialog;
use crate::translation::{EngineCredentials, Language, TranslationEngine};
use crate::util::to_wide;
use super::file_dialog::{FileFilter, open_files_multi, save_file};
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
    pub const ENGINE_LABEL: u16 = 4040;
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
    pub engine: TranslationEngine,
    pub source_lang: Language,
    pub target_lang: Language,
    pub credentials: EngineCredentials,
    /// EzTrans 사용 시 필요한 DLL/DAT 경로. 다른 엔진에서는 빈 문자열이어도 무방.
    pub eztrans_dll_path: String,
    pub eztrans_dat_path: String,
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
    config: Rc<RefCell<Config>>,
    load_edit: HWND,
    save_edit: HWND,
    save_browser_btn: HWND,
    preview_edit: HWND,
    engine_label: HWND,
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
    height: 450,
    extra_style: WINDOW_STYLE::default(),
    params: (_parent: HWND, config: Rc<RefCell<Config>>),
    init: |hwnd, _parent, config| {
        FileTransDialog {
            hwnd,
            config,
            load_edit: HWND::default(),
            save_edit: HWND::default(),
            save_browser_btn: HWND::default(),
            preview_edit: HWND::default(),
            engine_label: HWND::default(),
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
            let dpi = crate::dpi::dpi_for_window(self.hwnd);
            let s = |v: i32| crate::dpi::scale(v, dpi);

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
                s(75), s(27), s(370), s(24),
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
                s(75), s(102), s(370), s(24),
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
                s(20), s(175), s(505), s(100),
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

            // ====== 엔진 안내 라벨 (다이얼로그 하단) ======
            // 파일 번역은 별도의 엔진 선택 UI 를 두지 않고 전역 설정 (번역 다이얼로그/
            // 설정 다이얼로그) 에서 선택된 엔진을 그대로 사용한다. 이용자가 현재
            // 어떤 엔진/언어쌍으로 동작할지 헷갈리지 않도록 표시만 해 준다.
            self.engine_label = self.create_label_with_id(
                20, 355, 510, 36, ctrl_id::ENGINE_LABEL, "",
            )?;
            self.update_engine_label();

            Ok(())
        }
    }

    /// 엔진 안내 라벨 텍스트 갱신
    fn update_engine_label(&self) {
        use crate::translation::lang_utils::to_korean_name;
        let (engine_name, source, target) = {
            let config = self.config.borrow();
            let engine = config.translation.get_engine();
            let engine_name = match engine {
                TranslationEngine::EzTrans => "EzTrans",
                TranslationEngine::Google => "Google",
                TranslationEngine::DeepL => "DeepL",
                TranslationEngine::Papago => "Papago",
                TranslationEngine::Llm => "LLM",
            };
            let source = to_korean_name(config.translation.get_source_language());
            let target = to_korean_name(config.translation.get_target_language());
            (engine_name, source, target)
        };
        let text = format!(
            "현재 번역 엔진: {} ({} → {})\r\n엔진/언어는 \"번역\" 또는 \"설정\" 다이얼로그에서 변경할 수 있습니다.",
            engine_name, source, target,
        );
        // SAFETY: engine_label is a valid static label control handle from create_controls.
        unsafe { Self::set_edit_text(self.engine_label, &text); }
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

    /// 입력 파일 선택 (다중 선택)
    fn browse_input_files(&mut self) {
        let filters = [
            FileFilter { name: "텍스트 파일 (*.txt)", spec: "*.txt" },
            FileFilter { name: "모든 파일 (*.*)", spec: "*.*" },
        ];
        let picked = open_files_multi(self.hwnd, "입력 파일 선택", &filters);
        if picked.is_empty() {
            return;
        }

        self.input_files = picked;
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

        // SAFETY: load_edit/save_edit/save_browser_btn are valid control handles
        // from create_controls.
        unsafe {
            Self::set_edit_text(self.load_edit, &input_display.join(", "));
            Self::set_edit_text(self.save_edit, &output_display.join(", "));
            let _ = EnableWindow(self.save_browser_btn, self.input_files.len() == 1);
        }

        if let Some(first_file) = self.input_files.first() {
            self.show_preview(first_file);
        }
    }

    /// 출력 파일 위치 변경 (단일 파일만)
    fn browse_output_file(&mut self) {
        if self.input_files.len() != 1 { return; }

        let filters = [
            FileFilter { name: "텍스트 파일 (*.txt)", spec: "*.txt" },
            FileFilter { name: "모든 파일 (*.*)", spec: "*.*" },
        ];
        let initial = self.output_files.first().map(|p| p.as_path());
        let Some(path) = save_file(self.hwnd, "출력 파일 위치", &filters, Some("txt"), initial)
        else {
            return;
        };

        let path_str = path.to_string_lossy().to_string();
        self.output_files[0] = path;
        // SAFETY: save_edit is a valid control handle from create_controls.
        unsafe { Self::set_edit_text(self.save_edit, &path_str); }
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
        // 다이얼로그가 떠 있는 동안 다른 창에서 설정이 바뀌었을 수 있으므로
        // 번역 시작 직전에 안내 라벨을 한 번 갱신해 최신 상태를 보여준다.
        self.update_engine_label();

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

        // 현재 config 의 엔진 설정을 그대로 사용해 자격증명을 빌드.
        // 번역 다이얼로그(translate.rs)와 동일한 패턴.
        let (engine, source_lang, target_lang, credentials, dll, dat) = {
            let config = self.config.borrow();
            let engine = config.translation.get_engine();
            let source_lang = config.translation.get_source_language();
            let target_lang = config.translation.get_target_language();
            let credentials = match engine {
                TranslationEngine::DeepL => EngineCredentials::DeepL {
                    keys: config.translation.deepl_effective_keys(),
                    strategy: config.translation.deepl_strategy(),
                },
                TranslationEngine::Papago => EngineCredentials::Papago {
                    client_id: config.translation.papago_client_id.clone(),
                    client_secret: config.translation.papago_client_secret.clone(),
                },
                TranslationEngine::Llm => {
                    EngineCredentials::Llm(config.translation.llm.to_call_params())
                }
                _ => EngineCredentials::None,
            };
            (
                engine,
                source_lang,
                target_lang,
                credentials,
                config.translation.eztrans_dll_path.clone(),
                config.translation.eztrans_dat_path.clone(),
            )
        };

        // EzTrans 선택 시 경로 미설정이면 즉시 안내하고 중단.
        if engine == TranslationEngine::EzTrans && (dll.is_empty() || dat.is_empty()) {
            // SAFETY: self.hwnd is a valid dialog window handle used as the message box owner.
            unsafe {
                let _ = MessageBoxW(
                    Some(self.hwnd),
                    w!("EzTrans 경로가 설정되지 않았습니다. 번역 설정에서 DLL/DAT 경로를 지정하세요."),
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
            engine,
            source_lang,
            target_lang,
            credentials,
            eztrans_dll_path: dll,
            eztrans_dat_path: dat,
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
