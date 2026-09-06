//! 재시도/폴백을 포함한 실제 번역 실행 (엔진별 dispatch).

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::TranslationRequest;
use super::dispatch::TranslationDispatch;
use crate::translation::llm::LlmProvider;
use crate::translation::{
    DeepLStrategy, Language, PreparedEngineKind, TranslationError, TranslationResult,
};

impl TranslationDispatch {
    /// 최대 재시도 횟수
    const MAX_RETRIES: u32 = 3;
    /// 초기 재시도 대기 시간 (밀리초)
    const INITIAL_BACKOFF_MS: u64 = 500;

    /// DeepL key를 전략에 따라 선택하고 한도/인증 오류면 다음 key로 넘어간다.
    pub(super) async fn translate_deepl_multi_key(
        client: &reqwest::Client,
        text: &str,
        source: Language,
        target: Language,
        keys: &[String],
        strategy: DeepLStrategy,
    ) -> TranslationResult {
        // PreparedJob 구성 단계(`engine_build.rs`)에서 이미 빈 키 목록을 걸러내지만,
        // 방어적으로 한 번 더 확인해 패닉 대신 오류로 처리한다.
        if keys.is_empty() {
            return Err(TranslationError::MissingApiKey);
        }

        // 비어 있는 키는 건너뛰고 시작 오프셋만 결정한다.
        let start = match strategy {
            DeepLStrategy::Failover => 0,
            DeepLStrategy::RoundRobin => {
                use std::sync::atomic::AtomicUsize;
                static COUNTER: AtomicUsize = AtomicUsize::new(0);
                COUNTER.fetch_add(1, Ordering::Relaxed) % keys.len()
            }
        };

        // 루프는 항상 마지막 offset에서 return하므로 아래로 빠지지 않지만,
        // 방어적으로 마지막 오류를 보관해 두었다가 fallback으로 반환한다.
        let mut last_err = None;
        for offset in 0..keys.len() {
            let idx = (start + offset) % keys.len();
            let key = &keys[idx];
            match super::super::deepl::translate_async_with_client(
                client, text, source, target, key,
            )
            .await
            {
                Ok(s) => return Ok(s),
                Err(e) => {
                    if Self::deepl_should_fallback(&e) && offset + 1 < keys.len() {
                        tracing::warn!(
                            key_index = idx,
                            category = e.log_category(),
                            status_code = ?e.log_status_code(),
                            "DeepL key failed; trying fallback"
                        );
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_err.unwrap_or(TranslationError::MissingApiKey))
    }

    /// 한도 초과/인증 실패는 다음 키로 폴백, 그 외(파싱/네트워크 등)는 즉시 반환
    pub(super) fn deepl_should_fallback(err: &TranslationError) -> bool {
        match err {
            TranslationError::Api { code, .. } | TranslationError::RateLimited { code, .. } => {
                matches!(code, 429 | 456 | 403)
            }
            _ => false,
        }
    }

    /// 재시도를 포함한 비동기 번역. 별도 runtime에서도 직접 호출할 수 있다.
    pub(crate) async fn translate_async(
        req: &TranslationRequest,
        client: &reqwest::Client,
    ) -> TranslationResult {
        let mut last_err = None;
        // Dispatch IDs are allocated by more than one producer in this
        // process. Use one process-global namespace for request idempotency,
        // and retain this value for every retry in this call.
        let request_id = super::super::mys_translater::next_request_id();

        let length = req.text.chars().count();
        let max = req.job.engine().max_input_chars();
        if length > max {
            return Err(TranslationError::InputTooLong {
                engine: req.job.engine().display_name(),
                length,
                max,
            });
        }

        for attempt in 0..=Self::MAX_RETRIES {
            if attempt > 0 {
                let server_delay = Self::retry_delay(last_err.as_ref(), attempt);
                let jitter = Duration::from_millis(
                    (req.id.wrapping_mul(37).wrapping_add(attempt as u64 * 101)) % 251,
                );
                let delay = server_delay.saturating_add(jitter);
                tracing::warn!(
                    attempt,
                    max_retries = Self::MAX_RETRIES,
                    delay_ms = delay.as_millis(),
                    "translation retry scheduled"
                );
                tokio::time::sleep(delay).await;
            }

            match Self::translate_once(req, client, request_id).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    if e.is_retryable() && attempt < Self::MAX_RETRIES {
                        tracing::warn!(
                            category = e.log_category(),
                            status_code = ?e.log_status_code(),
                            "retryable translation failure"
                        );
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| TranslationError::Engine("알 수 없는 오류".to_string())))
    }

    pub(super) fn retry_delay(error: Option<&TranslationError>, attempt: u32) -> Duration {
        error
            .and_then(TranslationError::retry_after)
            .unwrap_or_else(|| {
                Duration::from_millis(Self::INITIAL_BACKOFF_MS * 2u64.pow(attempt - 1))
            })
    }

    /// 단일 번역 시도
    async fn translate_once(
        req: &TranslationRequest,
        client: &reqwest::Client,
        request_id: u64,
    ) -> TranslationResult {
        let languages = req.job.languages();
        let result = match req.job.engine().kind() {
            PreparedEngineKind::EzTrans { .. } => {
                // Arc clone으로 본문 복사 없이 blocking task에 넘긴다.
                let text = std::sync::Arc::clone(&req.text);
                let source = languages.source();
                let target = languages.target();

                match tokio::task::spawn_blocking(move || {
                    super::super::translate_with_eztrans(&text, source, target)
                })
                .await
                {
                    Ok(result) => result,
                    Err(e) => Err(TranslationError::Engine(format!("EzTrans 실행 오류: {e}"))),
                }
            }
            PreparedEngineKind::Google => {
                super::super::google::translate_async_with_client(
                    client,
                    &req.text,
                    languages.source(),
                    languages.target(),
                )
                .await
            }
            PreparedEngineKind::DeepL { keys, strategy } => {
                Self::translate_deepl_multi_key(
                    client,
                    &req.text,
                    languages.source(),
                    languages.target(),
                    keys.as_slice(),
                    *strategy,
                )
                .await
            }
            PreparedEngineKind::Papago {
                client_id,
                client_secret,
            } => {
                super::super::papago_api::translate_async_with_client(
                    client,
                    &req.text,
                    languages.source(),
                    languages.target(),
                    client_id,
                    client_secret,
                )
                .await
            }
            PreparedEngineKind::Llm(params) => {
                let result = match params.provider {
                    LlmProvider::OpenAi | LlmProvider::Grok | LlmProvider::OpenRouter => {
                        super::super::llm::openai_compat::translate_async_with_client(
                            client,
                            &req.text,
                            languages.source(),
                            languages.target(),
                            params,
                        )
                        .await
                    }
                    LlmProvider::Anthropic => {
                        super::super::llm::anthropic::translate_async_with_client(
                            client,
                            &req.text,
                            languages.source(),
                            languages.target(),
                            params,
                        )
                        .await
                    }
                    LlmProvider::Gemini => {
                        super::super::llm::gemini::translate_async_with_client(
                            client,
                            &req.text,
                            languages.source(),
                            languages.target(),
                            params,
                        )
                        .await
                    }
                };
                if let Ok(ref output) = result {
                    super::super::llm::usage::record(req.text.len(), output.len());
                }
                result
            }
            PreparedEngineKind::MysTranslater(params) => {
                super::super::mys_translater::translate_async_with_client_for_request(
                    client,
                    &req.text,
                    languages.source(),
                    languages.target(),
                    params,
                    request_id,
                )
                .await
            }
            PreparedEngineKind::Custom(params) => {
                super::super::custom::translate_async_with_client(
                    client,
                    &req.text,
                    languages.source(),
                    languages.target(),
                    params,
                )
                .await
            }
        };
        result.map(|translated| req.job.postprocess(translated))
    }
}
