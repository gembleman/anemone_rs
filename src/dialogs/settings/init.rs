//! DIALOGEX 리소스에서 생성된 설정 컨트롤의 런타임 초기화.

use super::*;

pub(super) const APPEARANCE_IDS: &[u16] = &[
    ctrl_id::BACKGROUND_TRACKBAR,
    ctrl_id::BACKGROUND_COLOR,
    ctrl_id::BACKGROUND_SWITCH,
    ctrl_id::BACKGROUND_EDIT,
    ctrl_id::TEXTSIZE_TRACKBAR,
    ctrl_id::TEXTSIZE_EDIT,
    ctrl_id::OUTLINE1_TRACKBAR,
    ctrl_id::OUTLINE1_EDIT,
    ctrl_id::OUTLINE2_TRACKBAR,
    ctrl_id::OUTLINE2_EDIT,
    ctrl_id::SHADOW_X_TRACKBAR,
    ctrl_id::SHADOW_X_EDIT,
    ctrl_id::SHADOW_Y_TRACKBAR,
    ctrl_id::SHADOW_Y_EDIT,
    ctrl_id::NAME_COLOR,
    ctrl_id::NAME_OUTLINE1,
    ctrl_id::NAME_OUTLINE2,
    ctrl_id::NAME_SHADOW_COLOR,
    ctrl_id::NAME_FONT,
    ctrl_id::NAME_SHADOW,
    ctrl_id::ORG_COLOR,
    ctrl_id::ORG_OUTLINE1,
    ctrl_id::ORG_OUTLINE2,
    ctrl_id::ORG_SHADOW_COLOR,
    ctrl_id::ORG_FONT,
    ctrl_id::ORG_SHADOW,
    ctrl_id::TRANS_COLOR,
    ctrl_id::TRANS_OUTLINE1,
    ctrl_id::TRANS_OUTLINE2,
    ctrl_id::TRANS_SHADOW_COLOR,
    ctrl_id::TRANS_FONT,
    ctrl_id::TRANS_SHADOW,
    ctrl_id::MARGIN_X_TRACKBAR,
    ctrl_id::MARGIN_X_EDIT,
    ctrl_id::MARGIN_Y_TRACKBAR,
    ctrl_id::MARGIN_Y_EDIT,
    ctrl_id::MARGIN_NAME_TRACKBAR,
    ctrl_id::MARGIN_NAME_EDIT,
    ctrl_id::BORDER_MODE,
    ctrl_id::BORDER_COLOR,
    ctrl_id::BORDER_SIZE_TRACKBAR,
    ctrl_id::BORDER_SIZE_EDIT,
];

pub(super) const DISPLAY_IDS: &[u16] = &[
    ctrl_id::PRINT_ORGTEXT,
    ctrl_id::PRINT_TRANSTEXT,
    ctrl_id::PRINT_ORGNAME,
    ctrl_id::SEPERATE_NAME,
    ctrl_id::TEXTALIGN_LEFT,
    ctrl_id::TEXTALIGN_MID,
    ctrl_id::TEXTALIGN_RIGHT,
    ctrl_id::TOPMOST,
    ctrl_id::USE_MAGNETIC,
    ctrl_id::MAGNETIC_MINIMIZE,
    ctrl_id::CLIPBOARD_WATCH,
    ctrl_id::WNDCLICK_THROUGH,
    ctrl_id::CLIPBOARD_CACHE_ENABLED,
    ctrl_id::CLIPBOARD_CACHE_CLEAR,
];

