//! 트랙바·콤보박스·edit killfocus 등 값 입력 컨트롤의 실시간 반영.

use windows_sys::Win32::{UI::Controls::*, UI::WindowsAndMessaging::*};

use super::handlers::{
    numeric_binding_for_edit, numeric_binding_for_trackbar, numeric_setting_value,
};
use super::model::SettingsChange;
use super::{SettingsDialog, ctrl_id};
use crate::translation::settings::TranslationSettingChange;

impl SettingsDialog {
    /// 트랙바 변경 처리
    pub(super) fn handle_trackbar(&mut self, id: u16, value: i32) {
        use ctrl_id::*;

        if id == LLM_TEMPERATURE_TRACKBAR {
            let _ = self
                .apply_translation_change(TranslationSettingChange::LlmTemperatureSlider(value));
            let temp = self.draft.borrow().translation.llm.temperature;
            self.set_control_text(LLM_TEMPERATURE_EDIT, &format!("{temp:.2}"));
            return;
        }

        let Some(binding) = numeric_binding_for_trackbar(id) else {
            return;
        };
        self.apply_settings_change(SettingsChange::Numeric {
            setting: binding.setting,
            value,
        });
        let actual = numeric_setting_value(&self.draft.borrow(), binding.setting);
        self.set_control_text(binding.edit_id, &actual.to_string());
    }

    /// ComboBox 선택 변경 처리
    pub(super) fn handle_combobox(&mut self, id: u16) {
        use ctrl_id::*;

        // SAFETY: self.hwnd is valid; GetDlgItem and SendMessageW use valid handles.
        let sel = unsafe {
            let combo = GetDlgItem(self.hwnd, id as i32);
            if combo.is_null() {
                return;
            }
            SendMessageW(combo, CB_GETCURSEL, 0, 0) as usize
        };

        match id {
            TRANS_ENGINE => self.handle_trans_engine_combo(sel),
            CUSTOM_API_SELECT => self.handle_custom_api_select_combo(sel),
            TRANS_SOURCE_LANG => self.handle_trans_source_lang_combo(sel),
            TRANS_TARGET_LANG => self.handle_trans_target_lang_combo(sel),
            LLM_PROVIDER => self.handle_llm_provider_combo(sel),
            LLM_MODEL_EDIT => self.handle_llm_model_edit_combo(),
            LLM_REASONING_EFFORT => self.handle_llm_reasoning_effort_combo(sel),
            DEEPL_STRATEGY_COMBO => self.handle_deepl_strategy_combo(sel),
            _ => {}
        }
    }

    /// 번역 엔진 선택 처리: 설정 반영, 엔진별 컨트롤 그룹 전환, 필요 시 후킹 안내.
    fn handle_trans_engine_combo(&mut self, sel: usize) {
        use crate::translation::TranslationEngine;
        let Some(engine) = TranslationEngine::from_u8(sel as u8) else {
            return;
        };
        if self
            .apply_translation_change(TranslationSettingChange::Engine(engine))
            .is_err()
        {
            return;
        }
        // 엔진 변경 시 해당 그룹만 활성화하고 언어 콤보 항목 재구성
        self.apply_engine_state(engine);
        if engine.requires_hook_session() && !crate::hook::session_active() {
            self.show_mys_translater_hook_notice();
        }
    }

    /// 이름 있는 사용자 정의 API 목록에서 선택 처리.
    fn handle_custom_api_select_combo(&mut self, sel: usize) {
        let custom_name = {
            let config = self.draft.borrow();
            if config.translation.custom_apis.is_empty() {
                None
            } else {
                config
                    .translation
                    .custom_apis
                    .get(sel)
                    .map(|api| api.name.clone())
            }
        };
        let Some(custom_name) = custom_name else {
            return;
        };
        let _ =
            self.apply_translation_change(TranslationSettingChange::SelectCustomApi(custom_name));
    }

    /// 원문 언어 선택 처리: 값 반영 후 대상 언어 콤보 항목을 재구성한다.
    fn handle_trans_source_lang_combo(&mut self, sel: usize) {
        let Ok(engine) = self.draft.borrow().translation.get_engine() else {
            return;
        };
        if let Some(&language) = engine.supported_source_languages().get(sel)
            && self
                .apply_translation_change(TranslationSettingChange::SourceLanguage(language))
                .is_ok()
        {
            self.refresh_language_combos(engine);
        }
    }

    /// 번역 대상 언어 선택 처리.
    fn handle_trans_target_lang_combo(&mut self, sel: usize) {
        let Ok(engine) = self.draft.borrow().translation.get_engine() else {
            return;
        };
        let Ok(source) = self.draft.borrow().translation.get_source_language() else {
            return;
        };
        let targets = engine.supported_targets_for(source);
        if let Some(&language) = targets.get(sel) {
            let _ =
                self.apply_translation_change(TranslationSettingChange::TargetLanguage(language));
        }
    }

    /// LLM 제공자 선택 처리: 값 반영 후 제공자별 UI(모델/기본 URL 등)를 갱신한다.
    fn handle_llm_provider_combo(&mut self, sel: usize) {
        use crate::translation::LlmProvider;
        let Some(provider) = LlmProvider::from_u8(sel as u8) else {
            return;
        };
        let _ = self.apply_translation_change(TranslationSettingChange::LlmProvider(provider));
        if let Err(error) = self.refresh_llm_provider_controls(provider) {
            tracing::warn!("LLM 제공자 설정 UI를 갱신할 수 없습니다: {error}");
        }
    }

