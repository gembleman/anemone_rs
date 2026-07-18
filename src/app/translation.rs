use std::sync::Arc;

use super::{App, state};
use crate::clipboard::ClipboardUpdate;
use crate::dialogs::{LogEntry, add_to_backlog};
use crate::translation::TranslationEngine;
use crate::translation_ui::{request_translation, take_response};

fn debounce_delay_ms(engine: TranslationEngine, configured_ms: u32) -> u32 {
    if engine == TranslationEngine::Llm {
        configured_ms
    } else {
        0
    }
}

fn log_clipboard_metadata(text: &str) {
    tracing::debug!(
        clipboard_chars = text.chars().count(),
        "clipboard text detected"
    );
}

fn log_translation_failure(error: &crate::translation::TranslationError) {
    tracing::error!(
        category = error.log_category(),
        status_code = ?error.log_status_code(),
        "translation failed"
    );
}

impl App {
    pub(super) fn handle_clipboard_change(&mut self) {
        if !self.config.borrow().clipboard_watch || !self.clipboard.is_watching() {
            return;
        }
        match self.clipboard.on_clipboard_update() {
            ClipboardUpdate::Unchanged => {}
            ClipboardUpdate::Retry(error) => {
                tracing::debug!("clipboard read failed temporarily; retrying: {error}");
                let timer = unsafe {
                    windows::Win32::UI::WindowsAndMessaging::SetTimer(
                        Some(self.hwnd),
                        super::CLIPBOARD_READ_RETRY_TIMER,
                        50,
                        None,
                    )
                };
                if timer == 0 {
                    tracing::warn!("clipboard read retry timer could not be created");
                }
            }
            ClipboardUpdate::Failed(error) => {
                tracing::warn!("clipboard read failed after retries: {error}");
            }
            ClipboardUpdate::Text(text) => {
                let max_len = self.config.borrow().clipboard_max_length as usize;
                if max_len > 0 {
                    let text_len = text.chars().count();
                    if text_len > max_len {
                        tracing::debug!(
                            "Clipboard text skipped: length {} exceeds clipboard_max_length {}",
                            text_len,
                            max_len
                        );
                        return;
                    }
                }

                log_clipboard_metadata(&text);

                let delay_ms = {
                    let config = self.config.borrow();
                    let engine = match config.translation.get_engine() {
                        Ok(engine) => engine,
                        Err(error) => {
                            tracing::error!("자동 번역 설정 오류: {error}");
                            return;
                        }
                    };
                    debounce_delay_ms(engine, config.translation.llm.debounce_ms)
                };
                self.schedule_clipboard_translation(text, delay_ms);
            }
        }
    }

    fn schedule_clipboard_translation(&mut self, text: String, debounce_ms: u32) {
        use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

        self.state.clipboard_debounce.submit(text);
        unsafe {
            let _ = KillTimer(Some(self.hwnd), super::CLIPBOARD_DEBOUNCE_TIMER);
            let _ = KillTimer(Some(self.hwnd), super::CLIPBOARD_READ_RETRY_TIMER);
        }
        if debounce_ms == 0 {
            self.handle_clipboard_debounce_timer();
            return;
        }

        let timer = unsafe {
            SetTimer(
                Some(self.hwnd),
                super::CLIPBOARD_DEBOUNCE_TIMER,
                debounce_ms,
                None,
            )
        };
        if timer == 0 {
            tracing::warn!(
                "clipboard debounce timer could not be created; dispatching immediately"
            );
            self.handle_clipboard_debounce_timer();
        }
    }

    pub(super) fn handle_clipboard_debounce_timer(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::KillTimer;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), super::CLIPBOARD_DEBOUNCE_TIMER);
            let _ = KillTimer(Some(self.hwnd), super::CLIPBOARD_READ_RETRY_TIMER);
        }
        if let Some(text) = self.state.clipboard_debounce.take() {
            self.request_translation_async(&text);
        }
    }

    pub(super) fn cancel_clipboard_translation(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::KillTimer;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), super::CLIPBOARD_DEBOUNCE_TIMER);
            let _ = KillTimer(Some(self.hwnd), super::CLIPBOARD_READ_RETRY_TIMER);
        }
        self.state.clipboard_debounce.clear();
        self.state.pending_translation = None;
        crate::translation_ui::cancel_translation(self.hwnd);
    }

    /// 비동기 번역 요청
    fn request_translation_async(&mut self, text: &str) {
        use crate::translation::PreparedJob;

        let config = self.config.borrow();
        let job = match PreparedJob::from_config(&config.translation) {
            Ok(spec) => spec,
            Err(error) => {
                tracing::warn!("자동 번역 요청을 구성할 수 없습니다: {error}");
                return;
            }
        };
        if let Err(error) = job.prepare() {
            tracing::warn!("자동 번역 엔진을 준비할 수 없습니다: {error}");
            return;
        }

        drop(config);

        let original: Arc<str> = Arc::from(text);

        // 디스패치에 번역 요청 (워커는 프로세스 전역)
        let request = request_translation(self.hwnd, Arc::clone(&original), job);

        match request {
            Ok(req_id) => {
                self.state.pending_translation =
                    Some(state::PendingTranslation::new(req_id, original));
                self.state.original_text = text.to_string();
                self.state.translated_text = "[번역 중...]".to_string();
            }
            Err(error) => {
                tracing::error!("Translation request failed: {error}");
                self.state.pending_translation = None;
                self.state.original_text = text.to_string();
                self.state.translated_text.clear();
                add_to_backlog(&self.backlog_store, LogEntry::new(text.to_string()));
            }
        }

        if let Err(e) = self.paint() {
            tracing::warn!("paint failed during translation: {e}");
        }
    }

    /// 대상별 완료 큐에서 원래 64-bit request ID와 응답을 함께 꺼낸다.
    pub(super) fn handle_translation_complete(&mut self) {
        let Some((req_id, response)) = take_response(self.hwnd) else {
            return;
        };

        let Some(completion) = state::correlate_translation(
            &mut self.state.pending_translation,
            req_id,
            response.result,
        ) else {
            tracing::debug!("Ignoring stale translation response: req_id={req_id}");
            return;
        };

        let original = completion.original;
        let translation = match completion.result {
            Ok(translated) => {
                self.state.original_text = original.to_string();
                self.state.translated_text = translated.clone();
                Some(translated)
            }
            Err(err) => {
                log_translation_failure(&err);
                self.state.original_text = original.to_string();
                self.state.translated_text.clear();
                None
            }
        };

        let mut entry = LogEntry::new(original.to_string());
        if let Some(trans) = translation {
            entry = entry.with_translation(trans);
        }
        add_to_backlog(&self.backlog_store, entry);

        if let Err(e) = self.paint() {
            tracing::warn!("paint failed after translation complete: {e}");
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app/translation.rs"]
mod tests;
