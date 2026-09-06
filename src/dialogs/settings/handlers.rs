//! 설정 대화상자 명령 처리(`WM_COMMAND` 라우팅)와 설정 변경 적용/취소.
//!
//! 콤보박스·트랙바·edit killfocus 처리는 `handlers_input`, 색상·글꼴·파일
//! 대화상자류는 `handlers_dialogs`에 있다. 이 파일은 두 곳에서 공통으로 쓰는
//! 트랙바 바인딩 표와 draft 적용/커밋 경로의 단일 출처다.

use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::SettingsDialog;
use super::ctrl_id;
use super::model::{
    BoolSetting, NumericSetting, SettingsChange, SettingsChangeResult, SettingsEditor,
};
use crate::config::{ColorType, TextAlign, TextType};
use crate::dialogs::models::SettingsDraft;
use crate::translation::settings::{
    TranslationSettingChange, TranslationSettingsChangeResult, TranslationSettingsEditor,
    TranslationSettingsError,
};

#[derive(Clone, Copy)]
pub(super) struct NumericControlBinding {
    pub(super) trackbar_id: u16,
    pub(super) edit_id: u16,
    pub(super) setting: NumericSetting,
}

pub(super) const NUMERIC_CONTROL_BINDINGS: &[NumericControlBinding] = &[
    NumericControlBinding {
        trackbar_id: ctrl_id::BACKGROUND_TRACKBAR,
        edit_id: ctrl_id::BACKGROUND_EDIT,
        setting: NumericSetting::BackgroundAlpha,
    },
    NumericControlBinding {
        trackbar_id: ctrl_id::TEXTSIZE_TRACKBAR,
        edit_id: ctrl_id::TEXTSIZE_EDIT,
        setting: NumericSetting::TextSize(ColorType::Primary),
    },
    NumericControlBinding {
        trackbar_id: ctrl_id::OUTLINE1_TRACKBAR,
        edit_id: ctrl_id::OUTLINE1_EDIT,
        setting: NumericSetting::TextSize(ColorType::Outline1),
    },
    NumericControlBinding {
        trackbar_id: ctrl_id::OUTLINE2_TRACKBAR,
        edit_id: ctrl_id::OUTLINE2_EDIT,
        setting: NumericSetting::TextSize(ColorType::Outline2),
    },
    NumericControlBinding {
        trackbar_id: ctrl_id::SHADOW_X_TRACKBAR,
        edit_id: ctrl_id::SHADOW_X_EDIT,
        setting: NumericSetting::ShadowOffsetX,
    },
    NumericControlBinding {
        trackbar_id: ctrl_id::SHADOW_Y_TRACKBAR,
        edit_id: ctrl_id::SHADOW_Y_EDIT,
        setting: NumericSetting::ShadowOffsetY,
    },
    NumericControlBinding {
        trackbar_id: ctrl_id::MARGIN_X_TRACKBAR,
        edit_id: ctrl_id::MARGIN_X_EDIT,
        setting: NumericSetting::TextMarginX,
    },
    NumericControlBinding {
        trackbar_id: ctrl_id::MARGIN_Y_TRACKBAR,
        edit_id: ctrl_id::MARGIN_Y_EDIT,
        setting: NumericSetting::TextMarginY,
    },
    NumericControlBinding {
        trackbar_id: ctrl_id::MARGIN_NAME_TRACKBAR,
        edit_id: ctrl_id::MARGIN_NAME_EDIT,
        setting: NumericSetting::NameMargin,
    },
    NumericControlBinding {
        trackbar_id: ctrl_id::BORDER_SIZE_TRACKBAR,
        edit_id: ctrl_id::BORDER_SIZE_EDIT,
        setting: NumericSetting::BorderWidth,
    },
];

pub(super) fn numeric_binding_for_trackbar(id: u16) -> Option<NumericControlBinding> {
    NUMERIC_CONTROL_BINDINGS
        .iter()
        .copied()
        .find(|binding| binding.trackbar_id == id)
}

