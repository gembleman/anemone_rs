//! 색상/글꼴/파일 대화상자를 여는 버튼과 DeepL 키 목록, EzTrans 경로 경고.

use windows_sys::Win32::{
    Graphics::Gdi::InvalidateRect, UI::Controls::*, UI::WindowsAndMessaging::*,
};

use super::model::{SettingsChange, SettingsEditor};
use super::secret_format::format_deepl_key;
use super::{SettingsDialog, ctrl_id};
use crate::config::{ColorType, TextType};
use crate::dialogs::color::{ColorDialog, ColorDialogConfig};
use crate::dialogs::font::{FontDialog, FontDialogConfig, FontStyle};
use crate::translation::settings::TranslationSettingChange;
use crate::win32::to_wide;
type Result<T> = windows_core::Result<T>;

impl SettingsDialog {
    /// 체크박스 상태에 맞춰 자격 증명 입력란의 마스킹을 켜고 끈다.
    pub(super) fn toggle_secret_visibility(&self, checkbox_id: u16, secret_edit_id: u16) {
        let Ok(checkbox) = self.control(checkbox_id) else {
            return;
        };
        let Ok(secret_edit) = self.control(secret_edit_id) else {
            return;
        };
        // SAFETY: 두 HWND는 현재 설정 대화상자가 소유한 유효한 컨트롤이다.
        unsafe {
            let visible = SendMessageW(checkbox, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
            let password_char = if visible { 0 } else { '●' as usize };
            let _ = SendMessageW(secret_edit, EM_SETPASSWORDCHAR, password_char, 0);
            let _ = InvalidateRect(secret_edit, std::ptr::null(), 1);
        }
    }

    /// 입력한 DeepL key를 list와 config에 추가한다.
    pub(super) fn deepl_keys_add(&mut self) {
        let key = self.get_control_text(ctrl_id::DEEPL_KEY_ADD_EDIT);
        let key = key.trim().to_string();
        if key.is_empty() {
            return;
        }
        let tier = unsafe {
            let combo = GetDlgItem(self.hwnd, ctrl_id::DEEPL_KEY_TIER_COMBO as i32);
            if combo.is_null() {
                return;
            }
            match SendMessageW(combo, CB_GETCURSEL, 0, 0) {
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
            let listbox = GetDlgItem(self.hwnd, ctrl_id::DEEPL_KEYS_LIST as i32);
            if listbox.is_null() {
                return;
            }
            let key_wide = to_wide(&format_deepl_key(&key));
            let _ = SendMessageW(listbox, LB_ADDSTRING, 0, key_wide.as_ptr() as isize);
        }
        self.set_control_text(ctrl_id::DEEPL_KEY_ADD_EDIT, "");
    }

    /// 선택한 DeepL key를 list와 config에서 제거한다.
    pub(super) fn deepl_keys_remove(&mut self) {
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid listbox.
        let sel = unsafe {
            let listbox = GetDlgItem(self.hwnd, ctrl_id::DEEPL_KEYS_LIST as i32);
            if listbox.is_null() {
                return;
            }
            let sel = SendMessageW(listbox, LB_GETCURSEL, 0, 0) as i32;
            if sel == LB_ERR {
                return;
            }
            let _ = SendMessageW(listbox, LB_DELETESTRING, sel as usize, 0);
            sel
        };
        let _ =
            self.apply_translation_change(TranslationSettingChange::RemoveDeepLKey(sel as usize));
    }

    /// 번역 캐시 비우기 (확인 후 AppAction으로 전달)
    pub(super) fn clear_translation_cache(&self) {
        let confirmed = unsafe {
            MessageBoxW(
                self.hwnd,
                crate::win32::to_wide("저장된 번역 캐시를 모두 지우시겠습니까?").as_ptr(),
                crate::win32::to_wide("캐시 비우기").as_ptr(),
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

    /// "업데이트 확인" 버튼 처리. 직전 수동 확인이 새 버전을 찾아 두었다면
    /// 다운로드·적용을 요청하고, 그렇지 않으면 새로 확인을 요청한다.
    pub(super) fn handle_update_button(&mut self) {
        let Some(actions) = &self.actions else {
            return;
        };
        if self.update_available.get() {
            actions.request_update_apply();
        } else {
            actions.request_update_check();
        }
    }

    /// 글로서리 편집기 다이얼로그 열기
    pub(super) fn open_glossary_editor(&mut self) {
        let draft = self.draft.clone();
        let _ = crate::dialogs::glossary::GlossaryDialog::show(self.hwnd, draft);
    }

    pub(super) fn open_eztrans_dictionary_editor(&mut self) {
        let draft = self.draft.clone();
        let _ = crate::dialogs::glossary::GlossaryDialog::show_eztrans(self.hwnd, draft);
    }

    pub(super) fn refresh_glossary_count(&self) {
        let count = self.draft.borrow().translation.llm.glossary.len();
        self.set_control_text(
            ctrl_id::LLM_GLOSSARY_COUNT_LABEL,
            &format!("사전 항목: {count}"),
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
    pub(super) fn handle_color_button(
        &mut self,
        ctrl_id: u16,
        text_type: TextType,
        color_type: ColorType,
    ) {
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
    pub(super) fn show_live_color_dialog<F>(
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
    pub(super) fn handle_font_button(&mut self, text_type: TextType) {
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

    /// 제목 지정 폴더 브라우저 열기
    pub(super) fn browse_folder_with_title(&self, title: &str) -> Result<Option<String>> {
        crate::dialogs::file_dialog::pick_folder(self.hwnd, title)
            .map(|path| path.map(|p| p.to_string_lossy().into_owned()))
    }

    /// 평면 사전 파일 브라우저 열기
    pub(super) fn browse_dictionary_file(&self, title: &str) -> Result<Option<String>> {
        let filters = [crate::dialogs::file_dialog::FileFilter {
            name: "EzTrans 평면 사전",
            spec: "*.bin",
        }];
        crate::dialogs::file_dialog::open_file(self.hwnd, title, &filters)
            .map(|path| path.map(|p| p.to_string_lossy().into_owned()))
    }

    pub(super) fn show_file_dialog_error(&self, error: &windows_core::Error) {
        tracing::error!("설정 파일 대화상자 오류: {error}");
        let message = to_wide(&format!("파일 대화상자를 열 수 없습니다.\n{error}"));
        unsafe {
            let _ = MessageBoxW(
                self.hwnd,
                message.as_ptr(),
                crate::win32::to_wide("오류").as_ptr(),
                MB_ICONERROR,
            );
        }
    }
}
