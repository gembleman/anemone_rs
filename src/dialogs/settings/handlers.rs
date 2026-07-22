//! 설정 대화상자 명령/이벤트 핸들러

use windows::{
    Win32::{Foundation::*, UI::Controls::*, UI::WindowsAndMessaging::*},
    core::*,
};

use super::ctrl_id;
use super::model::{
    BoolSetting, NumericSetting, SettingsChange, SettingsChangeResult, SettingsEditor,
};
use super::{SettingsDialog, format_deepl_key};
use crate::config::{ColorType, TextAlign, TextType};
use crate::dialogs::color::{ColorDialog, ColorDialogConfig};
use crate::dialogs::font::{FontDialog, FontDialogConfig, FontStyle};
use crate::dialogs::models::SettingsDraft;
use crate::translation::settings::{
    TranslationSettingChange, TranslationSettingsChangeResult, TranslationSettingsEditor,
    TranslationSettingsError,
};
use crate::win32::to_wide;

#[derive(Clone, Copy)]
struct NumericControlBinding {
    trackbar_id: u16,
    edit_id: u16,
    setting: NumericSetting,
}

const NUMERIC_CONTROL_BINDINGS: &[NumericControlBinding] = &[
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

fn numeric_binding_for_trackbar(id: u16) -> Option<NumericControlBinding> {
    NUMERIC_CONTROL_BINDINGS
        .iter()
        .copied()
        .find(|binding| binding.trackbar_id == id)
}

fn numeric_binding_for_edit(id: u16) -> Option<NumericControlBinding> {
    NUMERIC_CONTROL_BINDINGS
        .iter()
        .copied()
        .find(|binding| binding.edit_id == id)
}

fn numeric_setting_value(config: &crate::config::Config, setting: NumericSetting) -> i32 {
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
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
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
            CLIPBOARD_CACHE_CLEAR => self.clear_translation_cache(),
            HOTKEYS_RESET => self.reset_hotkeys_to_default(),

            // EzTrans DLL 찾아보기
            EZTRANS_DLL_BROWSE => match self.browse_dll_file("J2KEngine.dll 선택") {
                Ok(Some(path)) => {
                    let _ = self.apply_translation_change(
                        TranslationSettingChange::EzTransDllPath(path.clone()),
                    );
                    self.set_control_text(EZTRANS_DLL_EDIT, &path);
                }
                Ok(None) => {}
                Err(error) => self.show_file_dialog_error(&error),
            },

            // EzTrans Dat 폴더 찾아보기
            EZTRANS_DAT_BROWSE => match self.browse_folder_with_title("EzTrans Dat 폴더 선택") {
                Ok(Some(path)) => {
                    let _ = self.apply_translation_change(
                        TranslationSettingChange::EzTransDatPath(path.clone()),
                    );
                    self.set_control_text(EZTRANS_DAT_EDIT, &path);
                }
                Ok(None) => {}
                Err(error) => self.show_file_dialog_error(&error),
            },

            EZTRANS_DICTIONARY_EDIT_BTN => self.open_eztrans_dictionary_editor(),

            // DeepL 보조 키 추가
            DEEPL_KEY_ADD_BTN => self.deepl_keys_add(),
            // DeepL 보조 키 삭제
            DEEPL_KEY_REMOVE_BTN => self.deepl_keys_remove(),

            // 글로서리 편집 다이얼로그
            LLM_GLOSSARY_EDIT_BTN => self.open_glossary_editor(),

            _ => {}
        }
    }

    /// 입력한 DeepL key를 list와 config에 추가한다.
    fn deepl_keys_add(&mut self) {
        let key = self.get_control_text(ctrl_id::DEEPL_KEY_ADD_EDIT);
        let key = key.trim().to_string();
        if key.is_empty() {
            return;
        }
        let tier = unsafe {
            let Ok(combo) = GetDlgItem(Some(self.hwnd), ctrl_id::DEEPL_KEY_TIER_COMBO as i32)
            else {
                return;
            };
            match SendMessageW(combo, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 {
                1 => crate::translation::DeepLApiTier::Pro,
                _ => crate::translation::DeepLApiTier::Free,
            }
        };
        match self.apply_translation_change(TranslationSettingChange::AddDeepLKey {
            tier,
            key: key.clone(),
        }) {
            Ok(result) if result.changed => {}
            Ok(_) => return,
            Err(error) => {
                crate::dialogs::helpers::show_error_message(
                    self.hwnd,
                    "DeepL 키 유형 오류",
                    &error.to_string(),
                );
                return;
            }
        }
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid listbox.
        unsafe {
            let listbox = match GetDlgItem(Some(self.hwnd), ctrl_id::DEEPL_KEYS_LIST as i32) {
                Ok(h) if !h.is_invalid() => h,
                _ => return,
            };
            let key_wide = to_wide(&format_deepl_key(&key));
            let _ = SendMessageW(
                listbox,
                LB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(key_wide.as_ptr() as isize)),
            );
        }
        self.set_control_text(ctrl_id::DEEPL_KEY_ADD_EDIT, "");
    }

    /// 선택한 DeepL key를 list와 config에서 제거한다.
    fn deepl_keys_remove(&mut self) {
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid listbox.
        let sel = unsafe {
            let listbox = match GetDlgItem(Some(self.hwnd), ctrl_id::DEEPL_KEYS_LIST as i32) {
                Ok(h) if !h.is_invalid() => h,
                _ => return,
            };
            let sel =
                SendMessageW(listbox, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32;
            if sel == LB_ERR {
                return;
            }
            let _ = SendMessageW(
                listbox,
                LB_DELETESTRING,
                Some(WPARAM(sel as usize)),
                Some(LPARAM(0)),
            );
            sel
        };
        let _ =
            self.apply_translation_change(TranslationSettingChange::RemoveDeepLKey(sel as usize));
    }

    /// 번역 캐시 비우기 (확인 후 AppAction으로 전달)
    fn clear_translation_cache(&self) {
        let confirmed = unsafe {
            MessageBoxW(
                Some(self.hwnd),
                w!("저장된 번역 캐시를 모두 지우시겠습니까?"),
                w!("캐시 비우기"),
                MB_ICONQUESTION | MB_YESNO,
            )
        };
        if confirmed != IDYES {
            return;
        }
        if let Some(actions) = &self.actions {
            actions.clear_translation_cache();
        }
    }

    /// 글로서리 편집기 다이얼로그 열기
    fn open_glossary_editor(&mut self) {
        let draft = self.draft.clone();
        let _ = crate::dialogs::glossary::GlossaryDialog::show(self.hwnd, draft);
    }

    fn open_eztrans_dictionary_editor(&mut self) {
        let draft = self.draft.clone();
        let _ = crate::dialogs::glossary::GlossaryDialog::show_eztrans(self.hwnd, draft);
    }

    pub(super) fn refresh_glossary_count(&self) {
        let count = self.draft.borrow().translation.llm.glossary.len();
        self.set_control_text(
            ctrl_id::LLM_GLOSSARY_COUNT_LABEL,
            &format!("사전 항목: {}", count),
        );
    }

    pub(super) fn refresh_eztrans_dictionary_count(&self) {
        let count = self
            .draft
            .borrow()
            .translation
            .eztrans_postprocess_dictionary
            .len();
        self.set_control_text(
            ctrl_id::EZTRANS_DICTIONARY_COUNT_LABEL,
            &format!("후처리 사전: {count}"),
        );
    }

    /// 색상 버튼 처리
    fn handle_color_button(&mut self, ctrl_id: u16, text_type: TextType, color_type: ColorType) {
        let initial = self.draft.borrow().get_text_color(text_type, color_type);
        self.show_live_color_dialog(ctrl_id, initial, true, move |argb| {
            SettingsChange::TextColor {
                text_type,
                color_type,
                argb,
            }
        });
    }

    /// 선택 중에는 즉시 미리보기를 갱신하고, 취소하면 대화상자를 열기 전 색으로 복원한다.
    fn show_live_color_dialog<F>(
        &self,
        ctrl_id: u16,
        initial: u32,
        show_alpha: bool,
        make_change: F,
    ) where
        F: Fn(u32) -> SettingsChange + Copy + 'static,
    {
        let draft = self.draft.clone();
        let actions = self.actions.clone();
        let settings_hwnd = self.hwnd;
        let preview_change = make_change;
        let had_unapplied_changes = self.has_unapplied_changes.get();

        let result = ColorDialog::show(
            self.hwnd,
            ColorDialogConfig {
                initial_color: initial,
                on_color_change: Some(Box::new(move |argb| {
                    let result =
                        SettingsEditor::apply(&mut draft.borrow_mut(), preview_change(argb));
                    if result.preview_refresh_required
                        && let Some(actions) = &actions
                    {
                        actions.preview_settings(draft.borrow().clone());
                    }
                    SettingsDialog::invalidate_color_button_for(settings_hwnd, ctrl_id);
                })),
                no_activate: false,
                show_alpha,
            },
        );

        let final_color = result.map_or(initial, |result| result.argb);
        self.apply_settings_change(make_change(final_color));
        // callback에서 바뀐 값은 최종 apply가 no-op일 수 있고, 취소 복원은 새 변경이 아니다.
        self.has_unapplied_changes
            .set(had_unapplied_changes || (result.is_some() && final_color != initial));
        self.invalidate_color_button(ctrl_id);
    }

    /// 폰트 버튼 처리
    fn handle_font_button(&mut self, text_type: TextType) {
        let (face, style_bits, size) = {
            let cfg = self.draft.borrow();
            let style = cfg.get_text_style(text_type);
            (style.font_face.clone(), style.font_style, style.size)
        };

        let font_config = FontDialogConfig {
            initial_face: Some(face),
            initial_style: FontStyle::from_bits(style_bits),
            initial_point_size: size,
            no_activate: true,
        };

        if let Some(result) = FontDialog::show(self.hwnd, font_config) {
            self.apply_settings_change(SettingsChange::Font {
                text_type,
                face_name: result.face_name,
                style_bits: result.style.to_bits(),
            });
        }
    }

    /// 트랙바 변경 처리
    pub(super) fn handle_trackbar(&mut self, id: u16, value: i32) {
        use ctrl_id::*;

        if id == LLM_TEMPERATURE_TRACKBAR {
            let _ = self
                .apply_translation_change(TranslationSettingChange::LlmTemperatureSlider(value));
            let temp = self.draft.borrow().translation.llm.temperature;
            self.set_control_text(LLM_TEMPERATURE_LABEL, &format!("{:.2}", temp));
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
        unsafe {
            let combo = match GetDlgItem(Some(self.hwnd), id as i32) {
                Ok(h) if !h.is_invalid() => h,
                _ => return,
            };
            let sel =
                SendMessageW(combo, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 as usize;

            match id {
                TRANS_ENGINE => {
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
                }
                CUSTOM_API_SELECT => {
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
                    let _ = self.apply_translation_change(
                        TranslationSettingChange::SelectCustomApi(custom_name),
                    );
                }
                TRANS_SOURCE_LANG => {
                    let Ok(engine) = self.draft.borrow().translation.get_engine() else {
                        return;
                    };
                    if let Some(&language) = engine.supported_source_languages().get(sel)
                        && self
                            .apply_translation_change(TranslationSettingChange::SourceLanguage(
                                language,
                            ))
                            .is_ok()
                    {
                        self.refresh_language_combos(engine);
                    }
                }
                TRANS_TARGET_LANG => {
                    let Ok(engine) = self.draft.borrow().translation.get_engine() else {
                        return;
                    };
                    let Ok(source) = self.draft.borrow().translation.get_source_language() else {
                        return;
                    };
                    let targets = engine.supported_targets_for(source);
                    if let Some(&language) = targets.get(sel) {
                        let _ = self.apply_translation_change(
                            TranslationSettingChange::TargetLanguage(language),
                        );
                    }
                }
                LLM_PROVIDER => {
                    use crate::translation::LlmProvider;
                    let Some(provider) = LlmProvider::from_u8(sel as u8) else {
                        return;
                    };
                    let _ = self
                        .apply_translation_change(TranslationSettingChange::LlmProvider(provider));
                    if let Err(error) = self.refresh_llm_provider_controls(provider) {
                        tracing::warn!("LLM 제공자 설정 UI를 갱신할 수 없습니다: {error}");
                    }
                }
                LLM_MODEL_EDIT => {
                    let model = self.get_control_text(LLM_MODEL_EDIT);
                    let _ =
                        self.apply_translation_change(TranslationSettingChange::LlmModel(model));
                }
                LLM_REASONING_EFFORT => {
                    let effort = sel.checked_sub(1).and_then(|index| {
                        crate::translation::llm::ReasoningEffort::ALL
                            .get(index)
                            .copied()
                    });
                    let _ = self.apply_translation_change(
                        TranslationSettingChange::LlmReasoningEffort(effort),
                    );
                }
                DEEPL_STRATEGY_COMBO => {
                    let _ = self.apply_translation_change(
                        TranslationSettingChange::DeepLStrategyRoundRobin(sel == 1),
                    );
                }
                _ => {}
            }
        }
    }

    /// EzTrans 초기화 동기화 (다른 엔진은 워커가 매번 자격증명을 받아 stateless)
    fn sync_translation_manager(&self) {
        let config = self.draft.borrow();
        if let Err(e) = TranslationSettingsEditor::sync_runtime(&config.translation) {
            tracing::warn!("EzTrans init failed in sync: {e}");
        }
    }

    fn apply_settings_change(&self, change: SettingsChange) -> SettingsChangeResult {
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

    fn apply_translation_change(
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
        // SAFETY: self.hwnd is valid; GetDlgItem returns a valid control handle.
        unsafe {
            if let Ok(ctrl) = GetDlgItem(Some(self.hwnd), ctrl_id as i32)
                && !ctrl.is_invalid()
            {
                let _ = SetWindowTextW(ctrl, &HSTRING::from(text));
            }
        }
    }

    /// 제목 지정 폴더 브라우저 열기
    fn browse_folder_with_title(&self, title: &str) -> Result<Option<String>> {
        crate::dialogs::file_dialog::pick_folder(self.hwnd, title)
            .map(|path| path.map(|p| p.to_string_lossy().into_owned()))
    }

    /// DLL 파일 브라우저 열기
    fn browse_dll_file(&self, title: &str) -> Result<Option<String>> {
        let filters = [crate::dialogs::file_dialog::FileFilter {
            name: "DLL 파일",
            spec: "*.dll",
        }];
        crate::dialogs::file_dialog::open_file(self.hwnd, title, &filters)
            .map(|path| path.map(|p| p.to_string_lossy().into_owned()))
    }

    fn show_file_dialog_error(&self, error: &windows::core::Error) {
        tracing::error!("설정 파일 대화상자 오류: {error}");
        let message = HSTRING::from(format!("파일 대화상자를 열 수 없습니다.\n{error}"));
        unsafe {
            let _ = MessageBoxW(Some(self.hwnd), &message, w!("오류"), MB_ICONERROR);
        }
    }

    /// Edit 컨트롤 포커스 해제 시 값 저장
    fn handle_edit_killfocus(&mut self, ctrl_id: u16) {
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
        let change = match ctrl_id {
            PAPAGO_ID_EDIT => TranslationSettingChange::PapagoClientId(text),
            PAPAGO_SECRET_EDIT => TranslationSettingChange::PapagoClientSecret(text),
            EZTRANS_DLL_EDIT => TranslationSettingChange::EzTransDllPath(text),
            EZTRANS_DAT_EDIT => TranslationSettingChange::EzTransDatPath(text),
            LLM_MODEL_EDIT => TranslationSettingChange::LlmModel(text),
            LLM_API_KEY_EDIT => TranslationSettingChange::LlmApiKey(text),
            LLM_SYSTEM_PROMPT_EDIT => TranslationSettingChange::LlmSystemPrompt(text),
            LLM_MAX_TOKENS_EDIT => TranslationSettingChange::LlmMaxTokensText(text),
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
                LLM_DEBOUNCE_EDIT => {
                    let value = self.draft.borrow().translation.llm.debounce_ms;
                    self.set_control_text(ctrl_id, &value.to_string());
                }
                _ => {}
            }
        }
    }

    /// Edit 컨트롤에서 텍스트 가져오기
    fn get_control_text(&self, ctrl_id: u16) -> String {
        // SAFETY: self.hwnd is valid; GetDlgItem returns a valid control handle.
        unsafe {
            let ctrl = match GetDlgItem(Some(self.hwnd), ctrl_id as i32) {
                Ok(h) if !h.is_invalid() => h,
                _ => return String::new(),
            };
            let len = GetWindowTextLengthW(ctrl);
            if len == 0 {
                return String::new();
            }
            let mut buffer: Vec<u16> = vec![0; (len + 1) as usize];
            GetWindowTextW(ctrl, &mut buffer);
            String::from_utf16_lossy(&buffer[..len as usize])
        }
    }

    /// 트랙바 위치 업데이트
    fn update_trackbar_pos(&self, trackbar_id: u16, value: i32) {
        // SAFETY: self.hwnd is valid; GetDlgItem returns a valid control handle.
        unsafe {
            if let Ok(trackbar) = GetDlgItem(Some(self.hwnd), trackbar_id as i32)
                && !trackbar.is_invalid()
            {
                let _ = SendMessageW(
                    trackbar,
                    TBM_SETPOS,
                    Some(WPARAM(1)),
                    Some(LPARAM(value as isize)),
                );
            }
        }
    }

    fn update_numeric_ui(&self, trackbar_id: u16, edit_id: u16, value: i32) {
        self.update_trackbar_pos(trackbar_id, value);
        self.set_control_text(edit_id, &value.to_string());
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
