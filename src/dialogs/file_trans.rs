//! 파일 번역 대화상자
//!
//! 다중 파일 선택 및 배치 번역 기능.
//! Common Item Dialog (IFileOpenDialog / IFileSaveDialog) 사용.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::{
    Win32::{
        Foundation::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::Controls::{BST_CHECKED, CheckDlgButton},
        UI::Input::KeyboardAndMouse::EnableWindow,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::file_dialog::{FileFilter, open_files_multi, save_file};
use super::file_trans_progress::FileTransProgressDialog;
use super::helpers::{
    center_dialog_on_monitor, register_resource_dialog, rescale_dialog_children_for_dpi,
    show_dialog_window, unregister_resource_dialog,
};
use crate::config::Config;
use crate::define_dialog_instance;
use crate::translation::{EngineCredentials, Language, TranslationEngine};
use crate::util::to_wide;

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
    cancel_token: Arc<AtomicBool>,
}

define_dialog_instance!(FILE_TRANS_INSTANCE: FileTransDialog);

struct PendingFileTrans {
    config: Rc<RefCell<Config>>,
}

thread_local! {
    static FILE_TRANS_PENDING: RefCell<Option<PendingFileTrans>> = const { RefCell::new(None) };
    static FILE_TRANS_INIT_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// `resources/file_trans.rc`에서 생성된 모델리스 다이얼로그의 메시지 콜백.
unsafe extern "system" fn file_trans_dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    unsafe {
        if msg == WM_INITDIALOG {
            let pending = FILE_TRANS_PENDING.with(|slot| slot.borrow_mut().take());
            let Some(PendingFileTrans { config }) = pending else {
                FILE_TRANS_INIT_ERROR.with(|slot| {
                    *slot.borrow_mut() = Some("파일 번역 창 초기화 인자가 없습니다".into());
                });
                return 0;
            };

            let dialog = Rc::new(RefCell::new(FileTransDialog::new(hwnd, config)));
            FILE_TRANS_INSTANCE.with(|slot| {
                *slot.borrow_mut() = Some(dialog.clone());
            });

            if let Err(error) = dialog.borrow_mut().initialize_controls() {
                FILE_TRANS_INSTANCE.with(|slot| {
                    slot.borrow_mut().take();
                });
                FILE_TRANS_INIT_ERROR.with(|slot| {
                    *slot.borrow_mut() = Some(error.to_string());
                });
                return 0;
            }
            register_resource_dialog(hwnd);
            return 1;
        }

        let instance = FILE_TRANS_INSTANCE.with(|slot| {
            let Ok(guard) = slot.try_borrow() else {
                return None;
            };
            guard.clone()
        });
        let Some(dialog) = instance else {
            return 0;
        };

        match msg {
            WM_DPICHANGED => {
                if let Ok(mut dialog) = dialog.try_borrow_mut() {
                    dialog.handle_dpi_changed(wparam, lparam);
                }
                1
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as u16;
                let notify_code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                if id == IDCANCEL.0 as u16 {
                    let _ = DestroyWindow(hwnd);
                } else if let Ok(mut dialog) = dialog.try_borrow_mut() {
                    dialog.handle_command(id, notify_code);
                }
                1
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                1
            }
            WM_DESTROY => {
                unregister_resource_dialog(hwnd);
                FILE_TRANS_INSTANCE.with(|slot| {
                    if let Ok(mut guard) = slot.try_borrow_mut() {
                        *guard = None;
                    }
                });
                1
            }
            _ => 0,
        }
    }
}

impl FileTransDialog {
    fn new(hwnd: HWND, config: Rc<RefCell<Config>>) -> Self {
        Self {
            hwnd,
            config,
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
            cancel_token: Arc::new(AtomicBool::new(false)),
        }
    }

