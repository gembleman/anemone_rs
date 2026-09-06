//! 파일 번역 대화상자
//!
//! 다중 파일 선택 및 배치 번역 기능.
//! Common Item Dialog (IFileOpenDialog / IFileSaveDialog) 사용.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use windows_core::{Error, HRESULT};
use windows_sys::Win32::{
    Foundation::*,
    UI::Controls::{BST_CHECKED, CheckDlgButton},
    UI::Input::KeyboardAndMouse::EnableWindow,
    UI::WindowsAndMessaging::*,
};

use super::file_dialog::{FileFilter, open_files_multi, save_file};
use super::file_trans_progress::FileTransProgressDialog;
use super::helpers::set_window_text;
use super::host::{DialogHost, DialogResult, HostedDialog};
use crate::app::action::AppActionSender;
use crate::config::Config;
use crate::file_trans::{
    FileTranslationRequest, FileTranslationSupervisor, WriteType, default_output_paths,
    validate_job_paths,
};
type Result<T> = windows_core::Result<T>;
use crate::translation::{PreparedJob, TranslationEngine};

// 컨트롤 ID
mod ctrl_id {
    pub const DIALOG: u16 = 104;
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

/// 파일 번역 대화상자
pub struct FileTransDialog {
    hwnd: HWND,
    config: Config,
    supervisor: Rc<FileTranslationSupervisor>,
    applied_dpi: u32,
    load_edit: HWND,
    save_edit: HWND,
    save_browser_btn: HWND,
    preview_edit: HWND,
    engine_label: HWND,
    input_files: Vec<PathBuf>,
    output_files: Vec<PathBuf>,
    write_type: WriteType,
    no_trans_linefeed: bool,
    actions: AppActionSender,
    session: u64,
}

pub(crate) struct FileTransInit {
    config: Config,
    supervisor: Rc<FileTranslationSupervisor>,
    actions: AppActionSender,
    session: u64,
}

impl HostedDialog for FileTransDialog {
    type Init = FileTransInit;
    const RESOURCE_ID: u16 = ctrl_id::DIALOG;

    fn create(hwnd: HWND, init: Self::Init) -> Result<Self> {
        let mut dialog = Self::new(
            hwnd,
            init.config,
            init.supervisor,
            init.actions,
            init.session,
        );
        dialog.initialize_controls()?;
        Ok(dialog)
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> DialogResult {
        if msg != WM_COMMAND {
            return DialogResult::Unhandled;
        }
        let id = (wparam & 0xFFFF) as u16;
        if id == IDCANCEL as u16 || id == ctrl_id::BTN_CLOSE {
            return DialogResult::Close(1);
        }
        let notify_code = ((wparam >> 16) & 0xFFFF) as u32;
        self.handle_command(id, notify_code);
        DialogResult::Handled(1)
    }

    fn applied_dpi(&mut self) -> Option<&mut u32> {
        Some(&mut self.applied_dpi)
    }

    fn destroy(&mut self) {
        self.actions.file_trans_dialog_closed(self.session);
    }

    fn can_defer(msg: u32) -> bool {
        msg == WM_COMMAND
    }
}

impl FileTransDialog {
    fn new(
        hwnd: HWND,
        config: Config,
        supervisor: Rc<FileTranslationSupervisor>,
        actions: AppActionSender,
        session: u64,
    ) -> Self {
        Self {
            hwnd,
            config,
            supervisor,
            applied_dpi: crate::dpi::dpi_for_window(hwnd),
            load_edit: HWND::default(),
            save_edit: HWND::default(),
            save_browser_btn: HWND::default(),
            preview_edit: HWND::default(),
            engine_label: HWND::default(),
            input_files: Vec::new(),
            output_files: Vec::new(),
            write_type: WriteType::TranslationOnly,
            no_trans_linefeed: false,
            actions,
            session,
        }
    }

    /// `resources/file_trans.rc`의 모델리스 DIALOGEX 리소스를 연다.
    pub fn show(
        parent: HWND,
        config: Config,
        supervisor: Rc<FileTranslationSupervisor>,
        actions: AppActionSender,
        session: u64,
    ) -> Result<HWND> {
        DialogHost::<Self>::show(
            parent,
            FileTransInit {
                config,
                supervisor,
                actions,
                session,
            },
        )
    }

    pub(crate) fn current_session() -> Option<u64> {
        DialogHost::<Self>::with_state(|dialog| dialog.session)
    }

