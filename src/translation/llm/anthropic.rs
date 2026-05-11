//! Anthropic Claude messages API 백엔드
//!
//! OpenAI 호환이 아니므로 페이로드 구조가 다르다:
//! - 인증: `x-api-key` 헤더 (Bearer 아님)
//! - 시스템 프롬프트: `messages` 밖의 별도 `system` 필드
//! - 응답: `content[0].text`

use isolang::Language;

use super::super::http_common::{send_and_read_body, validate_not_empty};
use super::super::{TranslationError, TranslationResult};
use super::{LlmCallParams, build_system_prompt_with_glossary};

const ANTHROPIC_VERSION: &str = "2023-06-01";

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
        "max_tokens": params.max_tokens,
        "temperature": params.temperature,
        "system": system,
        "messages": [
            { "role": "user", "content": text },
        ],
    });

    let url = format!("{}/messages", params.effective_base_url());
    let response = client
        .post(&url)
        .header("x-api-key", &params.api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| TranslationError::Network(e.to_string()))?;

    let body = send_and_read_body(response).await?;
    parse_messages_response(&body)
}

fn parse_messages_response(json: &str) -> TranslationResult {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| TranslationError::Parse(e.to_string()))?;

    if let Some(arr) = value.get("content").and_then(|v| v.as_array()) {
        let mut out = String::new();
        for block in arr {
            if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    out.push_str(t);
                }
            }
        }
        if !out.is_empty() {
            return Ok(out.trim().to_string());
        }
    }

    if let Some(err) = value.get("error") {
        let message = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Anthropic 응답 에러")
            .to_string();
        let kind = err.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let code = match kind {
            "authentication_error" | "permission_error" => 401,
            "rate_limit_error" => 429,
            "overloaded_error" => 529,
            "api_error" => 500,
            _ => 0,
        };
        return Err(TranslationError::Api { code, message });
    }

    Err(TranslationError::Parse(
        "Anthropic 응답에서 번역 결과를 찾을 수 없습니다.".to_string(),
    ))
}
