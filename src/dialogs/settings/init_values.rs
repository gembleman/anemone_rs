//! `initialize_values`: 저장된 설정을 각 컨트롤의 초기값으로 반영한다.
//!
//! 컨트롤 ID/설정 필드 쌍이 반복되는 구간(트랙바+편집란, 체크박스)은 표를 순회해
//! 채우고, 나머지는 탭/엔진 구획별 헬퍼로 나눠 각 함수의 분기 수를 낮게 유지한다.

use super::*;
use crate::config::TextAlign;
use crate::translation::settings::TranslationSettingsEditor;
use secret_format::format_deepl_key;
use windows_core::{Error, HRESULT};

impl SettingsDialog {
    /// 리소스에 정의된 컨트롤을 저장된 설정값으로 초기화한다.
    pub(super) fn initialize_values(&self) -> Result<()> {
        let config = self.draft.borrow();
        self.initialize_numeric_controls(&config)?;
        self.initialize_checkbox_controls(&config)?;
        self.initialize_translation_engine_selection(&config)?;
        self.initialize_eztrans_values(&config)?;
        self.initialize_deepl_values(&config)?;
        self.initialize_papago_values(&config)?;
        self.initialize_mys_translater_values(&config)?;
        self.initialize_llm_values(&config)?;
        self.initialize_llm_advanced_values(&config)
    }

    /// 트랙바 + 편집란 쌍은 (트랙바 id, 편집란 id, 설정) 표가 하나뿐이다.
    /// `handlers::NUMERIC_CONTROL_BINDINGS`가 그 단일 출처다.
    fn initialize_numeric_controls(&self, config: &Config) -> Result<()> {
        for binding in handlers::NUMERIC_CONTROL_BINDINGS {
            let (min, max) = handlers::numeric_setting_range(binding.setting);
            let value = handlers::numeric_setting_value(config, binding.setting);
            self.initialize_trackbar(binding.trackbar_id, min, max, value)?;
            self.set_text(binding.edit_id, &value.to_string())?;
        }
        Ok(())
    }

    /// 단순 on/off 체크박스(정렬 라디오 포함)는 (id, 값) 쌍의 표로 채운다.
    fn initialize_checkbox_controls(&self, config: &Config) -> Result<()> {
        let checks: [(u16, bool); 19] = [
            (ctrl_id::BACKGROUND_SWITCH, config.background_visible),
            (ctrl_id::NAME_SHADOW, config.name_style.shadow_enabled),
            (ctrl_id::ORG_SHADOW, config.original_style.shadow_enabled),
            (
                ctrl_id::TRANS_SHADOW,
                config.translation_style.shadow_enabled,
            ),
            (ctrl_id::BORDER_MODE, config.border_visible),
            (ctrl_id::PRINT_ORGTEXT, config.show_original),
            (ctrl_id::PRINT_TRANSTEXT, config.show_translation),
            (ctrl_id::PRINT_ORGNAME, config.show_name),
            (ctrl_id::SEPERATE_NAME, config.separate_name),
            (
                ctrl_id::TEXTALIGN_LEFT,
                config.text_align == TextAlign::Left,
            ),
            (
                ctrl_id::TEXTALIGN_MID,
                config.text_align == TextAlign::Center,
            ),
            (
                ctrl_id::TEXTALIGN_RIGHT,
                config.text_align == TextAlign::Right,
            ),
            (ctrl_id::TOPMOST, config.window_topmost),
            (ctrl_id::USE_MAGNETIC, config.magnetic_mode),
            (ctrl_id::MAGNETIC_MINIMIZE, config.magnetic_minimize),
            (ctrl_id::CLIPBOARD_WATCH, config.clipboard_watch),
            (ctrl_id::WNDCLICK_THROUGH, config.click_through),
            (
                ctrl_id::CLIPBOARD_CACHE_ENABLED,
                config.clipboard_cache_enabled,
            ),
            (
                ctrl_id::CLIPBOARD_SOURCE_LANG_GUARD,
                config.clipboard_source_language_guard,
            ),
        ];
        for (id, checked) in checks {
            self.set_checked(id, checked)?;
        }
        Ok(())
    }

    fn initialize_translation_engine_selection(&self, config: &Config) -> Result<()> {
        let engine_names: Vec<&str> = crate::translation::TranslationEngine::ALL
            .iter()
            .map(|engine| engine.display_name())
            .collect();
        let engine = config
            .translation
            .get_engine()
            .map_err(|error| Error::new(HRESULT(E_INVALIDARG), error.to_string()))?;
        self.initialize_combo(ctrl_id::TRANS_ENGINE, &engine_names, engine as usize)?;

        let custom_names: Vec<&str> = if config.translation.custom_apis.is_empty() {
            vec!["없음"]
        } else {
            config
                .translation
                .custom_apis
                .iter()
                .map(|api| api.name.as_str())
                .collect()
        };
        let custom_index = if config.translation.custom_apis.is_empty() {
            0
        } else {
            config
                .translation
                .active_custom_api_index()
                .map_err(|error| Error::new(HRESULT(E_INVALIDARG), error.to_string()))?
        };
        self.initialize_combo(ctrl_id::CUSTOM_API_SELECT, &custom_names, custom_index)?;
        unsafe {
            let _ = SendMessageW(
                self.control(ctrl_id::TRANS_ENGINE)?,
                CB_SETDROPPEDWIDTH,
                220,
                0,
            );
        }
        Ok(())
    }