pub(super) const TRANSLATION_IDS: &[u16] = &[
    ctrl_id::TRANS_ENGINE,
    ctrl_id::TRANS_SOURCE_LANG,
    ctrl_id::TRANS_TARGET_LANG,
    ctrl_id::EZTRANS_DLL_EDIT,
    ctrl_id::EZTRANS_DLL_BROWSE,
    ctrl_id::EZTRANS_DAT_EDIT,
    ctrl_id::EZTRANS_DAT_BROWSE,
    ctrl_id::EZTRANS_DICTIONARY_EDIT_BTN,
    ctrl_id::DEEPL_KEYS_LIST,
    ctrl_id::DEEPL_KEY_ADD_EDIT,
    ctrl_id::DEEPL_KEY_ADD_BTN,
    ctrl_id::DEEPL_KEY_REMOVE_BTN,
    ctrl_id::DEEPL_STRATEGY_COMBO,
    ctrl_id::DEEPL_KEY_TIER_COMBO,
    ctrl_id::PAPAGO_ID_EDIT,
    ctrl_id::PAPAGO_SECRET_EDIT,
    ctrl_id::LLM_PROVIDER,
    ctrl_id::LLM_MODEL_EDIT,
    ctrl_id::LLM_API_KEY_EDIT,
    ctrl_id::LLM_API_KEY_VISIBLE,
    ctrl_id::LLM_REASONING_EFFORT,
    ctrl_id::LLM_SYSTEM_PROMPT_EDIT,
    ctrl_id::LLM_MAX_TOKENS_EDIT,
    ctrl_id::LLM_TEMPERATURE_TRACKBAR,
    ctrl_id::LLM_TEMPERATURE_EDIT,
    ctrl_id::LLM_DEBOUNCE_EDIT,
    ctrl_id::LLM_GLOSSARY_EDIT_BTN,
    ctrl_id::LLM_GLOSSARY_COUNT_LABEL,
    ctrl_id::CUSTOM_API_SELECT,
];

pub(super) const HOTKEYS_IDS: &[u16] = &[ctrl_id::HOTKEYS_LIST, ctrl_id::HOTKEYS_RESET];
pub(super) const INFO_IDS: &[u16] = &[ctrl_id::APP_VERSION];