pub(super) fn numeric_binding_for_edit(id: u16) -> Option<NumericControlBinding> {
    NUMERIC_CONTROL_BINDINGS
        .iter()
        .copied()
        .find(|binding| binding.edit_id == id)
}

pub(super) fn commits_on_enter(id: u16) -> bool {
    numeric_binding_for_edit(id).is_some()
        || matches!(
            id,
            ctrl_id::LLM_MAX_TOKENS_EDIT
                | ctrl_id::LLM_TEMPERATURE_EDIT
                | ctrl_id::LLM_DEBOUNCE_EDIT
        )
}

pub(super) fn numeric_setting_value(
    config: &crate::config::Config,
    setting: NumericSetting,
) -> i32 {
    match setting {
        NumericSetting::BackgroundAlpha => ((config.background_color >> 24) & 0xff) as i32,
        NumericSetting::TextSize(ColorType::Primary) => config.translation_style.size,
        NumericSetting::TextSize(ColorType::Outline1) => config.translation_style.outline1_size,
        NumericSetting::TextSize(ColorType::Outline2) => config.translation_style.outline2_size,
        NumericSetting::TextSize(ColorType::Shadow) => 0,
        NumericSetting::ShadowOffsetX => config.shadow_offset_x,
        NumericSetting::ShadowOffsetY => config.shadow_offset_y,
        NumericSetting::TextMarginX => config.text_margin_x,
        NumericSetting::TextMarginY => config.text_margin_y,
        NumericSetting::NameMargin => config.name_margin,
        NumericSetting::BorderWidth => config.border_width,
    }
}

/// 트랙바 초기화에 필요한 (최소, 최대) 범위. 현재 값은 [`numeric_setting_value`]가 맡는다.
pub(super) fn numeric_setting_range(setting: NumericSetting) -> (i32, i32) {
    match setting {
        NumericSetting::BackgroundAlpha => (0, 255),
        NumericSetting::TextSize(ColorType::Primary) => (6, 100),
        NumericSetting::TextSize(ColorType::Outline1)
        | NumericSetting::TextSize(ColorType::Outline2) => (0, 20),
        NumericSetting::TextSize(ColorType::Shadow) => (0, 0),
        NumericSetting::ShadowOffsetX | NumericSetting::ShadowOffsetY => (0, 20),
        NumericSetting::TextMarginX | NumericSetting::TextMarginY | NumericSetting::NameMargin => {
            (0, 300)
        }
        NumericSetting::BorderWidth => (0, 10),
    }
}

fn take_unapplied_changes(pending: &std::cell::Cell<bool>) -> bool {
    pending.replace(false)
}

fn restore_last_applied(
    pending: &std::cell::Cell<bool>,
    draft: &std::cell::RefCell<SettingsDraft>,
    last_applied: &std::cell::RefCell<SettingsDraft>,
) -> Option<SettingsDraft> {
    if !take_unapplied_changes(pending) {
        return None;
    }
    let restored = last_applied.borrow().clone();
    draft.replace(restored.clone());
    Some(restored)
}

fn record_last_applied(
    draft: &std::cell::RefCell<SettingsDraft>,
    last_applied: &std::cell::RefCell<SettingsDraft>,
) -> SettingsDraft {
    let applied = draft.borrow().clone();
    last_applied.replace(applied.clone());
    applied
}

/// 체크박스 토글 매크로: 컨트롤 ID와 독립적인 설정 명령으로 변환한다.
macro_rules! toggle_field {
    ($self:expr, $setting:expr) => {{
        $self.apply_settings_change(SettingsChange::Toggle($setting));
    }};
}