    /// 저장된 경로를 쓸 수 없으면 편집란을 비운 채로 연다. (설정 값 자체는 보존한다.)
    /// 입력란이 비어 있으므로 경고 라벨도 함께 비워 둔다.
    fn initialize_eztrans_values(&self, config: &Config) -> Result<()> {
        let dictionary_path = &config.translation.eztrans_dictionary_path;
        let dictionary_display =
            if TranslationSettingsEditor::eztrans_dictionary_invalid(dictionary_path) {
                ""
            } else {
                dictionary_path.as_str()
            };
        self.set_text(ctrl_id::EZTRANS_DICTIONARY_EDIT, dictionary_display)?;
        self.set_text(ctrl_id::EZTRANS_DICTIONARY_WARNING_LABEL, "")?;

        let ehnd_path = &config.translation.eztrans_ehnd_path;
        let ehnd_display = if TranslationSettingsEditor::eztrans_ehnd_invalid(ehnd_path) {
            ""
        } else {
            ehnd_path.as_str()
        };
        self.set_text(ctrl_id::EZTRANS_EHND_EDIT, ehnd_display)?;
        self.set_text(ctrl_id::EZTRANS_EHND_WARNING_LABEL, "")?;
        self.set_text(
            ctrl_id::EZTRANS_DICTIONARY_COUNT_LABEL,
            &format!(
                "후처리 사전: {}",
                config.translation.eztrans_postprocess_dictionary.len()
            ),
        )
    }

    fn initialize_deepl_values(&self, config: &Config) -> Result<()> {
        let strategy = match config.translation.deepl_strategy.to_lowercase().as_str() {
            "round-robin" | "roundrobin" | "rr" => 1,
            _ => 0,
        };
        self.initialize_combo(
            ctrl_id::DEEPL_STRATEGY_COMBO,
            &["failover", "round-robin"],
            strategy,
        )?;
        self.initialize_combo(ctrl_id::DEEPL_KEY_TIER_COMBO, &["무료", "유료"], 0)?;
        let key_list = self.control(ctrl_id::DEEPL_KEYS_LIST)?;
        for key in &config.translation.deepl_keys {
            let wide = to_wide(&format_deepl_key(key));
            unsafe {
                let _ = SendMessageW(key_list, LB_ADDSTRING, 0, wide.as_ptr() as isize);
            }
        }
        Ok(())
    }

    fn initialize_papago_values(&self, config: &Config) -> Result<()> {
        self.set_text(
            ctrl_id::PAPAGO_ID_EDIT,
            &config.translation.papago_client_id,
        )?;
        self.set_text(
            ctrl_id::PAPAGO_SECRET_EDIT,
            &config.translation.papago_client_secret,
        )
    }

    /// 서버 URL도 API 토큰도 입력란이 없다 — URL은 config.toml의 값을 그대로
    /// 쓰고, 토큰은 앱이 받아서 암호화해 보관한다. 화면에는 보유 여부만 띄운다.
    fn initialize_mys_translater_values(&self, _config: &Config) -> Result<()> {
        self.refresh_mys_token_status();
        self.refresh_mys_usage();
        Ok(())
    }

    fn initialize_llm_values(&self, config: &Config) -> Result<()> {
        let provider = config
            .translation
            .llm
            .get_provider()
            .map_err(|error| Error::new(HRESULT(E_INVALIDARG), error.to_string()))?;
        let providers: Vec<&str> = crate::translation::LlmProvider::ALL
            .iter()
            .map(|provider| provider.display_name())
            .collect();
        self.initialize_combo(ctrl_id::LLM_PROVIDER, &providers, provider as u8 as usize)?;
        self.populate_llm_model_combo(provider, &config.translation.llm.model)?;
        self.set_text(ctrl_id::LLM_API_KEY_EDIT, &config.translation.llm.api_key)?;
        self.set_checked(ctrl_id::LLM_API_KEY_VISIBLE, false)?;
        self.set_text(
            ctrl_id::LLM_SYSTEM_PROMPT_EDIT,
            &config.translation.llm.system_prompt,
        )
    }

    fn initialize_llm_advanced_values(&self, config: &Config) -> Result<()> {
        let temperature = config.translation.llm.temperature;
        self.initialize_trackbar(
            ctrl_id::LLM_TEMPERATURE_TRACKBAR,
            crate::config::limits::LLM_TEMPERATURE_SLIDER_MIN,
            crate::config::limits::LLM_TEMPERATURE_SLIDER_MAX,
            crate::config::limits::llm_temperature_to_slider(temperature),
        )?;
        self.set_text(ctrl_id::LLM_TEMPERATURE_EDIT, &format!("{temperature:.2}"))?;
        self.set_text(
            ctrl_id::LLM_MAX_TOKENS_EDIT,
            &config.translation.llm.max_tokens.to_string(),
        )?;
        let mut reasoning_efforts = vec!["기본값"];
        reasoning_efforts.extend(
            crate::translation::llm::ReasoningEffort::ALL
                .iter()
                .map(|effort| effort.to_str()),
        );
        let reasoning_effort_index = config
            .translation
            .llm
            .reasoning_effort
            .and_then(|configured| {
                crate::translation::llm::ReasoningEffort::ALL
                    .iter()
                    .position(|&effort| effort == configured)
            })
            .map_or(0, |index| index + 1);
        self.initialize_combo(
            ctrl_id::LLM_REASONING_EFFORT,
            &reasoning_efforts,
            reasoning_effort_index,
        )?;
        self.set_text(
            ctrl_id::LLM_DEBOUNCE_EDIT,
            &config.translation.llm.debounce_ms.to_string(),
        )?;
        self.set_text(
            ctrl_id::LLM_GLOSSARY_COUNT_LABEL,
            &format!("사전 항목: {}", config.translation.llm.glossary.len()),
        )
    }
}
