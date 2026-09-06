//! 컨트롤 바인딩, 초기값 반영, `WM_COMMAND` 처리.

use windows_core::{Error, HRESULT};
use windows_sys::Win32::{
    Foundation::*,
    UI::Controls::*,
    UI::Input::KeyboardAndMouse::*,
    UI::Shell::{DefSubclassProc, SetWindowSubclass},
    UI::WindowsAndMessaging::*,
};

use super::{AUTO_TRANSLATE_TIMER, Result, TranslateDialog, ctrl_id, subclass_id};
use crate::dialogs::helpers::set_window_text;
use crate::dialogs::translation_route::{
    CUSTOM_INDEX, ENGINES, engine_from_index, index_from_engine,
};
use crate::translation::manual::ManualOutputFormat;
use crate::translation::{LlmProvider, TranslationEngine};

/// [`TranslateDialog::resolve_initial_settings`]가 config에서 읽어온 초기 UI 상태.
struct InitialSettings {
    engine: TranslationEngine,
    engine_index: usize,
    source_index: usize,
    target_index: usize,
    provider_index: usize,
    model: String,
    api_key: String,
}

impl TranslateDialog {
    pub(super) fn initialize_controls(&mut self) -> Result<()> {
        self.bind_controls()?;
        self.configure_edit_controls();
        self.populate_engine_combo();
        self.populate_llm_provider_combo();
        let settings = self.resolve_initial_settings()?;
        self.apply_initial_settings(&settings)
    }

    /// 리소스에서 컨트롤 ID `id`의 핸들을 조회한다. 없으면 오류를 반환한다.
    fn get_control(&self, id: u16) -> Result<HWND> {
        // SAFETY: self.hwnd는 리소스 dialog로 생성된 유효한 창이다.
        let control = unsafe { GetDlgItem(self.hwnd, id as i32) };
        if control.is_null() {
            Err(Error::new(
                HRESULT(E_FAIL),
                format!("번역 컨트롤 ID {id}를 찾을 수 없습니다"),
            ))
        } else {
            Ok(control)
        }
    }

    /// 리소스 컨트롤 핸들을 모두 조회해 필드에 채운다.
    fn bind_controls(&mut self) -> Result<()> {
        self.source_edit = self.get_control(ctrl_id::SOURCE_EDIT)?;
        self.dest_edit = self.get_control(ctrl_id::DEST_EDIT)?;
        self.engine_combo = self.get_control(ctrl_id::COMBO_ENGINE)?;
        self.source_lang_combo = self.get_control(ctrl_id::COMBO_SOURCE_LANG)?;
        self.target_lang_combo = self.get_control(ctrl_id::COMBO_TARGET_LANG)?;
        self.llm_group = self.get_control(ctrl_id::LLM_GROUP)?;
        self.llm_provider_label = self.get_control(ctrl_id::LLM_PROVIDER_LABEL)?;
        self.llm_provider_combo = self.get_control(ctrl_id::COMBO_LLM_PROVIDER)?;
        self.llm_model_label = self.get_control(ctrl_id::LLM_MODEL_LABEL)?;
        self.llm_model_edit = self.get_control(ctrl_id::EDIT_LLM_MODEL)?;
        self.llm_api_key_label = self.get_control(ctrl_id::LLM_API_KEY_LABEL)?;
        self.llm_api_key_edit = self.get_control(ctrl_id::EDIT_LLM_API_KEY)?;
        self.verify_simple_controls()
    }

