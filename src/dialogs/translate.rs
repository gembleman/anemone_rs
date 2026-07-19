//! Engine과 언어를 선택해 수동 번역하는 dialog.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use windows::{
    Win32::{
        Foundation::*,
        System::DataExchange::*,
        System::LibraryLoader::GetModuleHandleW,
        System::Memory::*,
        System::Ole::CF_UNICODETEXT,
        UI::Controls::*,
        UI::Input::KeyboardAndMouse::*,
        UI::Shell::{DefSubclassProc, SetWindowSubclass},
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::helpers::{
    center_dialog_on_monitor, get_window_text, register_resource_dialog,
    rescale_dialog_children_for_dpi, set_window_text, show_dialog_window,
    unregister_resource_dialog,
};
use crate::app::action::AppActionSender;
use crate::define_dialog_instance;
use crate::util::to_wide;

use crate::clipboard::{ClipboardGuard, OwnedGlobalMemory};
use crate::config::Config;
use crate::constants::WM_TRANSLATION_COMPLETE;
use crate::translation::manual::{ManualOutputFormat, ManualTranslationOptions};
use crate::translation::{Language, LlmProvider, PreparedJob, TranslationEngine};
use crate::translation_ui::GuiTranslationHost;

// 컨트롤 ID
mod ctrl_id {
    pub const DIALOG: u16 = 105;
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
    // LLM 하위 설정 (엔진이 LLM일 때만 노출)
    pub const COMBO_LLM_PROVIDER: u16 = 2030;
    pub const EDIT_LLM_MODEL: u16 = 2031;
    pub const EDIT_LLM_API_KEY: u16 = 2032;
    pub const LLM_GROUP: u16 = 2040;
    pub const LLM_PROVIDER_LABEL: u16 = 2041;
    pub const LLM_MODEL_LABEL: u16 = 2042;
    pub const LLM_API_KEY_LABEL: u16 = 2043;
}

// 서브클래스 ID (uIdSubclass): 컨트롤별로 구분
mod subclass_id {
    pub const SOURCE_EDIT: usize = 1;
    pub const DEST_EDIT: usize = 2;
}

const AUTO_TRANSLATE_TIMER: usize = 0xA710;
const CUSTOM_ENGINE_INDEX: usize = TranslationEngine::Custom as usize;

/// 번역 대화상자
pub struct TranslateDialog {
    hwnd: HWND,
    config: Config,
    translation_service: Rc<GuiTranslationHost>,
    applied_dpi: u32,
    source_edit: HWND,
    dest_edit: HWND,
    engine_combo: HWND,
    source_lang_combo: HWND,
    target_lang_combo: HWND,
    /// LLM 하위 컨트롤. 엔진이 LLM일 때만 visible 처리.
    llm_group: HWND,
    llm_provider_label: HWND,
    llm_provider_combo: HWND,
    llm_model_label: HWND,
    llm_model_edit: HWND,
    llm_api_key_label: HWND,
    llm_api_key_edit: HWND,
    one_go: bool,
    manual_options: ManualTranslationOptions,
    /// 현재 최신 요청 ID. 새 자동 요청이 들어오면 워커가 이전 요청을 취소한다.
    in_flight_id: Option<u64>,
    last_submitted_source: String,
    actions: AppActionSender,
    session: u64,
}

define_dialog_instance!(TRANSLATE_INSTANCE: TranslateDialog);

struct PendingTranslate {
    config: Config,
    translation_service: Rc<GuiTranslationHost>,
    actions: AppActionSender,
    session: u64,
}

thread_local! {
    static TRANSLATE_PENDING: std::cell::RefCell<Option<PendingTranslate>> = const { std::cell::RefCell::new(None) };
    static TRANSLATE_INIT_ERROR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// `resources/translate.rc`에서 생성된 모델리스 다이얼로그의 메시지 콜백.
unsafe extern "system" fn translate_dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    unsafe {
        if msg == WM_INITDIALOG {
            let pending = TRANSLATE_PENDING.with(|slot| slot.borrow_mut().take());
            let Some(PendingTranslate {
                config,
                translation_service,
                actions,
                session,
            }) = pending
            else {
                TRANSLATE_INIT_ERROR.with(|slot| {
                    *slot.borrow_mut() = Some("번역 창 초기화 인자가 없습니다".into());
                });
                return 0;
            };

            let dialog = Rc::new(RefCell::new(TranslateDialog::new(
                hwnd,
                config,
                translation_service,
                actions,
                session,
            )));
            TRANSLATE_INSTANCE.with(|slot| {
                *slot.borrow_mut() = Some(dialog.clone());
            });

            if let Err(error) = dialog.borrow_mut().initialize_controls() {
                TRANSLATE_INSTANCE.with(|slot| {
                    slot.borrow_mut().take();
                });
                TRANSLATE_INIT_ERROR.with(|slot| {
                    *slot.borrow_mut() = Some(error.to_string());
                });
                return 0;
            }
            register_resource_dialog(hwnd);
            return 1;
        }

        let instance = TRANSLATE_INSTANCE.with(|slot| {
            let Ok(guard) = slot.try_borrow() else {
                return None;
            };
            guard.clone()
        });
        let Some(dialog) = instance else {
            return 0;
        };

        let mut can_flush = false;
        let result = match msg {
            WM_TRANSLATION_COMPLETE => {
                if let Ok(mut dialog) = dialog.try_borrow_mut() {
                    dialog.handle_translation_complete();
                    can_flush = true;
                } else {
                    super::helpers::defer_dialog_message(hwnd, msg, WPARAM(0), LPARAM(0));
                }
                1
            }
            WM_DPICHANGED => {
                if let Ok(mut dialog) = dialog.try_borrow_mut() {
                    dialog.handle_dpi_changed(wparam, lparam);
                    can_flush = true;
                } else {
                    super::helpers::defer_dialog_dpi_change(hwnd, wparam, lparam);
                }
                1
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as u16;
                let notify_code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                if id == IDCANCEL.0 as u16 {
                    let _ = DestroyWindow(hwnd);
                } else if let Ok(mut dialog) = dialog.try_borrow_mut() {
                    if id == ctrl_id::SOURCE_EDIT && notify_code == EN_CHANGE {
                        dialog.invalidate_stale_translation();
                        if dialog.one_go {
                            dialog.schedule_auto_translate();
                        }
                    }
                    dialog.handle_command(id, notify_code);
                    can_flush = true;
                } else {
                    super::helpers::defer_dialog_message(hwnd, msg, wparam, lparam);
                }
                1
            }
            WM_TIMER if wparam.0 == AUTO_TRANSLATE_TIMER => {
                let _ = KillTimer(Some(hwnd), AUTO_TRANSLATE_TIMER);
                if let Ok(mut dialog) = dialog.try_borrow_mut() {
                    dialog.do_translate();
                    can_flush = true;
                } else {
                    super::helpers::defer_dialog_message(hwnd, msg, wparam, LPARAM(0));
                }
                1
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                1
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), AUTO_TRANSLATE_TIMER);
                dialog.borrow().translation_service.unregister(hwnd);
                let (actions, session) = {
                    let dialog = dialog.borrow();
                    (dialog.actions.clone(), dialog.session)
                };
                actions.translate_dialog_closed(session);
                unregister_resource_dialog(hwnd);
                TRANSLATE_INSTANCE.with(|slot| {
                    if let Ok(mut guard) = slot.try_borrow_mut() {
                        *guard = None;
                    }
                });
                1
            }
            _ => 0,
        };
        if can_flush {
            super::helpers::flush_deferred_dialog_messages(hwnd);
        }
        result
    }
}

