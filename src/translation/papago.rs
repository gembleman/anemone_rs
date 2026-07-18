//! Papago (네이버) 번역 엔진
//!
//! Ncloud Papago Text Translation API. 워커 스레드에서 호출되는 async 함수만 제공한다.

use super::Language;
use super::http_common::{send_and_read_body, validate_not_empty};
use super::{TranslationError, TranslationResult, lang_utils};
use secrecy::{ExposeSecret, SecretString};

const ENDPOINT: &str = "https://papago.apigw.ntruss.com/nmt/v1/translation";

/// 공유 Client를 받는 비동기 번역 함수
pub async fn translate_async_with_client(
    client: &reqwest::Client,
    text: &str,
    source: Language,
    target: Language,
    client_id: &SecretString,
    client_secret: &SecretString,
) -> TranslationResult {
    validate_not_empty(text)?;
    let client_id = client_id.expose_secret();
    let client_secret = client_secret.expose_secret();

    if !super::TranslationEngine::Papago.supports_pair(source, target) {
        return Err(TranslationError::UnsupportedLanguagePair);
    }

    if client_id.is_empty() || client_secret.is_empty() {
        return Err(TranslationError::MissingApiKey);
    }

    let source_code = lang_utils::to_papago_code(source)?;
    let target_code = lang_utils::to_papago_code(target)?;

    let params = [
        ("source", source_code),
        ("target", target_code),
        ("text", text),
    ];

    let response = client
        .post(ENDPOINT)
        .header("X-NCP-APIGW-API-KEY-ID", client_id)
        .header("X-NCP-APIGW-API-KEY", client_secret)
        .form(&params)
        .send()
        .await
        .map_err(|e| TranslationError::Network(e.to_string()))?;

    let body = send_and_read_body(response).await?;
    parse_papago_response(&body)
}

fn parse_papago_response(json: &str) -> TranslationResult {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| TranslationError::Parse(e.to_string()))?;

    if let Some(text) = value
        .pointer("/message/result/translatedText")
        .and_then(|v| v.as_str())
    {
        return Ok(text.to_string());
    }

    if let Some(msg) = value.get("errorMessage").and_then(|v| v.as_str()) {
        let code = value
            .get("errorCode")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        return Err(TranslationError::Api {
            code,
            message: msg.to_string(),
            retry_after: None,
        });
    }

    Err(TranslationError::Parse(
        "Papago 응답에서 번역 결과를 찾을 수 없습니다.".to_string(),
    ))
}