impl SettingsDialog {
    /// 명령 처리
    pub(super) fn handle_command(&mut self, cmd: u16, notify_code: u32) {
        // 편집 가능한 모델 콤보의 직접 입력은 포커스를 잃을 때 확정한다.
        if cmd == ctrl_id::LLM_MODEL_EDIT && notify_code == CBN_KILLFOCUS {
            self.handle_edit_killfocus(cmd);
            return;
        }

        // ComboBox 선택 변경은 별도 처리
        if notify_code == CBN_SELCHANGE {
            self.handle_combobox(cmd);
            return;
        }

        // Edit 컨트롤 포커스 해제 시 값 저장
        if notify_code == EN_KILLFOCUS {
            self.handle_edit_killfocus(cmd);
            return;
        }

        use ctrl_id::*;

        match cmd {
            APPLY => self.apply_changes(),

            // SAFETY: self.hwnd is a valid window handle from dialog creation.
            CLOSE => unsafe {
                let _ = PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
            },

            // 배경 색상
            BACKGROUND_COLOR => {
                let initial = self.draft.borrow().background_color;
                self.show_live_color_dialog(BACKGROUND_COLOR, initial, false, |argb| {
                    SettingsChange::BackgroundColor(argb)
                });
            }

            // 배경 표시 토글
            BACKGROUND_SWITCH => {
                toggle_field!(self, BoolSetting::BackgroundVisible);
            }

            // NAME/ORG/TRANS 색상 버튼
            NAME_COLOR => self.handle_color_button(NAME_COLOR, TextType::Name, ColorType::Primary),
            NAME_OUTLINE1 => {
                self.handle_color_button(NAME_OUTLINE1, TextType::Name, ColorType::Outline1)
            }
            NAME_OUTLINE2 => {
                self.handle_color_button(NAME_OUTLINE2, TextType::Name, ColorType::Outline2)
            }
            NAME_SHADOW_COLOR => {
                self.handle_color_button(NAME_SHADOW_COLOR, TextType::Name, ColorType::Shadow)
            }
            NAME_FONT => self.handle_font_button(TextType::Name),
            NAME_SHADOW => {
                toggle_field!(self, BoolSetting::TextShadow(TextType::Name));
            }

            ORG_COLOR => {
                self.handle_color_button(ORG_COLOR, TextType::Original, ColorType::Primary)
            }
            ORG_OUTLINE1 => {
                self.handle_color_button(ORG_OUTLINE1, TextType::Original, ColorType::Outline1)
            }
            ORG_OUTLINE2 => {
                self.handle_color_button(ORG_OUTLINE2, TextType::Original, ColorType::Outline2)
            }
            ORG_SHADOW_COLOR => {
                self.handle_color_button(ORG_SHADOW_COLOR, TextType::Original, ColorType::Shadow)
            }
            ORG_FONT => self.handle_font_button(TextType::Original),
            ORG_SHADOW => {
                toggle_field!(self, BoolSetting::TextShadow(TextType::Original));
            }

            TRANS_COLOR => {
                self.handle_color_button(TRANS_COLOR, TextType::Translation, ColorType::Primary)
            }
            TRANS_OUTLINE1 => {
                self.handle_color_button(TRANS_OUTLINE1, TextType::Translation, ColorType::Outline1)
            }
            TRANS_OUTLINE2 => {
                self.handle_color_button(TRANS_OUTLINE2, TextType::Translation, ColorType::Outline2)
            }
            TRANS_SHADOW_COLOR => self.handle_color_button(
                TRANS_SHADOW_COLOR,
                TextType::Translation,
                ColorType::Shadow,
            ),
            TRANS_FONT => self.handle_font_button(TextType::Translation),
            TRANS_SHADOW => {
                toggle_field!(self, BoolSetting::TextShadow(TextType::Translation));
            }

            // 테두리 설정
            BORDER_MODE => {
                toggle_field!(self, BoolSetting::BorderVisible);
            }
            BORDER_COLOR => {
                let initial = self.draft.borrow().border_color;
                self.show_live_color_dialog(BORDER_COLOR, initial, true, |argb| {
                    SettingsChange::BorderColor(argb)
                });
            }

            // 표시 옵션 체크박스
            PRINT_ORGTEXT => toggle_field!(self, BoolSetting::ShowOriginal),
            PRINT_TRANSTEXT => toggle_field!(self, BoolSetting::ShowTranslation),
            PRINT_ORGNAME => toggle_field!(self, BoolSetting::ShowName),
            SEPERATE_NAME => toggle_field!(self, BoolSetting::SeparateName),

            // 텍스트 정렬
            TEXTALIGN_LEFT => {
                self.apply_settings_change(SettingsChange::TextAlignment(TextAlign::Left));
            }
            TEXTALIGN_MID => {
                self.apply_settings_change(SettingsChange::TextAlignment(TextAlign::Center));
            }
            TEXTALIGN_RIGHT => {
                self.apply_settings_change(SettingsChange::TextAlignment(TextAlign::Right));
            }

            // 윈도우 옵션 체크박스
            TOPMOST => toggle_field!(self, BoolSetting::WindowTopmost),
            USE_MAGNETIC => {
                toggle_field!(self, BoolSetting::MagneticMode);
            }
            MAGNETIC_MINIMIZE => toggle_field!(self, BoolSetting::MagneticMinimize),
            CLIPBOARD_WATCH => {
                toggle_field!(self, BoolSetting::ClipboardWatch);
            }
            WNDCLICK_THROUGH => {
                toggle_field!(self, BoolSetting::ClickThrough);
            }
            CLIPBOARD_CACHE_ENABLED => {
                toggle_field!(self, BoolSetting::ClipboardCacheEnabled);
            }
            CLIPBOARD_SOURCE_LANG_GUARD => {
                toggle_field!(self, BoolSetting::ClipboardSourceLanguageGuard);
            }
            CLIPBOARD_CACHE_CLEAR => self.clear_translation_cache(),
            HOTKEYS_RESET => self.reset_hotkeys_to_default(),

            // EzTrans 평면 사전 찾아보기
            EZTRANS_DICTIONARY_BROWSE => match self.browse_dictionary_file("JisJK.flat.bin 선택")
            {
                Ok(Some(path)) => {
                    let _ = self.apply_translation_change(
                        TranslationSettingChange::EzTransDictionaryPath(path.clone()),
                    );
                    self.set_control_text(EZTRANS_DICTIONARY_EDIT, &path);
                    // 경고 라벨만 갱신하고 선택한 경로는 그대로 유지한다.
                    self.refresh_eztrans_path_warnings();
                }
                Ok(None) => {}
                Err(error) => self.show_file_dialog_error(&error),
            },

            // EzTrans Ehnd 폴더 찾아보기
            EZTRANS_EHND_BROWSE => match self.browse_folder_with_title("EzTrans Ehnd 폴더 선택")
            {
                Ok(Some(path)) => {
                    let _ = self.apply_translation_change(
                        TranslationSettingChange::EzTransEhndPath(path.clone()),
                    );
                    self.set_control_text(EZTRANS_EHND_EDIT, &path);
                    self.refresh_eztrans_path_warnings();
                }
                Ok(None) => {}
                Err(error) => self.show_file_dialog_error(&error),
            },

            EZTRANS_DICTIONARY_EDIT_BTN => self.open_eztrans_dictionary_editor(),

            // DeepL 보조 키 추가
            DEEPL_KEY_ADD_BTN => self.deepl_keys_add(),
            // DeepL 보조 키 삭제
            DEEPL_KEY_REMOVE_BTN => self.deepl_keys_remove(),

            // API 키 마스킹 표시 전환 (대화상자를 열 때마다 기본은 숨김)
            LLM_API_KEY_VISIBLE => {
                self.toggle_secret_visibility(LLM_API_KEY_VISIBLE, LLM_API_KEY_EDIT)
            }
            // 번역 서버 무료 토큰 받기
            MYS_TRANSLATER_FREE_TOKEN_BTN => self.handle_mys_free_token_button(),
            MYS_TRANSLATER_PURCHASE_BTN => self.handle_mys_purchase_button(),
            MYS_TRANSLATER_USAGE_REFRESH_BTN => self.refresh_mys_usage(),

            // 글로서리 편집 다이얼로그
            LLM_GLOSSARY_EDIT_BTN => self.open_glossary_editor(),

            // 업데이트 확인 / 릴리스 페이지 열기 / 자동 확인 체크박스
            UPDATE_CHECK_BTN => self.handle_update_button(),
            UPDATE_RELEASE_PAGE => crate::app::open_release_page(self.hwnd),
            UPDATE_AUTO_CHECK => toggle_field!(self, BoolSetting::UpdateCheckEnabled),

            _ => {}
        }
    }

