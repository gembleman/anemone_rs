//! DIALOGEX 리소스에서 생성된 설정 컨트롤의 런타임 초기화.

use super::*;
use windows_core::{Error, HRESULT};

/// 탭 컨트롤 여백 드래그 subclass의 `uIdSubclass`.
const TAB_DRAG_SUBCLASS_ID: usize = 2;

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
    ctrl_id::CLIPBOARD_SOURCE_LANG_GUARD,
];

/// 엔진별 컨트롤의 단일 출처.
///
/// 번역 탭 목록은 여기에서 유도되므로(`adopt_engine_controls_into_translation_tab`)
/// 엔진 컨트롤을 추가할 때는 이 표만 고치면 된다.
pub(super) const ENGINE_CONTROL_IDS: &[(EngineGroup, &[u16])] = &[
    (EngineGroup::EzTrans, ctrl_id::EZTRANS_STATIC_IDS),
    (
        EngineGroup::EzTrans,
        &[
            ctrl_id::EZTRANS_DICTIONARY_EDIT,
            ctrl_id::EZTRANS_DICTIONARY_BROWSE,
            ctrl_id::EZTRANS_EHND_EDIT,
            ctrl_id::EZTRANS_EHND_BROWSE,
            ctrl_id::EZTRANS_DICTIONARY_EDIT_BTN,
        ],
    ),
    (EngineGroup::DeepL, ctrl_id::DEEPL_STATIC_IDS),
    (
        EngineGroup::DeepL,
        &[
            ctrl_id::DEEPL_KEYS_LIST,
            ctrl_id::DEEPL_KEY_ADD_EDIT,
            ctrl_id::DEEPL_KEY_ADD_BTN,
            ctrl_id::DEEPL_KEY_REMOVE_BTN,
            ctrl_id::DEEPL_STRATEGY_COMBO,
            ctrl_id::DEEPL_KEY_TIER_COMBO,
        ],
    ),
    (EngineGroup::Papago, ctrl_id::PAPAGO_STATIC_IDS),
    (
        EngineGroup::Papago,
        &[ctrl_id::PAPAGO_ID_EDIT, ctrl_id::PAPAGO_SECRET_EDIT],
    ),
    (EngineGroup::Llm, ctrl_id::LLM_STATIC_IDS),
    (
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
    ),
    (EngineGroup::Custom, ctrl_id::CUSTOM_STATIC_IDS),
    (EngineGroup::Custom, &[ctrl_id::CUSTOM_API_SELECT]),
    (
        EngineGroup::MysTranslater,
        ctrl_id::MYS_TRANSLATER_STATIC_IDS,
    ),
    (
        EngineGroup::MysTranslater,
        &[
            ctrl_id::MYS_TRANSLATER_FREE_TOKEN_BTN,
            ctrl_id::MYS_TRANSLATER_USAGE_REFRESH_BTN,
        ],
    ),
];

/// 엔진 그룹에 등록된 모든 컨트롤 ID (테스트에서 누락 검증에 쓴다).
#[cfg(test)]
pub(super) fn engine_control_ids() -> impl Iterator<Item = u16> {
    ENGINE_CONTROL_IDS
        .iter()
        .flat_map(|(_, ids)| ids.iter().copied())
}

pub(super) const HOTKEYS_IDS: &[u16] = &[ctrl_id::HOTKEYS_LIST, ctrl_id::HOTKEYS_RESET];
pub(super) const INFO_IDS: &[u16] = &[
    ctrl_id::APP_VERSION,
    ctrl_id::UPDATE_CHECK_BTN,
    ctrl_id::UPDATE_STATUS,
    ctrl_id::UPDATE_AUTO_CHECK,
    ctrl_id::UPDATE_RELEASE_PAGE,
];

/// 탭 정적 라벨과 컨트롤 ID를 함께 등록하는 (탭, ID 목록) 쌍의 단일 출처.
const TAB_ID_REGISTRATIONS: &[(usize, &[u16])] = &[
    (TAB_APPEARANCE, ctrl_id::APPEARANCE_STATIC_IDS),
    (TAB_APPEARANCE, APPEARANCE_IDS),
    (TAB_DISPLAY, ctrl_id::DISPLAY_STATIC_IDS),
    (TAB_DISPLAY, DISPLAY_IDS),
    (TAB_TRANSLATION, ctrl_id::TRANSLATION_COMMON_IDS),
    (TAB_HOTKEYS, ctrl_id::HOTKEYS_STATIC_IDS),
    (TAB_HOTKEYS, HOTKEYS_IDS),
    (TAB_INFO, ctrl_id::INFO_STATIC_IDS),
    (TAB_INFO, INFO_IDS),
];

/// 초기화 시 숨겨둔 채 시작하는 탭 (외관 탭만 처음부터 표시된다).
const INITIALLY_HIDDEN_TABS: &[usize] = &[TAB_DISPLAY, TAB_TRANSLATION, TAB_HOTKEYS, TAB_INFO];