    /// LLM 모델 콤보 편집란의 현재 텍스트를 설정에 반영한다.
    fn handle_llm_model_edit_combo(&mut self) {
        let model = self.get_control_text(ctrl_id::LLM_MODEL_EDIT);
        let _ = self.apply_translation_change(TranslationSettingChange::LlmModel(model));
    }

    /// LLM reasoning effort 선택 처리 (index 0은 "미지정").
    fn handle_llm_reasoning_effort_combo(&mut self, sel: usize) {
        let effort = sel.checked_sub(1).and_then(|index| {
            crate::translation::llm::ReasoningEffort::ALL
                .get(index)
                .copied()
        });
        let _ = self.apply_translation_change(TranslationSettingChange::LlmReasoningEffort(effort));
    }

    /// DeepL 라운드로빈 전략 선택 처리.
    fn handle_deepl_strategy_combo(&mut self, sel: usize) {
        let _ = self
            .apply_translation_change(TranslationSettingChange::DeepLStrategyRoundRobin(sel == 1));
    }

    fn show_mys_translater_hook_notice(&self) {
        // SAFETY: self.hwnd는 살아 있는 설정 대화상자다.
        unsafe {
            let _ = MessageBoxW(
                self.hwnd,
                crate::win32::to_wide("MyS Translater는 게임 텍스트를 후킹해 번역하는 엔진입니다.\n먼저 메뉴의 '후킹 관리…'에서 게임을 후킹해 주세요.").as_ptr(),
                crate::win32::to_wide("MyS Translater").as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }

    /// Edit 컨트롤 포커스 해제 시 값 저장
    pub(super) fn handle_edit_killfocus(&mut self, ctrl_id: u16) {
        let text = self.get_control_text(ctrl_id);
        if let Some(binding) = numeric_binding_for_edit(ctrl_id) {
            if let Ok(value) = text.trim().parse::<i32>() {
                self.apply_settings_change(SettingsChange::Numeric {
                    setting: binding.setting,
                    value,
                });
            }
            let actual = numeric_setting_value(&self.draft.borrow(), binding.setting);
            self.update_numeric_ui(binding.trackbar_id, binding.edit_id, actual);
            return;
        }

        use ctrl_id::*;
        // EzTrans 경로 편집란은 읽기 전용이라 "찾아보기" 결과만 담긴다.
        // 포커스 해제로 다시 저장할 사용자 입력이 없으므로 여기서 다루지 않는다.
        let change = match ctrl_id {
            PAPAGO_ID_EDIT => TranslationSettingChange::PapagoClientId(text),
            PAPAGO_SECRET_EDIT => TranslationSettingChange::PapagoClientSecret(text),
            LLM_MODEL_EDIT => TranslationSettingChange::LlmModel(text),
            LLM_API_KEY_EDIT => TranslationSettingChange::LlmApiKey(text),
            LLM_SYSTEM_PROMPT_EDIT => TranslationSettingChange::LlmSystemPrompt(text),
            LLM_MAX_TOKENS_EDIT => TranslationSettingChange::LlmMaxTokensText(text),
            LLM_TEMPERATURE_EDIT => TranslationSettingChange::LlmTemperatureText(text),
            LLM_DEBOUNCE_EDIT => TranslationSettingChange::LlmDebounceText(text),
            _ => return,
        };
        if let Err(error) = self.apply_translation_change(change) {
            tracing::warn!("번역 설정 입력을 적용할 수 없습니다: {error}");
            match ctrl_id {
                LLM_MAX_TOKENS_EDIT => {
                    let value = self.draft.borrow().translation.llm.max_tokens;
                    self.set_control_text(ctrl_id, &value.to_string());
                }
                LLM_TEMPERATURE_EDIT => {
                    let value = self.draft.borrow().translation.llm.temperature;
                    self.set_control_text(ctrl_id, &format!("{value:.2}"));
                }
                LLM_DEBOUNCE_EDIT => {
                    let value = self.draft.borrow().translation.llm.debounce_ms;
                    self.set_control_text(ctrl_id, &value.to_string());
                }
                _ => {}
            }
        } else if ctrl_id == LLM_TEMPERATURE_EDIT {
            let value = self.draft.borrow().translation.llm.temperature;
            self.update_trackbar_pos(
                LLM_TEMPERATURE_TRACKBAR,
                crate::config::limits::llm_temperature_to_slider(value),
            );
            self.set_control_text(ctrl_id, &format!("{value:.2}"));
        }
    }

    /// 트랙바 위치 업데이트
    fn update_trackbar_pos(&self, trackbar_id: u16, value: i32) {
        // SAFETY: self.hwnd is valid; GetDlgItem returns a valid control handle.
        unsafe {
            let trackbar = GetDlgItem(self.hwnd, trackbar_id as i32);
            if !trackbar.is_null() {
                let _ = SendMessageW(trackbar, TBM_SETPOS, 1, value as isize);
            }
        }
    }

    fn update_numeric_ui(&self, trackbar_id: u16, edit_id: u16, value: i32) {
        self.update_trackbar_pos(trackbar_id, value);
        self.set_control_text(edit_id, &value.to_string());
    }
}
