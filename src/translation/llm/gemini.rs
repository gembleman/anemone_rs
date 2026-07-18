//! Google Gemini (AI Studio) generateContent 백엔드
//!
//! - 인증: `x-goog-api-key` 헤더 (AI Studio 키 — Vertex AI OAuth 와 다름)
//! - 페이로드: `system_instruction` + `contents` + `generationConfig`
//! - 응답: `candidates[0].content.parts[*].text`

use super::super::http_common::{LLM_REQUEST_TIMEOUT, send_and_read_body, validate_not_empty};
use super::super::{Language, TranslationError, TranslationResult};
use super::{LlmCallParams, build_system_prompt_with_glossary};
use secrecy::ExposeSecret;
use serde::Serialize;

#[derive(Serialize)]
struct TextPart<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct Parts<'a> {
    parts: [TextPart<'a>; 1],
}

#[derive(Serialize)]
struct Content<'a> {
    role: &'static str,
    parts: [TextPart<'a>; 1],
}

#[derive(Serialize)]
struct GenerationConfig {
    temperature: f32,
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
}

#[derive(Serialize)]
struct GenerateContentRequest<'a> {
    system_instruction: Parts<'a>,
    contents: [Content<'a>; 1],
    #[serde(rename = "generationConfig")]
    generation_config: GenerationConfig,
}

fn request_payload<'a>(
    params: &LlmCallParams,
    system: &'a str,
    text: &'a str,
) -> GenerateContentRequest<'a> {
    GenerateContentRequest {
        system_instruction: Parts {
            parts: [TextPart { text: system }],
        },
        contents: [Content {
            role: "user",
            parts: [TextPart { text }],
        }],
        generation_config: GenerationConfig {
            temperature: params.temperature,
            max_output_tokens: params.max_tokens,
        },
    }
}

pub async fn translate_async_with_client(
    client: &reqwest::Client,
    text: &str,
    source: Language,
    target: Language,
    params: &LlmCallParams,
) -> TranslationResult {
    validate_not_empty(text)?;

    if params.api_key.expose_secret().is_empty() {
        return Err(TranslationError::MissingApiKey);
    }

    let system = build_system_prompt_with_glossary(
        params.effective_system_prompt(),
        source,
        target,
        &params.glossary,
    );
    let payload = request_payload(params, &system, text);

    let url = format!(
        "{}/models/{}:generateContent",
        params.effective_base_url(),
        params.effective_model()
    );

    // API 키는 헤더로 전달 (URL 쿼리스트링에 노출되는 것보다 안전하고,
    // reqwest의 `query()` 기능에 의존하지 않는다)
    let response = client
        .post(&url)
        .timeout(LLM_REQUEST_TIMEOUT)
        .header("x-goog-api-key", params.api_key.expose_secret())
        .json(&payload)
        .send()
        .await
        .map_err(|e| TranslationError::Network(e.to_string()))?;

    let body = send_and_read_body(response).await?;
    parse_generate_content_response(&body)
}

fn parse_generate_content_response(json: &str) -> TranslationResult {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| TranslationError::Parse(e.to_string()))?;

    let finish_reason = value
        .pointer("/candidates/0/finishReason")
        .and_then(|v| v.as_str());
    if finish_reason == Some("MAX_TOKENS") {
        return Err(TranslationError::OutputTruncated {
            provider: "Gemini",
            reason: "finishReason=MAX_TOKENS".to_string(),
        });
    }
    if let Some(reason) = finish_reason
        && reason != "STOP"
    {
        return Err(TranslationError::Api {
            code: 0,
            message: format!("Gemini 응답 중단: {reason}"),
            retry_after: None,
        });
    }

    if let Some(parts) = value
        .pointer("/candidates/0/content/parts")
        .and_then(|v| v.as_array())
    {
        let mut out = String::new();
        for p in parts {
            if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                out.push_str(t);
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
            .unwrap_or("Gemini 응답 에러")
            .to_string();
        let code = err.get("code").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
        return Err(TranslationError::Api {
            code,
            message,
            retry_after: None,
        });
    }

    Err(TranslationError::Parse(
        "Gemini 응답에서 번역 결과를 찾을 수 없습니다.".to_string(),
    ))
}

#[cfg(test)]
#[path = "../../../tests/unit/translation/llm/gemini.rs"]
mod tests;