impl SettingsDialog {
    /// 리소스에 정의된 컨트롤을 탭/엔진 그룹에 연결하고 설정값을 주입한다.
    pub(super) fn initialize_controls(&mut self) -> Result<()> {
        self.register_tab_ids()?;
        // 엔진 컨트롤은 모두 번역 탭에 속한다. 두 목록을 따로 관리하면
        // 한쪽에만 추가하는 실수가 생기므로(2207/2208 경고 라벨 누락 사례)
        // 엔진 그룹에서 번역 탭 목록을 자동으로 유도한다.
        self.register_engine_controls()?;
        self.adopt_engine_controls_into_translation_tab();
        self.initialize_tab_titles()?;
        self.initialize_values()?;
        self.initialize_info()?;
        self.initialize_hotkey_list()?;
        self.hide_tabs(INITIALLY_HIDDEN_TABS);

        self.adjust_dialog_size_for_tab(TAB_APPEARANCE);
        let engine = self
            .draft
            .borrow()
            .translation
            .get_engine()
            .map_err(|error| Error::new(HRESULT(E_INVALIDARG), error.to_string()))?;
        self.apply_engine_state(engine);
        Ok(())
    }

    fn register_tab_ids(&mut self) -> Result<()> {
        for &(tab, ids) in TAB_ID_REGISTRATIONS {
            self.register_ids(tab, ids)?;
        }
        Ok(())
    }

    /// 주어진 탭들에 속한 컨트롤을 모두 숨긴다.
    fn hide_tabs(&self, tabs: &[usize]) {
        for &tab in tabs {
            for &hwnd in &self.tab_controls[tab] {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }
        }
    }

    fn register_engine_controls(&mut self) -> Result<()> {
        for &(group, ids) in ENGINE_CONTROL_IDS {
            self.register_engine_ids(group, ids)?;
        }
        Ok(())
    }

    fn initialize_info(&self) -> Result<()> {
        self.set_text(ctrl_id::APP_VERSION, APP_VERSION)?;
        self.set_checked(
            ctrl_id::UPDATE_AUTO_CHECK,
            self.draft.borrow().update_check_enabled,
        )?;
        self.set_text(ctrl_id::UPDATE_STATUS, "")
    }

    pub(super) fn control(&self, id: u16) -> Result<HWND> {
        let hwnd = unsafe { GetDlgItem(self.hwnd, id as i32) };
        if hwnd.is_null() {
            Err(Error::from_thread())
        } else {
            Ok(hwnd)
        }
    }

    fn register_ids(&mut self, tab: usize, ids: &[u16]) -> Result<()> {
        let controls = ids
            .iter()
            .map(|&id| self.control(id))
            .collect::<Result<Vec<_>>>()?;
        self.tab_controls[tab].extend(controls);
        Ok(())
    }

    /// 엔진별 컨트롤을 번역 탭 목록에 합친다.
    ///
    /// 엔진 컨트롤은 전부 번역 탭 위에 놓이므로 탭 전환 시 함께 표시/숨김되어야
    /// 한다. `engine_controls`를 단일 출처로 삼아 중복 없이 복사한다.
    fn adopt_engine_controls_into_translation_tab(&mut self) {
        let mut adopted: Vec<HWND> = Vec::new();
        for group in &self.engine_controls {
            for &hwnd in group {
                if !self.tab_controls[TAB_TRANSLATION].contains(&hwnd) && !adopted.contains(&hwnd) {
                    adopted.push(hwnd);
                }
            }
        }
        self.tab_controls[TAB_TRANSLATION].extend(adopted);
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
                pszText: wide.as_mut_ptr(),
                iImage: -1,
                ..Default::default()
            };
            unsafe {
                let _ = SendMessageW(
                    tab,
                    TCM_INSERTITEMW,
                    index,
                    &item as *const TCITEMW as isize,
                );
            }
        }
        // 설정 창의 여백은 대부분 탭 본문이라 dialog까지 클릭이 오지 않는다.
        crate::dialogs::helpers::enable_tab_client_drag(tab, TAB_DRAG_SUBCLASS_ID);
        Ok(())
    }

    pub(super) fn initialize_combo(&self, id: u16, items: &[&str], selected: usize) -> Result<()> {
        let combo = self.control(id)?;
        for item in items {
            let wide = to_wide(item);
            unsafe {
                let _ = SendMessageW(combo, CB_ADDSTRING, 0, wide.as_ptr() as isize);
            }
        }
        unsafe {
            let _ = SendMessageW(combo, CB_SETCURSEL, selected, 0);
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
            let _ = SendMessageW(combo, CB_RESETCONTENT, 0, 0);
        }
        for model in provider.model_presets() {
            let wide = to_wide(model);
            unsafe {
                let _ = SendMessageW(combo, CB_ADDSTRING, 0, wide.as_ptr() as isize);
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
            let _ = SendMessageW(combo, CB_SETCURSEL, selected, 0);
        }
        Ok(())
    }

    pub(super) fn initialize_trackbar(
        &self,
        id: u16,
        min: i32,
        max: i32,
        value: i32,
    ) -> Result<()> {
        let trackbar = self.control(id)?;
        let range = ((max & 0xffff) << 16) | (min & 0xffff);
        unsafe {
            let _ = SendMessageW(trackbar, TBM_SETRANGE, 1, range as isize);
            let _ = SendMessageW(trackbar, TBM_SETPOS, 1, value as isize);
        }
        Ok(())
    }

    pub(super) fn set_checked(&self, id: u16, checked: bool) -> Result<()> {
        let control = self.control(id)?;
        let state = if checked { BST_CHECKED } else { BST_UNCHECKED };
        unsafe {
            let _ = SendMessageW(control, BM_SETCHECK, state as usize, 0);
        }
        Ok(())
    }

    pub(super) fn set_text(&self, id: u16, text: &str) -> Result<()> {
        let control = self.control(id)?;
        let wide = crate::win32::to_wide(text);
        if unsafe { SetWindowTextW(control, wide.as_ptr()) } == 0 {
            Err(Error::from_thread())
        } else {
            Ok(())
        }
    }
}