    fn initialize_controls(&mut self) -> Result<()> {
        let get_control = |id| {
            let control = unsafe { GetDlgItem(self.hwnd, id) };
            if control.is_null() {
                Err(Error::new(
                    HRESULT(E_FAIL),
                    format!("파일 번역 컨트롤 ID {id}를 찾을 수 없습니다"),
                ))
            } else {
                Ok(control)
            }
        };

        self.load_edit = get_control(ctrl_id::LOAD_EDIT as i32)?;
        self.save_edit = get_control(ctrl_id::SAVE_EDIT as i32)?;
        self.save_browser_btn = get_control(ctrl_id::SAVE_BROWSER as i32)?;
        self.preview_edit = get_control(ctrl_id::PREVIEW_EDIT as i32)?;
        self.engine_label = get_control(ctrl_id::ENGINE_LABEL as i32)?;
        for id in [
            ctrl_id::LOAD_BROWSER,
            ctrl_id::OUTPUT_1,
            ctrl_id::OUTPUT_2,
            ctrl_id::OUTPUT_3,
            ctrl_id::NO_TRANS_LINEFEED,
            ctrl_id::BTN_TRANSLATE,
            ctrl_id::BTN_CLOSE,
        ] {
            get_control(id as i32)?;
        }

        unsafe {
            let _ = EnableWindow(self.save_browser_btn, 0);
            let _ = CheckDlgButton(self.hwnd, ctrl_id::OUTPUT_1 as i32, BST_CHECKED);
        }
        self.update_engine_label();
        Ok(())
    }

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
            _ => {}
        }
    }

    /// 엔진 안내 라벨 텍스트 갱신
    fn update_engine_label(&self) {
        use crate::translation::lang_utils::to_korean_name;
        let (engine_name, source, target) = {
            let config = &self.config;
            let engine = match config.translation.get_engine() {
                Ok(engine) => engine,
                Err(error) => {
                    let _ = set_window_text(self.engine_label, &format!("번역 설정 오류: {error}"));
                    return;
                }
            };
            let engine_name: String = match engine {
                TranslationEngine::EzTrans => "EzTrans64".into(),
                TranslationEngine::Google => "Google".into(),
                TranslationEngine::DeepL => "DeepL".into(),
                TranslationEngine::Papago => "Papago".into(),
                TranslationEngine::Llm => match config.translation.llm.get_provider() {
                    Ok(provider) => format!("LLM: {}", provider.display_name()),
                    Err(error) => format!("LLM 설정 오류: {error}"),
                },
                TranslationEngine::MysTranslater => "MyS Translater".into(),
                TranslationEngine::Custom => match config.translation.active_custom_api() {
                    Ok(api) => format!("Custom API: {}", api.name),
                    Err(error) => format!("Custom API 설정 오류: {error}"),
                },
            };
            let source = match config.translation.get_source_language() {
                Ok(language) => to_korean_name(language),
                Err(error) => {
                    let _ = set_window_text(self.engine_label, &format!("번역 설정 오류: {error}"));
                    return;
                }
            };
            let target = match config.translation.get_target_language() {
                Ok(language) => to_korean_name(language),
                Err(error) => {
                    let _ = set_window_text(self.engine_label, &format!("번역 설정 오류: {error}"));
                    return;
                }
            };
            (engine_name, source, target)
        };
        let text = format!(
            "현재 번역 엔진: {engine_name} ({source} → {target})\r\n엔진/언어는 \"번역\" 또는 \"설정\" 다이얼로그에서 변경할 수 있습니다.",
        );
        let _ = set_window_text(self.engine_label, &text);
    }

    /// 입력 파일 선택 (다중 선택)
    fn browse_input_files(&mut self) {
        let filters = [
            FileFilter {
                name: "텍스트 파일 (*.txt)",
                spec: "*.txt",
            },
            FileFilter {
                name: "모든 파일 (*.*)",
                spec: "*.*",
            },
        ];
        let picked = match open_files_multi(self.hwnd, "입력 파일 선택", &filters) {
            Ok(Some(paths)) => paths,
            Ok(None) => return,
            Err(error) => {
                self.show_file_dialog_error(&error);
                return;
            }
        };

        self.input_files = picked;
        self.output_files = match default_output_paths(&self.input_files) {
            Ok(outputs) => outputs,
            Err(error) => {
                self.input_files.clear();
                unsafe {
                    let message = crate::win32::to_wide(&error.to_string());
                    let _ = MessageBoxW(
                        self.hwnd,
                        message.as_ptr(),
                        crate::win32::to_wide("경로 오류").as_ptr(),
                        MB_ICONERROR,
                    );
                }
                return;
            }
        };

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

        // SAFETY: load_edit/save_edit/save_browser_btn are valid control handles
        // from create_controls.
        unsafe {
            let _ = set_window_text(self.load_edit, &input_display.join(", "));
            let _ = set_window_text(self.save_edit, &output_display.join(", "));
            let _ = EnableWindow(
                self.save_browser_btn,
                if self.input_files.len() == 1 { 1 } else { 0 },
            );
        }

        if let Some(first_file) = self.input_files.first() {
            self.show_preview(first_file);
        }
    }