    /// 필드에 담지 않는 단순 버튼/체크박스/라디오 컨트롤의 존재만 검증한다.
    fn verify_simple_controls(&self) -> Result<()> {
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
            self.get_control(id)?;
        }
        Ok(())
    }

    /// 글자 수 제한 해제, Ctrl+A subclass, 기본 출력 형식 라디오 선택.
    fn configure_edit_controls(&self) {
        unsafe {
            let _ = SendMessageW(self.source_edit, EM_SETLIMITTEXT, 0, 0);
            let _ = SendMessageW(self.dest_edit, EM_SETLIMITTEXT, 0, 0);
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
    }

    /// 엔진 콤보에 내장 엔진과 Custom API 항목을 채운다.
    fn populate_engine_combo(&self) {
        for engine in &ENGINES[..CUSTOM_INDEX] {
            self.add_combobox_item(self.engine_combo, engine.display_name());
        }
        let config = &self.config;
        if config.translation.custom_apis.is_empty() {
            self.add_combobox_item(self.engine_combo, &config.translation.custom.name);
        } else {
            for api in &config.translation.custom_apis {
                self.add_combobox_item(self.engine_combo, &api.name);
            }
        }
        unsafe {
            let _ = SendMessageW(self.engine_combo, CB_SETDROPPEDWIDTH, 220, 0);
        }
    }

    fn populate_llm_provider_combo(&self) {
        for provider in LlmProvider::ALL {
            self.add_combobox_item(self.llm_provider_combo, provider.display_name());
        }
    }

    /// config에서 현재 엔진/언어/LLM 설정을 읽어 UI에 반영할 초기값을 만든다.
    fn resolve_initial_settings(&self) -> Result<InitialSettings> {
        let config = &self.config;
        let configured_engine = config
            .translation
            .get_engine()
            .map_err(|error| Error::new(HRESULT(E_INVALIDARG), error.to_string()))?;
        let engine = if configured_engine.requires_hook_session() {
            ENGINES[0]
        } else {
            configured_engine
        };
        let engine_index = if engine == TranslationEngine::Custom {
            CUSTOM_INDEX
                + config
                    .translation
                    .active_custom_api_index()
                    .map_err(|error| Error::new(HRESULT(E_INVALIDARG), error.to_string()))?
        } else {
            index_from_engine(engine)
                .ok_or_else(|| Error::new(HRESULT(E_INVALIDARG), "지원하지 않는 번역 엔진"))?
        };
        let provider_index = config
            .translation
            .llm
            .get_provider()
            .map_err(|error| Error::new(HRESULT(E_INVALIDARG), error.to_string()))?
            as u8 as usize;
        let source = config
            .translation
            .get_source_language()
            .map_err(|error| Error::new(HRESULT(E_INVALIDARG), error.to_string()))?;
        let target = config
            .translation
            .get_target_language()
            .map_err(|error| Error::new(HRESULT(E_INVALIDARG), error.to_string()))?;
        let source_index = engine
            .supported_source_languages()
            .iter()
            .position(|&language| language == source)
            .unwrap_or(0);
        let source = engine.supported_source_languages()[source_index];
        let target_index = engine
            .supported_targets_for(source)
            .iter()
            .position(|&language| language == target)
            .unwrap_or(0);

        Ok(InitialSettings {
            engine,
            engine_index,
            source_index,
            target_index,
            provider_index,
            model: config.translation.llm.model.clone(),
            api_key: config.translation.llm.api_key.clone(),
        })
    }

    /// [`resolve_initial_settings`]가 읽어온 값을 컨트롤에 반영한다.
    fn apply_initial_settings(&mut self, settings: &InitialSettings) -> Result<()> {
        let provider = LlmProvider::from_u8(settings.provider_index as u8)
            .ok_or_else(|| Error::new(HRESULT(E_INVALIDARG), "잘못된 LLM 제공자 설정"))?;
        self.populate_llm_model_combo(provider, &settings.model);

        unsafe {
            let _ = SendMessageW(self.engine_combo, CB_SETCURSEL, settings.engine_index, 0);
        }
        self.populate_language_combos(settings.engine);
        let selected_source = settings
            .engine
            .supported_source_languages()
            .get(settings.source_index)
            .copied()
            .ok_or_else(|| Error::new(HRESULT(E_INVALIDARG), "잘못된 소스 언어 설정"))?;
        unsafe {
            let _ = SendMessageW(
                self.source_lang_combo,
                CB_SETCURSEL,
                settings.source_index,
                0,
            );
            self.populate_target_combo(settings.engine, selected_source);
            let _ = SendMessageW(
                self.target_lang_combo,
                CB_SETCURSEL,
                settings.target_index,
                0,
            );
            let _ = SendMessageW(
                self.llm_provider_combo,
                CB_SETCURSEL,
                settings.provider_index,
                0,
            );
            let _ = set_window_text(self.llm_api_key_edit, &settings.api_key);
        }
        self.update_llm_group_visibility(settings.engine);
        Ok(())
    }

    /// 명령 처리
    pub(super) fn handle_command(&mut self, cmd: u16, notify_code: u32) {
        use ctrl_id::*;

        match cmd {
            BTN_TRANSLATE => {
                unsafe {
                    let _ = KillTimer(self.hwnd, AUTO_TRANSLATE_TIMER);
                }
                self.do_translate();
            }
            BTN_COPY => self.copy_to_clipboard(),
            BTN_CLEAR => self.clear_text(),
            CHK_ONE_GO => {
                self.one_go = !self.one_go;
                if !self.one_go {
                    unsafe {
                        let _ = KillTimer(self.hwnd, AUTO_TRANSLATE_TIMER);
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
                self.handle_language_combo_change(cmd);
            }
            COMBO_LLM_PROVIDER if notify_code == 1 => {
                self.invalidate_translation_route();
                self.apply_llm_provider();
            }
            EDIT_LLM_MODEL if notify_code == CBN_SELCHANGE || notify_code == CBN_EDITCHANGE => {
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

    /// 엔진/소스 언어 콤보 변경 시 언어 콤보를 다시 채우고 현재 설정에 반영한다.
    fn handle_language_combo_change(&mut self, cmd: u16) {
        self.invalidate_translation_route();
        if cmd == ctrl_id::COMBO_ENGINE {
            // SAFETY: engine_combo is a valid handle.
            let engine_idx = unsafe { SendMessageW(self.engine_combo, CB_GETCURSEL, 0, 0) as u8 };
            let Some(engine) = engine_from_index(engine_idx as usize) else {
                return;
            };
            self.populate_language_combos(engine);
            self.update_llm_group_visibility(engine);
        } else if cmd == ctrl_id::COMBO_SOURCE_LANG {
            let engine_idx = unsafe { SendMessageW(self.engine_combo, CB_GETCURSEL, 0, 0) as u8 };
            let source_idx =
                unsafe { SendMessageW(self.source_lang_combo, CB_GETCURSEL, 0, 0) as usize };
            let Some(engine) = engine_from_index(engine_idx as usize) else {
                return;
            };
            if let Some(&source) = engine.supported_source_languages().get(source_idx) {
                self.populate_target_combo(engine, source);
            }
        }
        self.apply_current_settings();
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
                && wparam == 'A' as usize
                && (GetKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0
            {
                let _ = SendMessageW(hwnd, EM_SETSEL, 0, -1);
                return 0;
            }

            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
    }
}
