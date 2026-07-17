use super::*;

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
        use crate::translation::{EngineCredentials, TranslationEngine, get_eztrans_manager};

        let config = self.config.borrow();
        let engine = config.translation.get_engine();
        let source_lang = config.translation.get_source_language();
        let target_lang = config.translation.get_target_language();
        let credentials = match engine {
            TranslationEngine::DeepL => EngineCredentials::DeepL {
                keys: config.translation.deepl_effective_keys(),
                strategy: config.translation.deepl_strategy(),
            },
            TranslationEngine::Papago => EngineCredentials::Papago {
                client_id: config.translation.papago_client_id.clone(),
                client_secret: config.translation.papago_client_secret.clone(),
            },
            TranslationEngine::Llm => {
                EngineCredentials::Llm(config.translation.llm.to_call_params())
            }
            _ => EngineCredentials::None,
        };

        // EzTrans 초기화 (필요시)
        if engine == TranslationEngine::EzTrans && !config.translation.eztrans_dll_path.is_empty() {
            let manager = get_eztrans_manager();
            if let Ok(mut mgr) = manager.lock()
                && let Err(e) = mgr.init(
                    &config.translation.eztrans_dll_path,
                    &config.translation.eztrans_dat_path,
                )
            {
                tracing::warn!("EzTrans init failed: {e}");
            }
        }

        drop(config);

        // 원문 저장 (번역 완료 시 백로그에 추가)
        self.pending_original_text = Some(text.to_string());

        // 번역 중 표시
        self.current_text = format!("[번역 중...]\n{}", text);
        if let Err(e) = self.paint() {
            tracing::warn!("paint failed during translation: {e}");
        }

        // 디스패치에 번역 요청 (워커는 프로세스 전역)
        request_translation(
            self.hwnd,
            text.to_string(),
            engine,
            source_lang,
            target_lang,
            credentials,
        );
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

        let translation = match response.result {
            Ok(translated) => {
                self.current_text = translated.clone();
                Some(translated)
            }
            Err(err) => {
                tracing::error!("Translation error: {}", err);
                if let Some(ref original) = self.pending_original_text {
                    self.current_text = original.clone();
                }
                None
            }
        };

        if let Some(original) = self.pending_original_text.take() {
            let mut entry = LogEntry::new(original);
            if let Some(trans) = translation {
                entry = entry.with_translation(trans);
            }
            add_to_backlog(entry);
        }

        if let Err(e) = self.paint() {
            tracing::warn!("paint failed after translation complete: {e}");
        }
    }
}
