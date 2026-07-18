//! Google Translate API (비공식 웹 API)
//!
//! Google Translate 비공식 웹 API를 사용한 번역.
//! 워커 스레드에서 호출되는 async 함수만 제공한다.

use super::Language;
use super::http_common::{send_and_read_body, validate_not_empty};
use super::{TranslationError, TranslationResult, lang_utils};

/// 공유 Client를 받는 비동기 번역 함수
pub async fn translate_async_with_client(
    client: &reqwest::Client,
    text: &str,
    source: Language,
    target: Language,
) -> TranslationResult {
    validate_not_empty(text)?;

    let source_code = lang_utils::to_google_code(source)?;
    let target_code = lang_utils::to_google_code(target)?;

    let url = format!(
        "https://translate.googleapis.com/translate_a/single?client=gtx&sl={source_code}&tl={target_code}&dt=t"
    );
    let response = client
        .post(url)
        // 원문은 URL이 아니라 form body에 둔다. 긴 UTF-8 문자열의 URL 팽창과
        // 프록시/진단 로그 노출을 피하기 위함이다.
        .form(&[("q", text)])
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .send()
        .await
        .map_err(|e| TranslationError::Network(e.to_string()))?;

    let body = send_and_read_body(response).await?;
    parse_google_response(&body)
}

/// Google API 응답 파싱
fn parse_google_response(json: &str) -> TranslationResult {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| TranslationError::Parse(e.to_string()))?;

    let mut result = String::with_capacity(json.len() / 4);

    if let Some(outer) = value.as_array()
        && let Some(first) = outer.first()
        && let Some(translations) = first.as_array()
    {
        for item in translations {
            if let Some(inner) = item.as_array()
                && let Some(first_elem) = inner.first()
                && let Some(translated) = first_elem.as_str()
            {
                result.push_str(translated);
            }
        }
    }

    if result.is_empty() {
        Err(TranslationError::Parse(
            "번역 결과를 찾을 수 없습니다.".to_string(),
        ))
    } else {
        Ok(result)
    }
}
