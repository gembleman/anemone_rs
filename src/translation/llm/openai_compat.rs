//! OpenAI 호환 챗 컴플리션 백엔드
//!
//! OpenAI / xAI Grok / OpenRouter 가 동일한 `POST {base}/chat/completions` 형식을 공유한다.
//! 본 모듈은 이 셋을 한 번에 처리한다.

use isolang::Language;

use super::super::http_common::{LLM_REQUEST_TIMEOUT, send_and_read_body, validate_not_empty};
use super::super::{TranslationError, TranslationResult};
use super::{LlmCallParams, LlmProvider, build_system_prompt_with_glossary};

/// OpenAI 호환 chat completions 요청
pub async fn translate_async_with_client(
    client: &reqwest::Client,
    text: &str,
    source: Language,
    target: Language,
    params: &LlmCallParams,
) -> TranslationResult {
    validate_not_empty(text)?;

    if params.api_key.is_empty() {
        return Err(TranslationError::MissingApiKey);
    }

    let system =
        build_system_prompt_with_glossary(&params.system_prompt, source, target, &params.glossary);
    let payload = serde_json::json!({
        "model": params.effective_model(),
        "messages": [
            { "role": "system", "content": system },
            { "role": "user",   "content": text },
        ],
        "temperature": params.temperature,
        "max_tokens": params.max_tokens,
    });

    let url = format!("{}/chat/completions", params.effective_base_url());
    let mut req = client
        .post(&url)
        .timeout(LLM_REQUEST_TIMEOUT)
        .bearer_auth(&params.api_key)
        .json(&payload);

    // OpenRouter는 출처 헤더를 권장한다 (rate limit 우대 / 통계용)
    if params.provider == LlmProvider::OpenRouter {
        req = req
            .header("HTTP-Referer", "https://github.com/gembleman/anemone_rs")
            .header("X-Title", "Anemone");
    }

    let response = req
        .send()
        .await
        .map_err(|e| TranslationError::Network(e.to_string()))?;

    let body = send_and_read_body(response).await?;
    parse_chat_completion(&body)
}

fn parse_chat_completion(json: &str) -> TranslationResult {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| TranslationError::Parse(e.to_string()))?;

    if let Some(text) = value
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
    {
        return Ok(text.trim().to_string());
    }

    // OpenAI 표준 에러 형식
    if let Some(err) = value.get("error") {
        let message = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("LLM 응답 에러")
            .to_string();
        let code = err
            .get("code")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        return Err(TranslationError::Api { code, message });
    }

    Err(TranslationError::Parse(
        "LLM 응답에서 번역 결과를 찾을 수 없습니다.".to_string(),
    ))
}
