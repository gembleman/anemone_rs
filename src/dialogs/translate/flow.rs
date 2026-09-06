//! 번역 요청/응답 흐름, 클립보드 복사, 입력 초기화.

use windows_sys::Win32::{
    Foundation::HWND,
    UI::Input::KeyboardAndMouse::SetFocus,
    UI::WindowsAndMessaging::{CB_GETCURSEL, KillTimer, SendMessageW, SetTimer},
};

use std::sync::Arc;

use super::{AUTO_TRANSLATE_TIMER, CUSTOM_ENGINE_INDEX, TranslateDialog, engine_from_combo_index};
use crate::dialogs::helpers::{get_window_text, set_window_text};
use crate::translation::{PreparedJob, TranslationEngine};

impl TranslateDialog {
    /// 원문 텍스트 가져오기
    fn get_source_text(&self) -> String {
        // SAFETY: self.source_edit is a valid edit control handle from the resource template.
        get_window_text(self.source_edit)
    }

    /// 번역 결과 설정
    fn set_dest_text(&self, text: &str) {
        // SAFETY: self.dest_edit is a valid edit control handle from the resource template.
        let _ = set_window_text(self.dest_edit, text);
    }

    pub(super) fn schedule_auto_translate(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, AUTO_TRANSLATE_TIMER);
        }
        let engine_idx = unsafe { SendMessageW(self.engine_combo, CB_GETCURSEL, 0, 0) as u8 };
        let Some(engine) = engine_from_combo_index(engine_idx as usize) else {
            return;
        };
        let delay_ms = if engine == TranslationEngine::Llm {
            self.config.translation.llm.debounce_ms
        } else {
            0
        };
        if delay_ms == 0 {
            self.do_translate();
        } else {
            let timer = unsafe { SetTimer(self.hwnd, AUTO_TRANSLATE_TIMER, delay_ms, None) };
            if timer == 0 {
                tracing::warn!(
                    "auto-translate timer could not be created; dispatching immediately"
                );
                self.do_translate();
            }
        }
    }

    /// 현재 선택된 엔진/언어를 매니저에 적용
    pub(super) fn apply_current_settings(&mut self) {
        // SAFETY: combo handles are valid controls from the resource template. SendMessageW with
        // CB_GETCURSEL returns the current selection index.
        unsafe {
            let engine_idx = SendMessageW(self.engine_combo, CB_GETCURSEL, 0, 0) as usize;
            let source_idx = SendMessageW(self.source_lang_combo, CB_GETCURSEL, 0, 0) as usize;
            let target_idx = SendMessageW(self.target_lang_combo, CB_GETCURSEL, 0, 0) as usize;

            let Some(engine) = engine_from_combo_index(engine_idx) else {
                tracing::error!("잘못된 번역 엔진 콤보 선택: {engine_idx}");
                return;
            };
            let supported_source = engine.supported_source_languages();
            let Some(source_lang) = supported_source.get(source_idx).copied() else {
                tracing::error!("잘못된 소스 언어 콤보 선택: {source_idx}");
                return;
            };
            let supported_target = engine.supported_targets_for(source_lang);
            let Some(target_lang) = supported_target.get(target_idx).copied() else {
                tracing::error!("잘못된 대상 언어 콤보 선택: {target_idx}");
                return;
            };

            let config = &mut self.config;
            if engine == TranslationEngine::Custom {
                let custom_index = engine_idx - CUSTOM_ENGINE_INDEX;
                let custom_name = if config.translation.custom_apis.is_empty() {
                    (custom_index == 0).then(|| config.translation.custom.name.clone())
                } else {
                    config
                        .translation
                        .custom_apis
                        .get(custom_index)
                        .map(|api| api.name.clone())
                };
                let Some(custom_name) = custom_name else {
                    tracing::error!("잘못된 Custom API 콤보 선택: {custom_index}");
                    return;
                };
                if let Err(error) = config.translation.select_custom_api(&custom_name) {
                    tracing::error!("Custom API 선택 실패: {error}");
                    return;
                }
            }
            config.translation.set_engine(engine);
            config.translation.set_source_language(source_lang);
            config.translation.set_target_language(target_lang);
        }
    }

    /// 번역 수행 (비동기)
    pub(super) fn do_translate(&mut self) {
        let source = self.get_source_text();
        if source.is_empty() {
            return;
        }
        self.apply_current_settings();

        let text = self.manual_options.prepare_input(&source);

        let spec = {
            let config = &self.config;
            PreparedJob::from_config(&config.translation)
        };
        let spec = match spec {
            Ok(spec) => spec,
            Err(error) => {
                self.set_dest_text(&format!("[오류] {error}"));
                return;
            }
        };
        if let Err(error) = spec.prepare() {
            self.set_dest_text(&format!("[오류] {error}"));
            return;
        }

        self.set_dest_text("[번역 중...]");
        match self
            .translation_service
            .request(self.hwnd, Arc::from(text), spec)
        {
            Ok(req_id) => {
                self.in_flight_id = Some(req_id);
                self.last_submitted_source = source;
            }
            Err(error) => {
                self.in_flight_id = None;
                self.set_dest_text(&format!("[오류] {error}"));
            }
        }
    }

    /// 번역 완료 처리. WPARAM 의 `req_id` 로 자신의 응답만 꺼낸다.
    pub(super) fn handle_translation_complete(&mut self) {
        let Some((req_id, response)) = self.translation_service.take_response(self.hwnd) else {
            return;
        };
        if self.in_flight_id != Some(req_id) {
            return;
        }
        if self.get_source_text() != self.last_submitted_source {
            self.in_flight_id = None;
            return;
        }
        self.in_flight_id = None;

        let result = match response.result {
            Ok(translated) => self.manual_options.format_output(translated),
            Err(err) => format!("[오류] {err}"),
        };
        self.set_dest_text(&result);
    }

    /// 번역 결과를 클립보드에 복사
    pub(super) fn copy_to_clipboard(&self) {
        let text = get_window_text(self.dest_edit);
        if text.is_empty() {
            return;
        }
        Self::set_clipboard_text(&text, self.hwnd);
    }

    /// 클립보드에 텍스트 설정
    fn set_clipboard_text(text: &str, hwnd: HWND) {
        if let Err(error) = crate::clipboard::set_text(hwnd, text) {
            tracing::warn!("클립보드 복사 실패: {error}");
        }
    }

    /// 텍스트 초기화
    pub(super) fn clear_text(&mut self) {
        self.invalidate_translation_route();
        // SAFETY: source_edit and dest_edit are valid edit control handles.
        unsafe {
            let _ = set_window_text(self.source_edit, "");
            let _ = set_window_text(self.dest_edit, "");
            let _ = SetFocus(self.source_edit);
        }
    }

    pub(super) fn invalidate_stale_translation(&mut self) {
        if self.in_flight_id.is_some() && self.get_source_text() != self.last_submitted_source {
            self.invalidate_translation_route();
        }
    }

    pub(super) fn invalidate_translation_route(&mut self) {
        if self.in_flight_id.take().is_some() {
            self.translation_service.cancel(self.hwnd);
        }
        self.last_submitted_source.clear();
    }
}
