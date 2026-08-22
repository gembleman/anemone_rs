use std::sync::Arc;

use super::backlog::LogEntry;
use super::{App, state};
use crate::clipboard::ClipboardUpdate;
use crate::translation::TranslationEngine;

/// 유료/원격 HTTP 엔진(Google/DeepL/Papago/Custom)의 클립보드 연속 변경 코얼레싱
/// 지연. LLM은 설정값을 따르고, 로컬 무료인 EzTrans만 즉시 번역을 유지한다.
/// 이 엔진들은 클립보드가 연속 변경되면 취소될 요청도 이미 발신되어 과금/트래픽
/// 낭비로 이어지므로, 짧은 디바운스로 발신 자체를 막는다. Custom은 사용자가
/// 지정한 임의 HTTP 엔드포인트라 유료 LLM 프록시일 가능성이 높아 EzTrans가
/// 아니라 이 그룹에 둔다.
const PAID_ENGINE_DEBOUNCE_MS: u32 = 150;

fn debounce_delay_ms(engine: TranslationEngine, configured_ms: u32) -> u32 {
    match engine {
        TranslationEngine::Llm => configured_ms,
        TranslationEngine::Google
        | TranslationEngine::DeepL
        | TranslationEngine::Papago
        | TranslationEngine::Custom => PAID_ENGINE_DEBOUNCE_MS,
        TranslationEngine::EzTrans => 0,
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
        let max_len = self.model.config.clipboard_max_length as usize;
        match self.clipboard.on_clipboard_update(max_len) {
            ClipboardUpdate::Unchanged => {}
            ClipboardUpdate::TooLong => {
                tracing::debug!("Clipboard text skipped: exceeds clipboard_max_length {max_len}");
            }
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
                    match config.translation.get_engine() {
                        // 디바운스 지연만 정하는 값이므로, 설정이 잘못됐어도 캡처는 계속한다.
                        // 실제 오류 보고는 번역 요청 단계에서 이뤄진다.
                        Ok(engine) => debounce_delay_ms(engine, config.translation.llm.debounce_ms),
                        Err(error) => {
                            tracing::error!("자동 번역 설정 오류: {error}");
                            0
                        }
                    }
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
    pub(super) fn request_translation_async(&mut self, text: &str) {
        use crate::translation::PreparedJob;

        let cache_enabled = self.model.config.clipboard_cache_enabled;
        // 전역 재사용: EzTrans + 후처리 사전을 쓰면 매 요청 사전 clone과
        // automaton 재빌드가 일어나는데(캐시 hit여도), 같은 설정의 작업은
        // 공유 인스턴스를 돌려받아 이 비용을 설정 변경 시 1회로 줄인다.
        let job = match PreparedJob::from_config_cached(&self.model.config.translation) {
            Ok(spec) => spec,
            Err(error) => {
                tracing::warn!("자동 번역 요청을 구성할 수 없습니다: {error}");
                self.capture_without_translation(text);
                return;
            }
        };

        let cache_key = job.cache_key(text);

        if cache_enabled && let Some(cached) = self.services.translation_cache.get(&cache_key) {
            tracing::debug!("clipboard translation cache hit");
            // 캐시 히트로 새 요청을 보내지 않으면 워커의 supersede가 일어나지 않으므로,
            // in-flight 상태였던 이전 요청을 직접 취소해 유료 엔진 낭비 호출을 막는다.
            self.services.translation_ui.cancel(self.hwnd);
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
            self.capture_without_translation(text);
            return;
        }

        let original: Arc<str> = Arc::from(text);

        // 디스패치에 번역 요청 (워커는 프로세스 전역)
        let request =
            self.services
                .translation_ui
                .request(self.hwnd, Arc::clone(&original), (*job).clone());

        match request {
            Ok(req_id) => {
                self.model.runtime.pending_translation =
                    Some(state::PendingTranslation::new(req_id, original, cache_key));
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

    /// 번역 엔진을 쓸 수 없을 때도 클립보드 캡처 결과는 남긴다.
    ///
    /// 번역 설정/엔진 준비가 실패해도 캡처 자체는 정상 동작해야 하므로,
    /// 번역 요청 실패와 같은 방식으로 원문만 표시하고 백로그에 기록한다.
    fn capture_without_translation(&mut self, text: &str) {
        self.model.runtime.pending_translation = None;
        self.model.runtime.original_text = text.to_string();
        self.model.runtime.translated_text.clear();
        self.push_backlog(LogEntry::new(text.to_string()));
        if let Err(e) = self.paint() {
            tracing::warn!("paint failed during untranslated capture: {e}");
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