    /// EzTrans 초기화 동기화 (다른 엔진은 워커가 매번 자격증명을 받아 stateless)
    fn sync_translation_manager(&self) {
        let config = self.draft.borrow();
        if let Err(e) = TranslationSettingsEditor::sync_runtime(&config.translation) {
            tracing::warn!("EzTrans init failed in sync: {e}");
        }
    }

    pub(super) fn apply_settings_change(&self, change: SettingsChange) -> SettingsChangeResult {
        let result = SettingsEditor::apply(&mut self.draft.borrow_mut(), change);
        self.finish_settings_change(result);
        result
    }

    fn finish_settings_change(&self, result: SettingsChangeResult) {
        if result.preview_refresh_required {
            self.notify_preview();
        }
        if result.save_required {
            self.has_unapplied_changes.set(true);
        }
    }

    pub(super) fn apply_translation_change(
        &self,
        change: TranslationSettingChange,
    ) -> std::result::Result<TranslationSettingsChangeResult, TranslationSettingsError> {
        let result =
            TranslationSettingsEditor::apply(&mut self.draft.borrow_mut().translation, change)?;
        self.finish_translation_change(result);
        Ok(result)
    }

    fn finish_translation_change(&self, result: TranslationSettingsChangeResult) {
        if result.runtime_sync_required {
            self.sync_translation_manager();
        }
        self.finish_settings_change(SettingsChangeResult::from_changed(result.changed));
    }

