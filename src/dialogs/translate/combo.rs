//! 언어/LLM 콤보박스 채우기와 LLM 설정 반영.

use windows_sys::Win32::{Foundation::HWND, UI::WindowsAndMessaging::*};

use super::TranslateDialog;
use crate::dialogs::helpers::{get_window_text, set_window_text};
use crate::translation::{Language, LlmProvider, TranslationEngine};
use crate::win32::to_wide;

impl TranslateDialog {
    /// 엔진에 맞게 언어 콤보박스 항목 갱신
    pub(super) fn populate_language_combos(&self, engine: TranslationEngine) {
        use crate::translation::lang_utils;
        // SAFETY: combo handles are valid controls obtained during initialization.
        unsafe {
            let _ = SendMessageW(self.source_lang_combo, CB_RESETCONTENT, 0, 0);
            for &lang in engine.supported_source_languages() {
                self.add_combobox_item(self.source_lang_combo, lang_utils::to_korean_name(lang));
            }
            let _ = SendMessageW(self.source_lang_combo, CB_SETCURSEL, 0, 0);

            let _ = SendMessageW(self.target_lang_combo, CB_RESETCONTENT, 0, 0);
            let source = engine
                .supported_source_languages()
                .first()
                .copied()
                .unwrap_or(Language::Jpn);
            self.populate_target_combo(engine, source);
        }
    }

    pub(super) fn populate_target_combo(&self, engine: TranslationEngine, source: Language) {
        unsafe {
            let _ = SendMessageW(self.target_lang_combo, CB_RESETCONTENT, 0, 0);
            for lang in engine.supported_targets_for(source) {
                self.add_combobox_item(
                    self.target_lang_combo,
                    crate::translation::lang_utils::to_korean_name(lang),
                );
            }
            let _ = SendMessageW(self.target_lang_combo, CB_SETCURSEL, 0, 0);
        }
    }

    /// 엔진이 LLM일 때만 LLM 그룹 노출.
    pub(super) fn update_llm_group_visibility(&self, engine: TranslationEngine) {
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

    pub(super) fn apply_llm_provider(&mut self) {
        // SAFETY: 콤보 핸들은 리소스 템플릿에서 얻은 유효한 핸들.
        let sel = unsafe { SendMessageW(self.llm_provider_combo, CB_GETCURSEL, 0, 0) as u8 };
        let Some(provider) = LlmProvider::from_u8(sel) else {
            return;
        };
        self.config.translation.llm.set_provider(provider);
        let configured_model = self.config.translation.llm.model.clone();
        let api_key = self.config.translation.llm.api_key.clone();
        self.populate_llm_model_combo(provider, &configured_model);
        let _ = set_window_text(self.llm_api_key_edit, &api_key);
    }

    pub(super) fn apply_llm_model(&mut self) {
        // SAFETY: edit 핸들은 리소스 템플릿에서 얻은 유효한 핸들.
        let text = get_window_text(self.llm_model_edit);
        self.config.translation.llm.model = text;
    }

    pub(super) fn apply_llm_api_key(&mut self) {
        // SAFETY: edit 핸들은 리소스 템플릿에서 얻은 유효한 핸들.
        let text = get_window_text(self.llm_api_key_edit);
        self.config.translation.llm.api_key = text;
    }

    /// 제공자별 모델 프리셋을 다시 채우고 현재 직접 입력값을 유지한다.
    pub(super) fn populate_llm_model_combo(&self, provider: LlmProvider, configured_model: &str) {
        unsafe {
            let _ = SendMessageW(self.llm_model_edit, CB_RESETCONTENT, 0, 0);
        }
        for model in provider.model_presets() {
            self.add_combobox_item(self.llm_model_edit, model);
        }
        let _ = set_window_text(
            self.llm_model_edit,
            provider.model_or_default(configured_model),
        );
    }

    pub(super) fn add_combobox_item(&self, combo: HWND, text: &str) {
        // SAFETY: combo is a valid combobox handle. wide string is valid for the call.
        unsafe {
            let wide = to_wide(text);
            let _ = SendMessageW(combo, CB_ADDSTRING, 0, wide.as_ptr() as isize);
        }
    }
}
