use super::{App, state};
use crate::dialogs::{LogEntry, add_to_backlog};
use crate::translation::{request_translation, take_response};

impl App {
    pub(super) fn handle_clipboard_change(&mut self) {
        if let Some(text) = self.clipboard.on_clipboard_update() {
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

            // 클립보드 텍스트 처리
            tracing::debug!("Clipboard: {}", text);

            // 자동 번역 처리 (비동기)
            self.request_translation_async(&text);
        }
    }

    /// 비동기 번역 요청
    fn request_translation_async(&mut self, text: &str) {
        use crate::translation::TranslationJobSpec;

        let config = self.config.borrow();
        let spec = match TranslationJobSpec::from_config(&config.translation) {
            Ok(spec) => spec,
            Err(error) => {
                tracing::warn!("자동 번역 요청을 구성할 수 없습니다: {error}");
                return;
            }
        };
        if let Err(error) = spec.prepare() {
            tracing::warn!("자동 번역 엔진을 준비할 수 없습니다: {error}");
            return;
        }

        drop(config);

        // 디스패치에 번역 요청 (워커는 프로세스 전역)
        let request = request_translation(
            self.hwnd,
            text.to_string(),
            spec.engine(),
            spec.source_lang(),
            spec.target_lang(),
            spec.credentials(),
        );

        match request {
            Ok(req_id) => {
                self.state.pending_translation =
                    Some(state::PendingTranslation::new(req_id, text.to_string()));
                self.state.current_text = format!("[번역 중...]\n{text}");
            }
            Err(error) => {
                tracing::error!("Translation request failed: {error}");
                self.state.pending_translation = None;
                self.state.current_text = text.to_string();
                add_to_backlog(&self.backlog_store, LogEntry::new(text.to_string()));
            }
        }

        if let Err(e) = self.paint() {
            tracing::warn!("paint failed during translation: {e}");
        }
    }

    /// 번역 완료 처리
    ///
    /// `WM_TRANSLATION_COMPLETE` 의 WPARAM 으로 전달된 `req_id` 에 해당하는
    /// 응답만 꺼낸다. 디스패치가 hwnd 기준으로 라우팅하므로 다른 다이얼로그의
    /// 응답이 섞일 일은 없지만, 동일 hwnd 에 누적된 응답 중에서도 정확히
    /// 매칭된 한 건만 처리한다.
    pub(super) fn handle_translation_complete(&mut self, req_id: u64) {
        let Some(response) = take_response(req_id) else {
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

        let translation = match completion.result {
            Ok(translated) => {
                self.state.current_text = translated.clone();
                Some(translated)
            }
            Err(err) => {
                tracing::error!("Translation error: {}", err);
                self.state.current_text = completion.original.clone();
                None
            }
        };

        let mut entry = LogEntry::new(completion.original);
        if let Some(trans) = translation {
            entry = entry.with_translation(trans);
        }
        add_to_backlog(&self.backlog_store, entry);

        if let Err(e) = self.paint() {
            tracing::warn!("paint failed after translation complete: {e}");
        }
    }
}