impl SettingsDialog {
    /// 리소스에 정의된 컨트롤을 탭/엔진 그룹에 연결하고 설정값을 주입한다.
    pub(super) fn initialize_controls(&mut self) -> Result<()> {
        self.register_ids(TAB_APPEARANCE, ctrl_id::APPEARANCE_STATIC_IDS)?;
        self.register_ids(TAB_APPEARANCE, APPEARANCE_IDS)?;
        self.register_ids(TAB_DISPLAY, ctrl_id::DISPLAY_STATIC_IDS)?;
        self.register_ids(TAB_DISPLAY, DISPLAY_IDS)?;
        self.register_ids(TAB_TRANSLATION, ctrl_id::TRANSLATION_STATIC_IDS)?;
        self.register_ids(TAB_TRANSLATION, TRANSLATION_IDS)?;
        self.register_ids(TAB_HOTKEYS, ctrl_id::HOTKEYS_STATIC_IDS)?;
        self.register_ids(TAB_HOTKEYS, HOTKEYS_IDS)?;
        self.register_ids(TAB_INFO, ctrl_id::INFO_STATIC_IDS)?;
        self.register_ids(TAB_INFO, INFO_IDS)?;
        self.register_engine_controls()?;
        self.initialize_tab_titles()?;
        self.initialize_values()?;
        self.initialize_info()?;
        self.initialize_hotkey_list()?;
        for &hwnd in &self.tab_controls[TAB_DISPLAY] {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
        for &hwnd in &self.tab_controls[TAB_TRANSLATION] {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
        for &hwnd in &self.tab_controls[TAB_HOTKEYS] {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
        for &hwnd in &self.tab_controls[TAB_INFO] {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }

        self.adjust_dialog_size_for_tab(TAB_APPEARANCE);
        let engine = self
            .draft
            .borrow()
            .translation
            .get_engine()
            .map_err(|error| Error::new(E_INVALIDARG, error.to_string()))?;
        self.apply_engine_state(engine);
        Ok(())
    }

    fn register_engine_controls(&mut self) -> Result<()> {
        self.register_engine_ids(EngineGroup::EzTrans, ctrl_id::EZTRANS_STATIC_IDS)?;
        self.register_engine_ids(
            EngineGroup::EzTrans,
            &[
                ctrl_id::EZTRANS_DLL_EDIT,
                ctrl_id::EZTRANS_DLL_BROWSE,
                ctrl_id::EZTRANS_DAT_EDIT,
                ctrl_id::EZTRANS_DAT_BROWSE,
                ctrl_id::EZTRANS_DICTIONARY_EDIT_BTN,
            ],
        )?;
        self.register_engine_ids(EngineGroup::DeepL, ctrl_id::DEEPL_STATIC_IDS)?;
        self.register_engine_ids(
            EngineGroup::DeepL,
            &[
                ctrl_id::DEEPL_KEYS_LIST,
                ctrl_id::DEEPL_KEY_ADD_EDIT,
                ctrl_id::DEEPL_KEY_ADD_BTN,
                ctrl_id::DEEPL_KEY_REMOVE_BTN,
                ctrl_id::DEEPL_STRATEGY_COMBO,
                ctrl_id::DEEPL_KEY_TIER_COMBO,
            ],
        )?;
        self.register_engine_ids(EngineGroup::Papago, ctrl_id::PAPAGO_STATIC_IDS)?;
        self.register_engine_ids(
            EngineGroup::Papago,
            &[ctrl_id::PAPAGO_ID_EDIT, ctrl_id::PAPAGO_SECRET_EDIT],
        )?;
        self.register_engine_ids(EngineGroup::Llm, ctrl_id::LLM_STATIC_IDS)?;
        self.register_engine_ids(
            EngineGroup::Llm,
            &[
                ctrl_id::LLM_PROVIDER,
                ctrl_id::LLM_MODEL_EDIT,
                ctrl_id::LLM_API_KEY_EDIT,
                ctrl_id::LLM_API_KEY_VISIBLE,
                ctrl_id::LLM_REASONING_EFFORT,
                ctrl_id::LLM_SYSTEM_PROMPT_EDIT,
                ctrl_id::LLM_MAX_TOKENS_EDIT,
                ctrl_id::LLM_TEMPERATURE_TRACKBAR,
                ctrl_id::LLM_TEMPERATURE_EDIT,
                ctrl_id::LLM_DEBOUNCE_EDIT,
                ctrl_id::LLM_GLOSSARY_EDIT_BTN,
                ctrl_id::LLM_GLOSSARY_COUNT_LABEL,
            ],
        )?;
        self.register_engine_ids(EngineGroup::Custom, ctrl_id::CUSTOM_STATIC_IDS)?;
        self.register_engine_ids(EngineGroup::Custom, &[ctrl_id::CUSTOM_API_SELECT])
    }

    fn initialize_values(&self) -> Result<()> {
        let config = self.draft.borrow();

        self.initialize_trackbar(
            ctrl_id::BACKGROUND_TRACKBAR,
            0,
            255,
            ((config.background_color >> 24) & 0xff) as i32,
        )?;
        self.set_text(
            ctrl_id::BACKGROUND_EDIT,
            &((config.background_color >> 24) & 0xff).to_string(),
        )?;
        self.set_checked(ctrl_id::BACKGROUND_SWITCH, config.background_visible)?;
        self.initialize_trackbar(
            ctrl_id::TEXTSIZE_TRACKBAR,
            6,
            100,
            config.translation_style.size,
        )?;
        self.set_text(
            ctrl_id::TEXTSIZE_EDIT,
            &config.translation_style.size.to_string(),
        )?;
        self.initialize_trackbar(
            ctrl_id::OUTLINE1_TRACKBAR,
            0,
            20,
            config.translation_style.outline1_size,
        )?;
        self.set_text(
            ctrl_id::OUTLINE1_EDIT,
            &config.translation_style.outline1_size.to_string(),
        )?;
        self.initialize_trackbar(
            ctrl_id::OUTLINE2_TRACKBAR,
            0,
            20,
            config.translation_style.outline2_size,
        )?;
        self.set_text(
            ctrl_id::OUTLINE2_EDIT,
            &config.translation_style.outline2_size.to_string(),
        )?;
        self.initialize_trackbar(ctrl_id::SHADOW_X_TRACKBAR, 0, 20, config.shadow_offset_x)?;
        self.set_text(ctrl_id::SHADOW_X_EDIT, &config.shadow_offset_x.to_string())?;
        self.initialize_trackbar(ctrl_id::SHADOW_Y_TRACKBAR, 0, 20, config.shadow_offset_y)?;
        self.set_text(ctrl_id::SHADOW_Y_EDIT, &config.shadow_offset_y.to_string())?;
        self.set_checked(ctrl_id::NAME_SHADOW, config.name_style.shadow_enabled)?;
        self.set_checked(ctrl_id::ORG_SHADOW, config.original_style.shadow_enabled)?;
        self.set_checked(
            ctrl_id::TRANS_SHADOW,
            config.translation_style.shadow_enabled,
        )?;
        self.initialize_trackbar(ctrl_id::MARGIN_X_TRACKBAR, 0, 300, config.text_margin_x)?;
        self.set_text(ctrl_id::MARGIN_X_EDIT, &config.text_margin_x.to_string())?;
        self.initialize_trackbar(ctrl_id::MARGIN_Y_TRACKBAR, 0, 300, config.text_margin_y)?;
        self.set_text(ctrl_id::MARGIN_Y_EDIT, &config.text_margin_y.to_string())?;
        self.initialize_trackbar(ctrl_id::MARGIN_NAME_TRACKBAR, 0, 300, config.name_margin)?;
        self.set_text(ctrl_id::MARGIN_NAME_EDIT, &config.name_margin.to_string())?;
        self.set_checked(ctrl_id::BORDER_MODE, config.border_visible)?;
        self.initialize_trackbar(ctrl_id::BORDER_SIZE_TRACKBAR, 0, 10, config.border_width)?;
        self.set_text(ctrl_id::BORDER_SIZE_EDIT, &config.border_width.to_string())?;

        self.set_checked(ctrl_id::PRINT_ORGTEXT, config.show_original)?;
        self.set_checked(ctrl_id::PRINT_TRANSTEXT, config.show_translation)?;
        self.set_checked(ctrl_id::PRINT_ORGNAME, config.show_name)?;
        self.set_checked(ctrl_id::SEPERATE_NAME, config.separate_name)?;
        self.set_checked(
            ctrl_id::TEXTALIGN_LEFT,
            config.text_align == TextAlign::Left,
        )?;
        self.set_checked(
            ctrl_id::TEXTALIGN_MID,
            config.text_align == TextAlign::Center,
        )?;
        self.set_checked(
            ctrl_id::TEXTALIGN_RIGHT,
            config.text_align == TextAlign::Right,
        )?;
        self.set_checked(ctrl_id::TOPMOST, config.window_topmost)?;
        self.set_checked(ctrl_id::USE_MAGNETIC, config.magnetic_mode)?;
        self.set_checked(ctrl_id::MAGNETIC_MINIMIZE, config.magnetic_minimize)?;
        self.set_checked(ctrl_id::CLIPBOARD_WATCH, config.clipboard_watch)?;
        self.set_checked(ctrl_id::WNDCLICK_THROUGH, config.click_through)?;
        self.set_checked(
            ctrl_id::CLIPBOARD_CACHE_ENABLED,
            config.clipboard_cache_enabled,
        )?;

        let engine_names = [
            "EzTrans",
            "Google",
            "DeepL",
            "Papago API",
            "LLM",
            "Custom API",
        ];
        let engine = config
            .translation
            .get_engine()
            .map_err(|error| Error::new(E_INVALIDARG, error.to_string()))?;
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
                .map_err(|error| Error::new(E_INVALIDARG, error.to_string()))?
        };
        self.initialize_combo(ctrl_id::CUSTOM_API_SELECT, &custom_names, custom_index)?;
        unsafe {
            let _ = SendMessageW(
                self.control(ctrl_id::TRANS_ENGINE)?,
                CB_SETDROPPEDWIDTH,
                Some(WPARAM(220)),
                None,
            );
        }
        self.set_text(
            ctrl_id::EZTRANS_DLL_EDIT,
            &config.translation.eztrans_dll_path,
        )?;
        self.set_text(
            ctrl_id::EZTRANS_DAT_EDIT,
            &config.translation.eztrans_dat_path,
        )?;
        self.set_text(
            ctrl_id::EZTRANS_DICTIONARY_COUNT_LABEL,
            &format!(
                "후처리 사전: {}",
                config.translation.eztrans_postprocess_dictionary.len()
            ),
        )?;
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
                let _ = SendMessageW(
                    key_list,
                    LB_ADDSTRING,
                    Some(WPARAM(0)),
                    Some(LPARAM(wide.as_ptr() as isize)),
                );
            }
        }
        self.set_text(
            ctrl_id::PAPAGO_ID_EDIT,
            &config.translation.papago_client_id,
        )?;
        self.set_text(
            ctrl_id::PAPAGO_SECRET_EDIT,
            &config.translation.papago_client_secret,
        )?;

        let provider = config
            .translation
            .llm
            .get_provider()
            .map_err(|error| Error::new(E_INVALIDARG, error.to_string()))?;
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
        )?;
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
        )?;
        Ok(())
    }

    fn initialize_info(&self) -> Result<()> {
        self.set_text(ctrl_id::APP_VERSION, APP_VERSION)
    }

    pub(super) fn control(&self, id: u16) -> Result<HWND> {
        unsafe { GetDlgItem(Some(self.hwnd), id as i32) }
    }

    fn register_ids(&mut self, tab: usize, ids: &[u16]) -> Result<()> {
        let controls = ids
            .iter()
            .map(|&id| self.control(id))
            .collect::<Result<Vec<_>>>()?;
        self.tab_controls[tab].extend(controls);
        Ok(())
    }

    fn register_engine_ids(&mut self, group: EngineGroup, ids: &[u16]) -> Result<()> {
        let controls = ids
            .iter()
            .map(|&id| self.control(id))
            .collect::<Result<Vec<_>>>()?;
        self.engine_controls[group as usize].extend(controls);
        Ok(())
    }

    fn initialize_tab_titles(&self) -> Result<()> {
        let tab = self.control(ctrl_id::TAB_CONTROL)?;
        for (index, title) in ["외관", "표시·윈도우", "번역", "단축키", "정보"]
            .iter()
            .enumerate()
        {
            let mut wide = to_wide(title);
            let item = TCITEMW {
                mask: TCIF_TEXT,
                pszText: PWSTR(wide.as_mut_ptr()),
                iImage: -1,
                ..Default::default()
            };
            unsafe {
                let _ = SendMessageW(
                    tab,
                    TCM_INSERTITEMW,
                    Some(WPARAM(index)),
                    Some(LPARAM(&item as *const TCITEMW as isize)),
                );
            }
        }
        Ok(())
    }

    pub(super) fn initialize_combo(&self, id: u16, items: &[&str], selected: usize) -> Result<()> {
        let combo = self.control(id)?;
        for item in items {
            let wide = to_wide(item);
            unsafe {
                let _ = SendMessageW(
                    combo,
                    CB_ADDSTRING,
                    Some(WPARAM(0)),
                    Some(LPARAM(wide.as_ptr() as isize)),
                );
            }
        }
        unsafe {
            let _ = SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(selected)), Some(LPARAM(0)));
        }
        Ok(())
    }

    /// 제공자별 모델 프리셋을 채우되 목록 밖의 직접 입력값도 그대로 보존한다.
    pub(super) fn populate_llm_model_combo(
        &self,
        provider: crate::translation::LlmProvider,
        configured_model: &str,
    ) -> Result<()> {
        let combo = self.control(ctrl_id::LLM_MODEL_EDIT)?;
        unsafe {
            let _ = SendMessageW(combo, CB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
        }
        for model in provider.model_presets() {
            let wide = to_wide(model);
            unsafe {
                let _ = SendMessageW(
                    combo,
                    CB_ADDSTRING,
                    Some(WPARAM(0)),
                    Some(LPARAM(wide.as_ptr() as isize)),
                );
            }
        }
        self.set_text(
            ctrl_id::LLM_MODEL_EDIT,
            provider.model_or_default(configured_model),
        )
    }

    /// 선택된 제공자의 설정값으로 LLM 입력 컨트롤 전체를 갱신한다.
    pub(super) fn refresh_llm_provider_controls(
        &self,
        provider: crate::translation::LlmProvider,
    ) -> Result<()> {
        let (
            model,
            api_key,
            system_prompt,
            temperature,
            max_tokens,
            reasoning_effort,
            debounce_ms,
            glossary_count,
        ) = {
            let draft = self.draft.borrow();
            let llm = &draft.translation.llm;
            (
                llm.model.clone(),
                llm.api_key.clone(),
                llm.system_prompt.clone(),
                llm.temperature,
                llm.max_tokens,
                llm.reasoning_effort,
                llm.debounce_ms,
                llm.glossary.len(),
            )
        };

        self.populate_llm_model_combo(provider, &model)?;
        self.set_text(ctrl_id::LLM_API_KEY_EDIT, &api_key)?;
        self.set_text(ctrl_id::LLM_SYSTEM_PROMPT_EDIT, &system_prompt)?;
        self.initialize_trackbar(
            ctrl_id::LLM_TEMPERATURE_TRACKBAR,
            crate::config::limits::LLM_TEMPERATURE_SLIDER_MIN,
            crate::config::limits::LLM_TEMPERATURE_SLIDER_MAX,
            crate::config::limits::llm_temperature_to_slider(temperature),
        )?;
        self.set_text(ctrl_id::LLM_TEMPERATURE_EDIT, &format!("{temperature:.2}"))?;
        self.set_text(ctrl_id::LLM_MAX_TOKENS_EDIT, &max_tokens.to_string())?;
        let reasoning_effort_index = reasoning_effort
            .and_then(|configured| {
                crate::translation::llm::ReasoningEffort::ALL
                    .iter()
                    .position(|&effort| effort == configured)
            })
            .map_or(0, |index| index + 1);
        self.set_combo_selection(ctrl_id::LLM_REASONING_EFFORT, reasoning_effort_index)?;
        self.set_text(ctrl_id::LLM_DEBOUNCE_EDIT, &debounce_ms.to_string())?;
        self.set_text(
            ctrl_id::LLM_GLOSSARY_COUNT_LABEL,
            &format!("사전 항목: {glossary_count}"),
        )
    }

    fn set_combo_selection(&self, id: u16, selected: usize) -> Result<()> {
        let combo = self.control(id)?;
        unsafe {
            let _ = SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(selected)), Some(LPARAM(0)));
        }
        Ok(())
    }

    fn initialize_trackbar(&self, id: u16, min: i32, max: i32, value: i32) -> Result<()> {
        let trackbar = self.control(id)?;
        let range = ((max & 0xffff) << 16) | (min & 0xffff);
        unsafe {
            let _ = SendMessageW(
                trackbar,
                TBM_SETRANGE,
                Some(WPARAM(1)),
                Some(LPARAM(range as isize)),
            );
            let _ = SendMessageW(
                trackbar,
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM(value as isize)),
            );
        }
        Ok(())
    }

    pub(super) fn set_checked(&self, id: u16, checked: bool) -> Result<()> {
        let control = self.control(id)?;
        let state = if checked { BST_CHECKED } else { BST_UNCHECKED };
        unsafe {
            let _ = SendMessageW(
                control,
                BM_SETCHECK,
                Some(WPARAM(state.0 as usize)),
                Some(LPARAM(0)),
            );
        }
        Ok(())
    }

    pub(super) fn set_text(&self, id: u16, text: &str) -> Result<()> {
        let control = self.control(id)?;
        unsafe { SetWindowTextW(control, &HSTRING::from(text)) }
    }
}