    /// 컨트롤 텍스트 설정 헬퍼
    pub(super) fn set_control_text(&self, ctrl_id: u16, text: &str) {
        crate::dialogs::helpers::set_dlg_item_text(self.hwnd, ctrl_id, text);
    }

    /// Edit 컨트롤에서 텍스트 가져오기
    pub(super) fn get_control_text(&self, ctrl_id: u16) -> String {
        crate::dialogs::helpers::get_dlg_item_text(self.hwnd, ctrl_id)
    }

    fn notify_preview(&self) {
        if let Some(actions) = &self.actions {
            actions.preview_settings(self.draft.borrow().clone());
        }
    }

    pub(super) fn glossary_applied(&self) {
        self.refresh_glossary_count();
        self.has_unapplied_changes.set(true);
        self.notify_preview();
    }

    pub(super) fn eztrans_dictionary_applied(&self) {
        self.refresh_eztrans_dictionary_count();
        self.has_unapplied_changes.set(true);
        self.notify_preview();
    }

    fn apply_changes(&self) {
        if !take_unapplied_changes(&self.has_unapplied_changes) {
            return;
        }
        self.sync_translation_manager();
        let applied = record_last_applied(self.draft.as_ref(), &self.last_applied);
        if let Some(actions) = &self.actions {
            actions.commit_settings(applied);
        }
    }

    pub(super) fn discard_unapplied_changes(&self) {
        let Some(restored) = restore_last_applied(
            &self.has_unapplied_changes,
            self.draft.as_ref(),
            &self.last_applied,
        ) else {
            return;
        };
        self.sync_translation_manager();
        if let Some(actions) = &self.actions {
            actions.preview_settings(restored);
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/settings/handlers.rs"]
mod tests;