    /// 출력 파일 위치 변경 (단일 파일만)
    fn browse_output_file(&mut self) {
        if self.input_files.len() != 1 {
            return;
        }

        let filters = [
            FileFilter {
                name: "텍스트 파일 (*.txt)",
                spec: "*.txt",
            },
            FileFilter {
                name: "모든 파일 (*.*)",
                spec: "*.*",
            },
        ];
        let initial = self.output_files.first().map(std::path::PathBuf::as_path);
        let path = match save_file(self.hwnd, "출력 파일 위치", &filters, Some("txt"), initial)
        {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                self.show_file_dialog_error(&error);
                return;
            }
        };

        let path_str = path.to_string_lossy().to_string();
        self.output_files[0] = path;
        let _ = set_window_text(self.save_edit, &path_str);
    }

    /// 파일 미리보기 (처음 7줄). 입력은 UTF-8 / UTF-8 BOM 만 허용한다.
    fn show_preview(&self, path: &Path) {
        const PREVIEW_BYTE_LIMIT: u64 = 64 * 1024;
        let content = match crate::file_trans::read_utf8_preview(path, 7, PREVIEW_BYTE_LIMIT) {
            Ok(content) => content,
            Err(msg) => format!("! {msg}"),
        };

        let _ = set_window_text(self.preview_edit, &content);
    }

    fn show_file_dialog_error(&self, error: &windows_core::Error) {
        tracing::error!("파일 대화상자 오류: {error}");
        let message = crate::win32::to_wide(&format!("파일 대화상자를 열 수 없습니다.\n{error}"));
        unsafe {
            let _ = MessageBoxW(
                self.hwnd,
                message.as_ptr(),
                crate::win32::to_wide("오류").as_ptr(),
                MB_ICONERROR,
            );
        }
    }

    /// 번역 시작
    fn start_translation(&mut self) {
        if FileTransProgressDialog::activate_existing() {
            return;
        }
        // 다이얼로그가 떠 있는 동안 다른 창에서 설정이 바뀌었을 수 있으므로
        // 번역 시작 직전에 안내 라벨을 한 번 갱신해 최신 상태를 보여준다.
        self.update_engine_label();

        if self.input_files.is_empty() {
            // SAFETY: self.hwnd is a valid dialog window handle used as the message box owner.
            unsafe {
                let _ = MessageBoxW(
                    self.hwnd,
                    crate::win32::to_wide("파일을 먼저 선택해주세요.").as_ptr(),
                    crate::win32::to_wide("알림").as_ptr(),
                    MB_ICONINFORMATION,
                );
            }
            return;
        }

        if let Err(error) = validate_job_paths(&self.input_files, &self.output_files) {
            unsafe {
                let message = crate::win32::to_wide(&error.to_string());
                let _ = MessageBoxW(
                    self.hwnd,
                    message.as_ptr(),
                    crate::win32::to_wide("경로 오류").as_ptr(),
                    MB_ICONERROR,
                );
            }
            return;
        }

        let spec = {
            let config = &self.config;
            PreparedJob::from_config(&config.translation)
        };
        let spec = match spec {
            Ok(spec) => spec,
            Err(error) => {
                unsafe {
                    let message = crate::win32::to_wide(&error.to_string());
                    let _ = MessageBoxW(
                        self.hwnd,
                        message.as_ptr(),
                        crate::win32::to_wide("번역 설정 오류").as_ptr(),
                        MB_ICONERROR,
                    );
                }
                return;
            }
        };
        let job_data = FileTranslationRequest {
            input_files: self.input_files.clone(),
            output_files: self.output_files.clone(),
            write_type: self.write_type,
            no_trans_linefeed: self.no_trans_linefeed,
            // runner가 작업별 취소 토큰을 설정한다.
            cancel_token: Default::default(),
            translation: spec,
        };
        let task = match self.supervisor.start(job_data) {
            Ok(task) => task,
            Err(error) => {
                crate::dialogs::helpers::show_error_message(
                    self.hwnd,
                    "파일 번역 오류",
                    &error.to_string(),
                );
                return;
            }
        };
        if let Err(e) = FileTransProgressDialog::show(self.hwnd, task) {
            tracing::error!("Failed to create progress dialog: {:?}", e);
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "파일 번역 오류",
                &format!("진행률 창을 만들 수 없어 작업을 시작하지 못했습니다.\n\n{e}"),
            );
        }
    }
}