impl TranslateDialog {
    fn new(
        hwnd: HWND,
        config: Config,
        translation_service: Rc<GuiTranslationHost>,
        actions: AppActionSender,
        session: u64,
    ) -> Self {
        Self {
            hwnd,
            config,
            translation_service,
            applied_dpi: crate::dpi::dpi_for_window(hwnd),
            source_edit: HWND::default(),
            dest_edit: HWND::default(),
            engine_combo: HWND::default(),
            source_lang_combo: HWND::default(),
            target_lang_combo: HWND::default(),
            llm_group: HWND::default(),
            llm_provider_label: HWND::default(),
            llm_provider_combo: HWND::default(),
            llm_model_label: HWND::default(),
            llm_model_edit: HWND::default(),
            llm_api_key_label: HWND::default(),
            llm_api_key_edit: HWND::default(),
            one_go: false,
            manual_options: ManualTranslationOptions::default(),
            in_flight_id: None,
            last_submitted_source: String::new(),
            actions,
            session,
        }
    }

    /// `resources/translate.rc`의 모델리스 DIALOGEX 리소스를 연다.
    pub fn show(
        parent: HWND,
        config: Config,
        translation_service: Rc<GuiTranslationHost>,
        actions: AppActionSender,
        session: u64,
    ) -> Result<HWND> {
        let existing = TRANSLATE_INSTANCE
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
        TRANSLATE_INIT_ERROR.with(|slot| {
            slot.borrow_mut().take();
        });
        TRANSLATE_PENDING.with(|slot| {
            *slot.borrow_mut() = Some(PendingTranslate {
                config,
                translation_service,
                actions,
                session,
            });
        });

        let result = unsafe {
            CreateDialogParamW(
                Some(instance.into()),
                PCWSTR(ctrl_id::DIALOG as usize as *const u16),
                Some(parent),
                Some(translate_dialog_proc),
                LPARAM(0),
            )
        };

        let hwnd = match result {
            Ok(hwnd) => hwnd,
            Err(error) => {
                TRANSLATE_PENDING.with(|slot| {
                    slot.borrow_mut().take();
                });
                TRANSLATE_INIT_ERROR.with(|slot| {
                    slot.borrow_mut().take();
                });
                return Err(error);
            }
        };

        if let Some(message) = TRANSLATE_INIT_ERROR.with(|slot| slot.borrow_mut().take()) {
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

    pub(crate) fn current_session() -> Option<u64> {
        let (hwnd, session) = TRANSLATE_INSTANCE.with(|slot| {
            let dialog = slot.borrow();
            let dialog = dialog.as_ref()?.try_borrow().ok()?;
            Some((dialog.hwnd, dialog.session))
        })?;
        unsafe { IsWindow(Some(hwnd)).as_bool().then_some(session) }
    }

    fn initialize_controls(&mut self) -> Result<()> {
        let get_control = |id| {
            unsafe { GetDlgItem(Some(self.hwnd), id) }
                .map_err(|_| Error::new(E_FAIL, format!("번역 컨트롤 ID {id}를 찾을 수 없습니다")))
        };

        self.source_edit = get_control(ctrl_id::SOURCE_EDIT as i32)?;
        self.dest_edit = get_control(ctrl_id::DEST_EDIT as i32)?;
        self.engine_combo = get_control(ctrl_id::COMBO_ENGINE as i32)?;
        self.source_lang_combo = get_control(ctrl_id::COMBO_SOURCE_LANG as i32)?;
        self.target_lang_combo = get_control(ctrl_id::COMBO_TARGET_LANG as i32)?;
        self.llm_group = get_control(ctrl_id::LLM_GROUP as i32)?;
        self.llm_provider_label = get_control(ctrl_id::LLM_PROVIDER_LABEL as i32)?;
        self.llm_provider_combo = get_control(ctrl_id::COMBO_LLM_PROVIDER as i32)?;
        self.llm_model_label = get_control(ctrl_id::LLM_MODEL_LABEL as i32)?;
        self.llm_model_edit = get_control(ctrl_id::EDIT_LLM_MODEL as i32)?;
        self.llm_api_key_label = get_control(ctrl_id::LLM_API_KEY_LABEL as i32)?;
        self.llm_api_key_edit = get_control(ctrl_id::EDIT_LLM_API_KEY as i32)?;
        for id in [
            ctrl_id::BTN_TRANSLATE,
            ctrl_id::BTN_COPY,
            ctrl_id::BTN_CLEAR,
            ctrl_id::CHK_ONE_GO,
            ctrl_id::CHK_NO_LINEFEED,
            ctrl_id::RADIO_OUTPUT_1,
            ctrl_id::RADIO_OUTPUT_2,
            ctrl_id::RADIO_OUTPUT_3,
        ] {
            get_control(id as i32)?;
        }

        unsafe {
            let _ = SendMessageW(self.source_edit, EM_SETLIMITTEXT, Some(WPARAM(0)), None);
            let _ = SendMessageW(self.dest_edit, EM_SETLIMITTEXT, Some(WPARAM(0)), None);
            let _ = SetWindowSubclass(
                self.source_edit,
                Some(Self::edit_subclass_proc),
                subclass_id::SOURCE_EDIT,
                0,
            );
            let _ = SetWindowSubclass(
                self.dest_edit,
                Some(Self::edit_subclass_proc),
                subclass_id::DEST_EDIT,
                0,
            );
            let _ = CheckDlgButton(self.hwnd, ctrl_id::RADIO_OUTPUT_1 as i32, BST_CHECKED);
        }

        for engine in &TranslationEngine::ALL[..CUSTOM_ENGINE_INDEX] {
            self.add_combobox_item(self.engine_combo, engine.display_name());
        }
        {
            let config = &self.config;
            if config.translation.custom_apis.is_empty() {
                self.add_combobox_item(self.engine_combo, &config.translation.custom.name);
            } else {
                for api in &config.translation.custom_apis {
                    self.add_combobox_item(self.engine_combo, &api.name);
                }
            }
        }
        unsafe {
            let _ = SendMessageW(
                self.engine_combo,
                CB_SETDROPPEDWIDTH,
                Some(WPARAM(220)),
                None,
            );
        }
        for provider in LlmProvider::ALL {
            self.add_combobox_item(self.llm_provider_combo, provider.display_name());
        }

        let (engine, engine_index, source_index, target_index, provider_index, model, api_key) = {
            let config = &self.config;
            let engine = config
                .translation
                .get_engine()
                .map_err(|error| Error::new(E_INVALIDARG, error.to_string()))?;
            let engine_index = if engine == TranslationEngine::Custom {
                CUSTOM_ENGINE_INDEX
                    + config
                        .translation
                        .active_custom_api_index()
                        .map_err(|error| Error::new(E_INVALIDARG, error.to_string()))?
            } else {
                engine as usize
            };
            let provider_index = config
                .translation
                .llm
                .get_provider()
                .map_err(|error| Error::new(E_INVALIDARG, error.to_string()))?
                as u8 as usize;
            let source = config
                .translation
                .get_source_language()
                .map_err(|error| Error::new(E_INVALIDARG, error.to_string()))?;
            let target = config
                .translation
                .get_target_language()
                .map_err(|error| Error::new(E_INVALIDARG, error.to_string()))?;
            let target_index = engine
                .supported_targets_for(source)
                .iter()
                .position(|&language| language == target)
                .unwrap_or(0);
            (
                engine,
                engine_index,
                config
                    .translation
                    .source_lang_index(engine)
                    .map_err(|error| Error::new(E_INVALIDARG, error))?,
                target_index,
                provider_index,
                config.translation.llm.model.clone(),
                config.translation.llm.api_key.clone(),
            )
        };

        unsafe {
            let _ = SendMessageW(
                self.engine_combo,
                CB_SETCURSEL,
                Some(WPARAM(engine_index)),
                None,
            );
        }
        self.populate_language_combos(engine);
        let selected_source = engine
            .supported_source_languages()
            .get(source_index)
            .copied()
            .ok_or_else(|| Error::new(E_INVALIDARG, "잘못된 소스 언어 설정"))?;
        unsafe {
            let _ = SendMessageW(
                self.source_lang_combo,
                CB_SETCURSEL,
                Some(WPARAM(source_index)),
                None,
            );
            self.populate_target_combo(engine, selected_source);
            let _ = SendMessageW(
                self.target_lang_combo,
                CB_SETCURSEL,
                Some(WPARAM(target_index)),
                None,
            );
            let _ = SendMessageW(
                self.llm_provider_combo,
                CB_SETCURSEL,
                Some(WPARAM(provider_index)),
                None,
            );
            let _ = set_window_text(self.llm_model_edit, &model);
            let _ = set_window_text(self.llm_api_key_edit, &api_key);
        }
        self.update_llm_group_visibility(engine);
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

    /// 명령 처리
    fn handle_command(&mut self, cmd: u16, notify_code: u32) {
        use ctrl_id::*;

        match cmd {
            BTN_TRANSLATE => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), AUTO_TRANSLATE_TIMER);
                }
                self.do_translate();
            }
            BTN_COPY => self.copy_to_clipboard(),
            BTN_CLEAR => self.clear_text(),
            CHK_ONE_GO => {
                self.one_go = !self.one_go;
                if !self.one_go {
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), AUTO_TRANSLATE_TIMER);
                    }
                }
            }
            CHK_NO_LINEFEED => {
                self.manual_options.remove_linefeeds = !self.manual_options.remove_linefeeds;
            }
            RADIO_OUTPUT_1 => self.manual_options.output_format = ManualOutputFormat::Normal,
            RADIO_OUTPUT_2 => self.manual_options.output_format = ManualOutputFormat::Brackets,
            RADIO_OUTPUT_3 => self.manual_options.output_format = ManualOutputFormat::NameSplit,
            // CBN_SELCHANGE
            COMBO_ENGINE | COMBO_SOURCE_LANG | COMBO_TARGET_LANG if notify_code == 1 => {
                self.invalidate_translation_route();
                if cmd == COMBO_ENGINE {
                    // SAFETY: engine_combo is a valid handle.
                    let engine_idx = unsafe {
                        SendMessageW(self.engine_combo, CB_GETCURSEL, None, None).0 as u8
                    };
                    let Some(engine) = engine_from_combo_index(engine_idx as usize) else {
                        return;
                    };
                    self.populate_language_combos(engine);
                    self.update_llm_group_visibility(engine);
                } else if cmd == COMBO_SOURCE_LANG {
                    let engine_idx = unsafe {
                        SendMessageW(self.engine_combo, CB_GETCURSEL, None, None).0 as u8
                    };
                    let source_idx = unsafe {
                        SendMessageW(self.source_lang_combo, CB_GETCURSEL, None, None).0 as usize
                    };
                    let Some(engine) = engine_from_combo_index(engine_idx as usize) else {
                        return;
                    };
                    if let Some(&source) = engine.supported_source_languages().get(source_idx) {
                        self.populate_target_combo(engine, source);
                    }
                }
                self.apply_current_settings();
            }
            COMBO_LLM_PROVIDER if notify_code == 1 => {
                self.invalidate_translation_route();
                self.apply_llm_provider();
            }
            EDIT_LLM_MODEL if notify_code == EN_CHANGE => {
                self.invalidate_translation_route();
                self.apply_llm_model();
            }
            EDIT_LLM_API_KEY if notify_code == EN_CHANGE => {
                self.invalidate_translation_route();
                self.apply_llm_api_key();
            }
            _ => {}
        }
    }
}

