//! OpenAI 호환 챗 컴플리션 백엔드
//!
//! OpenAI / xAI Grok / OpenRouter 가 동일한 `POST {base}/chat/completions` 형식을 공유한다.
//! 본 모듈은 이 셋을 한 번에 처리한다.

use super::super::http_common::{LLM_REQUEST_TIMEOUT, send_and_read_body, validate_not_empty};
use super::super::{Language, TranslationError, TranslationResult};
use super::{LlmCallParams, LlmProvider, build_system_prompt_with_glossary};
use secrecy::ExposeSecret;
use serde::Serialize;

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [ChatMessage<'a>; 2],
    temperature: f32,
    max_tokens: u32,
}

fn request_payload<'a>(
    params: &'a LlmCallParams,
    system: &'a str,
    text: &'a str,
) -> ChatRequest<'a> {
    ChatRequest {
        model: params.effective_model(),
        messages: [
            ChatMessage {
                role: "system",
                content: system,
            },
            ChatMessage {
                role: "user",
                content: text,
            },
        ],
        temperature: params.temperature,
        max_tokens: params.max_tokens,
    }
}

/// OpenAI 호환 chat completions 요청
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

    let url = format!("{}/chat/completions", params.effective_base_url());
    let mut req = client
        .post(&url)
        .timeout(LLM_REQUEST_TIMEOUT)
        .bearer_auth(params.api_key.expose_secret())
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

    // OpenAI 표준 에러 형식 — choices 가 없을 때도 본문에 error 가 함께 올 수 있어
    // 텍스트 추출보다 먼저 검사하면 오해의 소지가 있으나, 정상 응답에는 error 가
    // 절대 동봉되지 않으므로 충돌 없음. 텍스트가 잡혔으면 정상으로 반환한다.
    let extracted = extract_message_text(value.pointer("/choices/0/message/content"));
    let finish_reason = value
        .pointer("/choices/0/finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if let Some(text) = extracted {
        let trimmed = text.trim().to_string();
        // finish_reason == "length" 는 max_tokens 한계로 응답이 절단됐다는 뜻.
        // 사용자에게 명시적으로 알려야 짤린 번역을 그대로 쓰는 사고를 막을 수 있다.
        if finish_reason == "length" {
            return Err(TranslationError::OutputTruncated {
                provider: "OpenAI 호환 API",
                reason: "finish_reason=length".to_string(),
            });
        }
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }

    // content 가 null/누락이지만 refusal 이 있으면 안전 정책에 의한 거부.
    if let Some(refusal) = value
        .pointer("/choices/0/message/refusal")
        .and_then(|v| v.as_str())
        && !refusal.is_empty()
    {
        return Err(TranslationError::Api {
            code: 0,
            message: format!("LLM 응답 거부됨: {refusal}"),
            retry_after: None,
        });
    }

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
        return Err(TranslationError::Api {
            code,
            message,
            retry_after: None,
        });
    }

    // finish_reason 이 비-STOP 인데 텍스트도 없으면 그 사실을 그대로 전달.
    if !finish_reason.is_empty() && finish_reason != "stop" {
        return Err(TranslationError::Api {
            code: 0,
            message: format!("LLM 응답에 텍스트 없음 (finish_reason={finish_reason})"),
            retry_after: None,
        });
    }

    Err(TranslationError::Parse(
        "LLM 응답에서 번역 결과를 찾을 수 없습니다.".to_string(),
    ))
}

/// `choices[0].message.content` 가 문자열인 일반 케이스와 멀티파트 배열인
/// 케이스 모두에서 텍스트를 추출. OpenAI 일부 모델 / OpenRouter 백엔드는
/// `content` 를 `[{ "type": "text", "text": "..." }, ...]` 형식으로 반환한다.
/// content 가 `null` 이면 `None`.
fn extract_message_text(content: Option<&serde_json::Value>) -> Option<String> {
    let v = content?;
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = v.as_array() {
        let mut out = String::new();
        for part in arr {
            // OpenAI spec: { "type": "text", "text": "..." } — type 이 없거나
            // 다른 값(예: "output_text") 이어도 text 필드가 있으면 누적.
            if let Some(t) = part.get("text").and_then(|x| x.as_str()) {
                out.push_str(t);
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    None
}

#[cfg(test)]
#[path = "../../../tests/unit/translation/llm/openai_compat.rs"]
mod tests;