    /// `resources/file_trans.rc`의 모델리스 DIALOGEX 리소스를 연다.
    pub fn show(parent: HWND, config: Rc<RefCell<Config>>) -> Result<HWND> {
        let existing = FILE_TRANS_INSTANCE
            .with(|slot| slot.borrow().as_ref().map(|dialog| dialog.borrow().hwnd));
        if let Some(hwnd) = existing
            && unsafe { IsWindow(Some(hwnd)).as_bool() }
        {
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
            return Ok(hwnd);
        }

        let instance = unsafe { GetModuleHandleW(None)? };
        FILE_TRANS_INIT_ERROR.with(|slot| {
            slot.borrow_mut().take();
        });
        FILE_TRANS_PENDING.with(|slot| {
            *slot.borrow_mut() = Some(PendingFileTrans { config });
        });

        let result = unsafe {
            CreateDialogParamW(
                Some(instance.into()),
                PCWSTR(ctrl_id::DIALOG as usize as *const u16),
                Some(parent),
                Some(file_trans_dialog_proc),
                LPARAM(0),
            )
        };

        let hwnd = match result {
            Ok(hwnd) => hwnd,
            Err(error) => {
                FILE_TRANS_PENDING.with(|slot| {
                    slot.borrow_mut().take();
                });
                FILE_TRANS_INIT_ERROR.with(|slot| {
                    slot.borrow_mut().take();
                });
                return Err(error);
            }
        };

        if let Some(message) = FILE_TRANS_INIT_ERROR.with(|slot| slot.borrow_mut().take()) {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(Error::new(E_FAIL, message));
        }

        unsafe {
            center_dialog_on_monitor(hwnd, parent);
            show_dialog_window(hwnd);
        }
        Ok(hwnd)
    }

    fn initialize_controls(&mut self) -> Result<()> {
        let get_control = |id| {
            unsafe { GetDlgItem(Some(self.hwnd), id) }.map_err(|_| {
                Error::new(
                    E_FAIL,
                    format!("파일 번역 컨트롤 ID {id}를 찾을 수 없습니다"),
                )
            })
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
            let _ = EnableWindow(self.save_browser_btn, false);
            let _ = CheckDlgButton(self.hwnd, ctrl_id::OUTPUT_1 as i32, BST_CHECKED);
        }
        self.update_engine_label();
        Ok(())
    }

    fn handle_dpi_changed(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let new_dpi = (wparam.0 & 0xFFFF) as u32;
        rescale_dialog_children_for_dpi(self.hwnd, self.applied_dpi, new_dpi);
        self.applied_dpi = new_dpi;

        if lparam.0 != 0 {
            unsafe {
                let rect = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
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
            BTN_CLOSE => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
            _ => {}
        }
    }

    /// 엔진 안내 라벨 텍스트 갱신
    fn update_engine_label(&self) {
        use crate::translation::lang_utils::to_korean_name;
        let (engine_name, source, target) = {
            let config = self.config.borrow();
            let engine = config.translation.get_engine();
            let engine_name: String = match engine {
                TranslationEngine::EzTrans => "EzTrans".into(),
                TranslationEngine::Google => "Google".into(),
                TranslationEngine::DeepL => "DeepL".into(),
                TranslationEngine::Papago => "Papago".into(),
                TranslationEngine::Llm => {
                    format!(
                        "LLM: {}",
                        config.translation.llm.get_provider().display_name()
                    )
                }
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
        unsafe {
            Self::set_edit_text(self.engine_label, &text);
        }
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
            FileFilter {
                name: "텍스트 파일 (*.txt)",
                spec: "*.txt",
            },
            FileFilter {
                name: "모든 파일 (*.*)",
                spec: "*.*",
            },
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
        let initial = self.output_files.first().map(|p| p.as_path());
        let Some(path) = save_file(self.hwnd, "출력 파일 위치", &filters, Some("txt"), initial)
        else {
            return;
        };

        let path_str = path.to_string_lossy().to_string();
        self.output_files[0] = path;
        // SAFETY: save_edit is a valid control handle from create_controls.
        unsafe {
            Self::set_edit_text(self.save_edit, &path_str);
        }
    }

    /// 파일 미리보기 (처음 7줄). 입력은 UTF-8 / UTF-8 BOM 만 허용한다.
    fn show_preview(&self, path: &Path) {
        use std::io::{BufRead, BufReader, Cursor};

        let content = match crate::util::read_utf8_translation_input(path) {
            Ok(body) => {
                let reader = BufReader::new(Cursor::new(body));
                let lines: Vec<String> = reader.lines().take(7).filter_map(|l| l.ok()).collect();
                lines.join("\r\n")
            }
            Err(msg) => format!("! {msg}"),
        };

        // SAFETY: self.preview_edit is a valid edit control handle created in create_controls.
        unsafe {
            Self::set_edit_text(self.preview_edit, &content);
        }
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
                    w!(
                        "EzTrans 경로가 설정되지 않았습니다. 번역 설정에서 DLL/DAT 경로를 지정하세요."
                    ),
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
}