impl TranslateDialog {
    /// 엔진에 맞게 언어 콤보박스 항목 갱신
    fn populate_language_combos(&self, engine: TranslationEngine) {
        use crate::translation::lang_utils;
        // SAFETY: combo handles are valid controls obtained during initialization.
        unsafe {
            let _ = SendMessageW(self.source_lang_combo, CB_RESETCONTENT, None, None);
            for &lang in engine.supported_source_languages() {
                self.add_combobox_item(self.source_lang_combo, lang_utils::to_korean_name(lang));
            }
            let _ = SendMessageW(self.source_lang_combo, CB_SETCURSEL, Some(WPARAM(0)), None);

            let _ = SendMessageW(self.target_lang_combo, CB_RESETCONTENT, None, None);
            let source = engine
                .supported_source_languages()
                .first()
                .copied()
                .unwrap_or(Language::Jpn);
            self.populate_target_combo(engine, source);
        }
    }

    fn populate_target_combo(&self, engine: TranslationEngine, source: Language) {
        unsafe {
            let _ = SendMessageW(self.target_lang_combo, CB_RESETCONTENT, None, None);
            for lang in engine.supported_targets_for(source) {
                self.add_combobox_item(
                    self.target_lang_combo,
                    crate::translation::lang_utils::to_korean_name(lang),
                );
            }
            let _ = SendMessageW(self.target_lang_combo, CB_SETCURSEL, Some(WPARAM(0)), None);
        }
    }

