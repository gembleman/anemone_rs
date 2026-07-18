//! DeepL API 번역 엔진
//!
//! DeepL 공식 API 비동기 번역. 워커 스레드에서 호출되는 async 함수만 제공한다.

use super::Language;
use super::http_common::{send_and_read_body, validate_not_empty};
use super::{TranslationError, TranslationResult, lang_utils};

/// 공유 Client를 받는 비동기 번역 함수
pub async fn translate_async_with_client(
    client: &reqwest::Client,
    text: &str,
    source: Language,
    target: Language,
    api_key: &str,
) -> TranslationResult {
    validate_not_empty(text)?;

    if api_key.is_empty() {
        return Err(TranslationError::MissingApiKey);
    }

    let source_code = lang_utils::to_deepl_code(source)?;
    let target_code = lang_utils::to_deepl_code(target)?;

    // API 엔드포인트 결정 (Free API vs Pro API)
    let base_url = if api_key.ends_with(":fx") {
        "https://api-free.deepl.com"
    } else {
        "https://api.deepl.com"
    };

    let url = format!("{}/v2/translate", base_url);
    let params = [
        ("auth_key", api_key),
        ("text", text),
        ("source_lang", source_code),
        ("target_lang", target_code),
    ];

    let response = client
        .post(&url)
        .form(&params)
        .header("User-Agent", "AnemoneRS/1.0")
        .send()
        .await
        .map_err(|e| TranslationError::Network(e.to_string()))?;

    let body = send_and_read_body(response).await?;
    parse_deepl_response(&body)
}

/// DeepL API 응답 파싱
fn parse_deepl_response(json: &str) -> TranslationResult {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| TranslationError::Parse(e.to_string()))?;

    // translations[0].text 추출
    if let Some(translations) = value.get("translations")
        && let Some(first) = translations.get(0)
        && let Some(text) = first.get("text")
        && let Some(s) = text.as_str()
    {
        return Ok(s.to_string());
    }

    // 에러 메시지 확인
    if let Some(message) = value.get("message")
        && let Some(s) = message.as_str()
    {
        return Err(TranslationError::Api {
            code: 0,
            message: s.to_string(),
            retry_after: None,
        });
    }

    Err(TranslationError::Parse(
        "번역 결과를 찾을 수 없습니다.".to_string(),
    ))
}
