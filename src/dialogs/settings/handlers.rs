//! 설정 대화상자 명령/이벤트 핸들러

use windows::{
    Win32::{Foundation::*, UI::Controls::*, UI::WindowsAndMessaging::*},
    core::*,
};

use super::ctrl_id;
use super::{SettingsDialog, format_deepl_key};
use crate::config::{ColorType, TextAlign, TextType};
use crate::dialogs::color::{ColorDialog, ColorDialogConfig};
use crate::dialogs::font::{FontDialog, FontDialogConfig, FontStyle};
use crate::dialogs::models::SettingsDraft;
use crate::settings_model::{
    BoolSetting, NumericSetting, SettingsChange, SettingsChangeResult, SettingsEditor,
};
use crate::translation::settings::{
    SettingsApplyResult, TranslationSettingChange, TranslationSettingsEditor,
    TranslationSettingsError,
};
use crate::util::to_wide;

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

/// +/- 버튼 처리 매크로: config에서 값을 읽고, 범위 내에서 증감 후, UI 업데이트
macro_rules! handle_size_button {
    ($self:expr, $get_field:expr, $set_color_type:expr, $delta:expr, $ui_update:expr) => {{
        let requested = $get_field(&$self.draft.borrow()) + $delta;
        $self.apply_settings_change(SettingsChange::Numeric {
            setting: NumericSetting::TextSize($set_color_type),
            value: requested,
        });
        let new_size = $get_field(&$self.draft.borrow());
        $ui_update($self, new_size);
    }};
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

            // 텍스트 크기 +/-
            TEXTSIZE_MINUS => handle_size_button!(
                self,
                |cfg: &crate::config::Config| cfg.translation_style.size,
                ColorType::Primary,
                -1,
                |s: &Self, v| s.update_textsize_ui(v)
            ),
            TEXTSIZE_PLUS => handle_size_button!(
                self,
                |cfg: &crate::config::Config| cfg.translation_style.size,
                ColorType::Primary,
                1,
                |s: &Self, v| s.update_textsize_ui(v)
            ),

            // 외곽선1 +/-
            OUTLINE1_MINUS => handle_size_button!(
                self,
                |cfg: &crate::config::Config| cfg.translation_style.outline1_size,
                ColorType::Outline1,
                -1,
                |s: &Self, v| s.update_trackbar_pos(OUTLINE1_TRACKBAR, v)
            ),
            OUTLINE1_PLUS => handle_size_button!(
                self,
                |cfg: &crate::config::Config| cfg.translation_style.outline1_size,
                ColorType::Outline1,
                1,
                |s: &Self, v| s.update_trackbar_pos(OUTLINE1_TRACKBAR, v)
            ),

            // 외곽선2 +/-
            OUTLINE2_MINUS => handle_size_button!(
                self,
                |cfg: &crate::config::Config| cfg.translation_style.outline2_size,
                ColorType::Outline2,
                -1,
                |s: &Self, v| s.update_trackbar_pos(OUTLINE2_TRACKBAR, v)
            ),
            OUTLINE2_PLUS => handle_size_button!(
                self,
                |cfg: &crate::config::Config| cfg.translation_style.outline2_size,
                ColorType::Outline2,
                1,
                |s: &Self, v| s.update_trackbar_pos(OUTLINE2_TRACKBAR, v)
            ),

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

    /// 글로서리 편집기 다이얼로그 열기
    fn open_glossary_editor(&mut self) {
        let draft = self.draft.clone();
        let _ = crate::dialogs::glossary::GlossaryDialog::show(self.hwnd, draft);
    }

    pub(super) fn refresh_glossary_count(&self) {
        let count = self.draft.borrow().translation.llm.glossary.len();
        self.set_control_text(
            ctrl_id::LLM_GLOSSARY_COUNT_LABEL,
            &format!("사전 항목: {}", count),
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

        let setting = match id {
            BACKGROUND_TRACKBAR => NumericSetting::BackgroundAlpha,
            TEXTSIZE_TRACKBAR => NumericSetting::TextSize(ColorType::Primary),
            OUTLINE1_TRACKBAR => NumericSetting::TextSize(ColorType::Outline1),
            OUTLINE2_TRACKBAR => NumericSetting::TextSize(ColorType::Outline2),
            SHADOW_X_TRACKBAR => NumericSetting::ShadowOffsetX,
            SHADOW_Y_TRACKBAR => NumericSetting::ShadowOffsetY,
            MARGIN_X_TRACKBAR => NumericSetting::TextMarginX,
            MARGIN_Y_TRACKBAR => NumericSetting::TextMarginY,
            MARGIN_NAME_TRACKBAR => NumericSetting::NameMargin,
            BORDER_SIZE_TRACKBAR => NumericSetting::BorderWidth,
            LLM_TEMPERATURE_TRACKBAR => {
                let _ = self.apply_translation_change(
                    TranslationSettingChange::LlmTemperatureSlider(value),
                );
                let temp = self.draft.borrow().translation.llm.temperature;
                self.set_control_text(LLM_TEMPERATURE_LABEL, &format!("{:.2}", temp));
                return;
            }
            _ => return,
        };
        self.apply_settings_change(SettingsChange::Numeric { setting, value });
        if id == TEXTSIZE_TRACKBAR {
            let size = self.draft.borrow().translation_style.size;
            self.set_control_text(TEXTSIZE_TEXT, &format!("크기: {size}"));
        }
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
    ) -> std::result::Result<SettingsApplyResult, TranslationSettingsError> {
        let result =
            TranslationSettingsEditor::apply(&mut self.draft.borrow_mut().translation, change)?;
        self.finish_translation_change(result);
        Ok(result)
    }

    fn finish_translation_change(&self, result: SettingsApplyResult) {
        if result.runtime_sync_required {
            self.sync_translation_manager();
        }
        if result.preview_refresh_required {
            self.notify_preview();
        }
        if result.save_required {
            self.has_unapplied_changes.set(true);
        }
    }

    /// 컨트롤 텍스트 설정 헬퍼
    fn set_control_text(&self, ctrl_id: u16, text: &str) {
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
        use ctrl_id::*;
        let change = match ctrl_id {
            DEEPL_API_KEY_EDIT => TranslationSettingChange::DeepLApiKey(text),
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

    /// 텍스트 크기 UI 업데이트 (트랙바 위치 및 레이블)
    fn update_textsize_ui(&self, size: i32) {
        self.update_trackbar_pos(ctrl_id::TEXTSIZE_TRACKBAR, size);
        self.set_control_text(ctrl_id::TEXTSIZE_TEXT, &format!("크기: {}", size));
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