    /// 엔진이 LLM일 때만 LLM 그룹 노출.
    fn update_llm_group_visibility(&self, engine: TranslationEngine) {
        let show = if engine == TranslationEngine::Llm {
            SW_SHOW
        } else {
            SW_HIDE
        };
        // SAFETY: 모든 LLM 그룹 HWND는 리소스 템플릿에서 얻은 유효한 핸들.
        unsafe {
            let _ = ShowWindow(self.llm_group, show);
            let _ = ShowWindow(self.llm_provider_label, show);
            let _ = ShowWindow(self.llm_provider_combo, show);
            let _ = ShowWindow(self.llm_model_label, show);
            let _ = ShowWindow(self.llm_model_edit, show);
            let _ = ShowWindow(self.llm_api_key_label, show);
            let _ = ShowWindow(self.llm_api_key_edit, show);
        }
    }

    fn apply_llm_provider(&mut self) {
        // SAFETY: 콤보 핸들은 리소스 템플릿에서 얻은 유효한 핸들.
        let sel =
            unsafe { SendMessageW(self.llm_provider_combo, CB_GETCURSEL, None, None).0 as u8 };
        let Some(provider) = LlmProvider::from_u8(sel) else {
            return;
        };
        self.config.translation.llm.set_provider(provider);
    }

