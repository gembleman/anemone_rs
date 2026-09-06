//! 번역 탭에서 선택된 엔진의 전용 패널만 보이도록 하는 컨트롤 그룹과,
//! 사용량 라벨 서식.

use windows_sys::Win32::{UI::Input::KeyboardAndMouse::EnableWindow, UI::WindowsAndMessaging::*};

use super::{SettingsDialog, TAB_TRANSLATION, ctrl_id};
use crate::translation::{TranslationEngine, lang_utils};
use crate::win32::to_wide;

/// 선택된 엔진 패널만 표시하기 위한 컨트롤 그룹.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EngineGroup {
    EzTrans = 0,
    DeepL = 1,
    Papago = 2,
    Llm = 3,
    Custom = 4,
    MysTranslater = 5,
}

fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(character);
    }
    formatted
}

pub(super) fn format_mys_usage(
    snapshot: Option<crate::translation::mys_usage::UsageSnapshot>,
) -> String {
    let Some(usage) = snapshot else {
        return "이 기기 이번 달 사용량: 아직 없음".to_string();
    };
    format!(
        "이 기기 이번 달: 신규 {}토큰 (입력 {} / 출력 {})\r\n캐시 {}자 · 처리 {}건",
        format_count(usage.fresh_total_tokens()),
        format_count(usage.fresh_prompt_tokens),
        format_count(usage.fresh_output_tokens),
        format_count(usage.cached_characters),
        format_count(usage.total_count()),
    )
}

impl SettingsDialog {
    pub(super) fn engine_group(engine: TranslationEngine) -> Option<EngineGroup> {
        match engine {
            TranslationEngine::EzTrans => Some(EngineGroup::EzTrans),
            TranslationEngine::DeepL => Some(EngineGroup::DeepL),
            TranslationEngine::Papago => Some(EngineGroup::Papago),
            TranslationEngine::Llm => Some(EngineGroup::Llm),
            TranslationEngine::Custom => Some(EngineGroup::Custom),
            TranslationEngine::MysTranslater => Some(EngineGroup::MysTranslater),
            TranslationEngine::Google => None,
        }
    }

    /// 번역 탭에서는 선택된 엔진의 전용 컨트롤만 표시한다.
    pub(super) fn update_engine_controls(&self, engine: TranslationEngine) {
        let active = Self::engine_group(engine).map(|group| group as usize);
        let translation_tab_visible = self.current_tab == TAB_TRANSLATION;
        let free_token_button = self.control(ctrl_id::MYS_TRANSLATER_FREE_TOKEN_BTN).ok();
        let signup_in_progress = self.mys_signup_in_progress.get();
        // SAFETY: HWNDs in engine_controls are valid child controls.
        unsafe {
            for (idx, group) in self.engine_controls.iter().enumerate() {
                let visible = translation_tab_visible && active == Some(idx);
                for &h in group {
                    let blocked_by_signup = signup_in_progress && Some(h) == free_token_button;
                    let enabled = visible && !blocked_by_signup;
                    let _ = EnableWindow(h, if enabled { 1 } else { 0 });
                    let _ = ShowWindow(h, if visible { SW_SHOW } else { SW_HIDE });
                }
            }
        }
        if translation_tab_visible && engine == TranslationEngine::MysTranslater {
            self.refresh_mys_token_status();
            self.refresh_mys_usage();
        }
    }

    pub(super) fn refresh_mys_token_status(&self) {
        let configured = !self
            .draft
            .borrow()
            .translation
            .mys_translater_api_key
            .trim()
            .is_empty();
        self.set_control_text(
            ctrl_id::MYS_TRANSLATER_TOKEN_STATUS_LABEL,
            if configured {
                "발급됨 (앱이 보관합니다)"
            } else {
                "없음 — [무료 토큰 받기]를 눌러 주세요"
            },
        );
    }

    pub(super) fn refresh_mys_usage(&self) {
        let (url, token) = {
            let draft = self.draft.borrow();
            (
                draft.translation.mys_translater_url.clone(),
                draft.translation.mys_translater_api_key.clone(),
            )
        };
        let text = format_mys_usage(crate::translation::mys_usage::snapshot(&url, &token));
        self.set_control_text(ctrl_id::MYS_TRANSLATER_USAGE_LABEL, &text);
    }

    /// 현재 엔진에 맞춰 소스/타겟 언어 콤보 항목을 갱신
    pub(super) fn refresh_language_combos(&self, engine: TranslationEngine) {
        // SAFETY: self.hwnd is valid; GetDlgItem returns valid combobox handles.
        unsafe {
            let src = GetDlgItem(self.hwnd, ctrl_id::TRANS_SOURCE_LANG as i32);
            let tgt = GetDlgItem(self.hwnd, ctrl_id::TRANS_TARGET_LANG as i32);
            if src.is_null() || tgt.is_null() {
                return;
            }

            let _ = SendMessageW(src, CB_RESETCONTENT, 0, 0);
            for &lang in engine.supported_source_languages() {
                let w = to_wide(lang_utils::to_korean_name(lang));
                let _ = SendMessageW(src, CB_ADDSTRING, 0, w.as_ptr() as isize);
            }
            let src_sel = match self.draft.borrow().translation.source_lang_index(engine) {
                Ok(index) => index,
                Err(error) => {
                    tracing::error!("번역 언어 설정 오류: {error}");
                    return;
                }
            };
            let _ = SendMessageW(src, CB_SETCURSEL, src_sel, 0);

            let source = engine
                .supported_source_languages()
                .get(src_sel)
                .copied()
                .or_else(|| engine.supported_source_languages().first().copied());
            let Some(source) = source else {
                return;
            };
            let targets = engine.supported_targets_for(source);

            let _ = SendMessageW(tgt, CB_RESETCONTENT, 0, 0);
            for &lang in &targets {
                let w = to_wide(lang_utils::to_korean_name(lang));
                let _ = SendMessageW(tgt, CB_ADDSTRING, 0, w.as_ptr() as isize);
            }
            let configured_target = self.draft.borrow().translation.get_target_language().ok();
            let tgt_sel = configured_target
                .and_then(|target| targets.iter().position(|&language| language == target))
                .unwrap_or(0);
            let _ = SendMessageW(tgt, CB_SETCURSEL, tgt_sel, 0);
        }
    }

    /// 엔진별 패널, 언어 콤보, 번역 탭 높이를 함께 갱신한다.
    pub(super) fn apply_engine_state(&mut self, engine: TranslationEngine) {
        self.resize_translation_group(engine);
        self.update_engine_controls(engine);
        self.refresh_language_combos(engine);
        if self.current_tab == TAB_TRANSLATION {
            self.adjust_dialog_size_for_tab(TAB_TRANSLATION);
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/settings/engine_panel.rs"]
mod tests;
