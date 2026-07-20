use std::sync::Arc;

use super::{App, state};
use crate::clipboard::ClipboardUpdate;
use crate::dialogs::LogEntry;
use crate::translation::TranslationEngine;

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
        if !self.model.config.clipboard_watch || !self.clipboard.is_watching() {
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
                let max_len = self.model.config.clipboard_max_length as usize;
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
                    let config = &self.model.config;
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

        self.model.runtime.clipboard_debounce.submit(text);
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
        if let Some(text) = self.model.runtime.clipboard_debounce.take() {
            self.request_translation_async(&text);
        }
    }

    pub(super) fn cancel_clipboard_translation(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::KillTimer;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), super::CLIPBOARD_DEBOUNCE_TIMER);
            let _ = KillTimer(Some(self.hwnd), super::CLIPBOARD_READ_RETRY_TIMER);
        }
        self.model.runtime.clipboard_debounce.clear();
        self.model.runtime.pending_translation = None;
        self.services.translation_ui.cancel(self.hwnd);
    }

    /// 비동기 번역 요청
    fn request_translation_async(&mut self, text: &str) {
        use crate::translation::PreparedJob;

        let cache_enabled = self.model.config.clipboard_cache_enabled;
        let job = match PreparedJob::from_config(&self.model.config.translation) {
            Ok(spec) => spec,
            Err(error) => {
                tracing::warn!("자동 번역 요청을 구성할 수 없습니다: {error}");
                return;
            }
        };

        let cache_key = job.cache_key(text);

        if cache_enabled && let Some(cached) = self.services.translation_cache.get(&cache_key) {
            tracing::debug!("clipboard translation cache hit");
            self.model.runtime.pending_translation = None;
            self.model.runtime.original_text = text.to_string();
            self.model.runtime.translated_text = cached.clone();
            self.push_backlog(LogEntry::new(text.to_string()).with_translation(cached));
            if let Err(e) = self.paint() {
                tracing::warn!("paint failed during cached translation: {e}");
            }
            return;
        }

        if let Err(error) = job.prepare() {
            tracing::warn!("자동 번역 엔진을 준비할 수 없습니다: {error}");
            return;
        }

        let original: Arc<str> = Arc::from(text);

        // 디스패치에 번역 요청 (워커는 프로세스 전역)
        let request = self
            .services
            .translation_ui
            .request(self.hwnd, Arc::clone(&original), job);

        match request {
            Ok(req_id) => {
                self.model.runtime.pending_translation = Some(state::PendingTranslation::new(
                    req_id,
                    original,
                    cache_key,
                ));
                self.model.runtime.original_text = text.to_string();
                self.model.runtime.translated_text = "[번역 중...]".to_string();
            }
            Err(error) => {
                tracing::error!("Translation request failed: {error}");
                self.model.runtime.pending_translation = None;
                self.model.runtime.original_text = text.to_string();
                self.model.runtime.translated_text.clear();
                self.push_backlog(LogEntry::new(text.to_string()));
            }
        }

        if let Err(e) = self.paint() {
            tracing::warn!("paint failed during translation: {e}");
        }
    }

    /// 대상별 완료 큐에서 원래 64-bit request ID와 응답을 함께 꺼낸다.
    pub(super) fn handle_translation_complete(&mut self) {
        let Some((req_id, response)) = self.services.translation_ui.take_response(self.hwnd) else {
            return;
        };

        let Some(completion) = state::correlate_translation(
            &mut self.model.runtime.pending_translation,
            req_id,
            response.result,
        ) else {
            tracing::debug!("Ignoring stale translation response: req_id={req_id}");
            return;
        };

        let original = completion.original;
        let translation = match completion.result {
            Ok(translated) => {
                self.model.runtime.original_text = original.to_string();
                self.model.runtime.translated_text = translated.clone();
                if self.model.config.clipboard_cache_enabled {
                    self.services
                        .translation_cache
                        .put(&completion.cache_key, &translated);
                }
                Some(translated)
            }
            Err(err) => {
                log_translation_failure(&err);
                self.model.runtime.original_text = original.to_string();
                self.model.runtime.translated_text.clear();
                None
            }
        };

        let mut entry = LogEntry::new(original.to_string());
        if let Some(trans) = translation {
            entry = entry.with_translation(trans);
        }
        self.push_backlog(entry);

        if let Err(e) = self.paint() {
            tracing::warn!("paint failed after translation complete: {e}");
        }
    }

    fn push_backlog(&mut self, entry: LogEntry) {
        let evicted = self.model.backlog.push(entry.clone());
        crate::dialogs::backlog::append_entry(entry, evicted);
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app/translation.rs"]
mod tests;