    fn apply_llm_model(&mut self) {
        // SAFETY: edit 핸들은 리소스 템플릿에서 얻은 유효한 핸들.
        let text = get_window_text(self.llm_model_edit);
        self.config.translation.llm.model = text;
    }

    fn apply_llm_api_key(&mut self) {
        // SAFETY: edit 핸들은 리소스 템플릿에서 얻은 유효한 핸들.
        let text = get_window_text(self.llm_api_key_edit);
        self.config.translation.llm.api_key = text;
    }

    fn add_combobox_item(&self, combo: HWND, text: &str) {
        // SAFETY: combo is a valid combobox handle. wide string is valid for the call.
        unsafe {
            let wide = to_wide(text);
            let _ = SendMessageW(
                combo,
                CB_ADDSTRING,
                None,
                Some(LPARAM(wide.as_ptr() as isize)),
            );
        }
    }

    /// Ctrl+A를 추가하는 edit control subclass procedure.
    // SAFETY: This is a subclassed Win32 window procedure. The system provides valid params.
    unsafe extern "system" fn edit_subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _uid_subclass: usize,
        _ref_data: usize,
    ) -> LRESULT {
        // SAFETY: hwnd is a valid edit control owned by this dialog while subclassed.
        unsafe {
            if msg == WM_KEYDOWN
                && wparam.0 == 'A' as usize
                && (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0
            {
                let _ = SendMessageW(hwnd, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                return LRESULT(0);
            }

            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
    }

    /// 원문 텍스트 가져오기
    fn get_source_text(&self) -> String {
        // SAFETY: self.source_edit is a valid edit control handle from the resource template.
        get_window_text(self.source_edit)
    }

    /// 번역 결과 설정
    fn set_dest_text(&self, text: &str) {
        // SAFETY: self.dest_edit is a valid edit control handle from the resource template.
        let _ = set_window_text(self.dest_edit, text);
    }

    fn schedule_auto_translate(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), AUTO_TRANSLATE_TIMER);
        }
        let engine_idx =
            unsafe { SendMessageW(self.engine_combo, CB_GETCURSEL, None, None).0 as u8 };
        let Some(engine) = engine_from_combo_index(engine_idx as usize) else {
            return;
        };
        let delay_ms = if engine == TranslationEngine::Llm {
            self.config.translation.llm.debounce_ms
        } else {
            0
        };
        if delay_ms == 0 {
            self.do_translate();
        } else {
            let timer = unsafe { SetTimer(Some(self.hwnd), AUTO_TRANSLATE_TIMER, delay_ms, None) };
            if timer == 0 {
                tracing::warn!(
                    "auto-translate timer could not be created; dispatching immediately"
                );
                self.do_translate();
            }
        }
    }

    /// 현재 선택된 엔진/언어를 매니저에 적용
    fn apply_current_settings(&mut self) {
        // SAFETY: combo handles are valid controls from the resource template. SendMessageW with
        // CB_GETCURSEL returns the current selection index.
        unsafe {
            let engine_idx = SendMessageW(self.engine_combo, CB_GETCURSEL, None, None).0 as usize;
            let source_idx =
                SendMessageW(self.source_lang_combo, CB_GETCURSEL, None, None).0 as usize;
            let target_idx =
                SendMessageW(self.target_lang_combo, CB_GETCURSEL, None, None).0 as usize;

            let Some(engine) = engine_from_combo_index(engine_idx) else {
                tracing::error!("잘못된 번역 엔진 콤보 선택: {engine_idx}");
                return;
            };
            let supported_source = engine.supported_source_languages();
            let Some(source_lang) = supported_source.get(source_idx).copied() else {
                tracing::error!("잘못된 소스 언어 콤보 선택: {source_idx}");
                return;
            };
            let supported_target = engine.supported_targets_for(source_lang);
            let Some(target_lang) = supported_target.get(target_idx).copied() else {
                tracing::error!("잘못된 대상 언어 콤보 선택: {target_idx}");
                return;
            };

            let config = &mut self.config;
            if engine == TranslationEngine::Custom {
                let custom_index = engine_idx - CUSTOM_ENGINE_INDEX;
                let custom_name = if config.translation.custom_apis.is_empty() {
                    (custom_index == 0).then(|| config.translation.custom.name.clone())
                } else {
                    config
                        .translation
                        .custom_apis
                        .get(custom_index)
                        .map(|api| api.name.clone())
                };
                let Some(custom_name) = custom_name else {
                    tracing::error!("잘못된 Custom API 콤보 선택: {custom_index}");
                    return;
                };
                if let Err(error) = config.translation.select_custom_api(&custom_name) {
                    tracing::error!("Custom API 선택 실패: {error}");
                    return;
                }
            }
            config.translation.set_engine(engine);
            config.translation.set_source_language(source_lang);
            config.translation.set_target_language(target_lang);
        }
    }

    /// 번역 수행 (비동기)
    fn do_translate(&mut self) {
        let source = self.get_source_text();
        if source.is_empty() {
            return;
        }
        self.apply_current_settings();

        let text = self.manual_options.prepare_input(&source);

        let spec = {
            let config = &self.config;
            PreparedJob::from_config(&config.translation)
        };
        let spec = match spec {
            Ok(spec) => spec,
            Err(error) => {
                self.set_dest_text(&format!("[오류] {error}"));
                return;
            }
        };
        if let Err(error) = spec.prepare() {
            self.set_dest_text(&format!("[오류] {error}"));
            return;
        }

        self.set_dest_text("[번역 중...]");
        match self
            .translation_service
            .request(self.hwnd, Arc::from(text), spec)
        {
            Ok(req_id) => {
                self.in_flight_id = Some(req_id);
                self.last_submitted_source = source;
            }
            Err(error) => {
                self.in_flight_id = None;
                self.set_dest_text(&format!("[오류] {error}"));
            }
        }
    }

    /// 번역 완료 처리. WPARAM 의 `req_id` 로 자신의 응답만 꺼낸다.
    fn handle_translation_complete(&mut self) {
        let Some((req_id, response)) = self.translation_service.take_response(self.hwnd) else {
            return;
        };
        if self.in_flight_id != Some(req_id) {
            return;
        }
        if self.get_source_text() != self.last_submitted_source {
            self.in_flight_id = None;
            return;
        }
        self.in_flight_id = None;

        let result = match response.result {
            Ok(translated) => self.manual_options.format_output(translated),
            Err(err) => format!("[오류] {}", err),
        };
        self.set_dest_text(&result);
    }

    /// 번역 결과를 클립보드에 복사
    fn copy_to_clipboard(&self) {
        let text = get_window_text(self.dest_edit);
        if text.is_empty() {
            return;
        }
        Self::set_clipboard_text(&text, self.hwnd);
    }

    /// 클립보드에 텍스트 설정
    fn set_clipboard_text(text: &str, hwnd: HWND) {
        unsafe {
            let wide = to_wide(text);
            let byte_len = wide.len() * 2;

            let _clipboard = match ClipboardGuard::open(hwnd) {
                Ok(guard) => guard,
                Err(error) => {
                    tracing::warn!("OpenClipboard failed: {error}");
                    return;
                }
            };
            if let Err(e) = EmptyClipboard() {
                tracing::warn!("EmptyClipboard failed: {e}");
                return;
            }

            let mut memory = match OwnedGlobalMemory::allocate(byte_len) {
                Ok(memory) => memory,
                Err(error) => {
                    tracing::warn!("GlobalAlloc failed: {error}");
                    return;
                }
            };
            let handle = memory.handle();
            let ptr = GlobalLock(handle) as *mut u16;
            if ptr.is_null() {
                tracing::warn!("GlobalLock returned null");
                return;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(handle);
            match SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(handle.0))) {
                Ok(_) => memory.release_to_system(),
                Err(error) => tracing::warn!("SetClipboardData failed: {error}"),
            }
        }
    }

    /// 텍스트 초기화
    fn clear_text(&mut self) {
        self.invalidate_translation_route();
        // SAFETY: source_edit and dest_edit are valid edit control handles.
        unsafe {
            let _ = set_window_text(self.source_edit, "");
            let _ = set_window_text(self.dest_edit, "");
            let _ = SetFocus(Some(self.source_edit));
        }
    }

    fn invalidate_stale_translation(&mut self) {
        if self.in_flight_id.is_some() && self.get_source_text() != self.last_submitted_source {
            self.invalidate_translation_route();
        }
    }

    fn invalidate_translation_route(&mut self) {
        if self.in_flight_id.take().is_some() {
            self.translation_service.cancel(self.hwnd);
        }
        self.last_submitted_source.clear();
    }
}

fn engine_from_combo_index(index: usize) -> Option<TranslationEngine> {
    if index >= CUSTOM_ENGINE_INDEX {
        Some(TranslationEngine::Custom)
    } else {
        TranslationEngine::from_u8(index as u8)
    }
}
