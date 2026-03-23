//! Google Translate API (비공식 웹 API)
//!
//! Google Translate 비공식 웹 API를 사용한 번역
//! 비동기 HTTPS 요청으로 UI 블로킹 없이 번역 수행

use super::{TranslationError, TranslationResult, Translator, lang_utils};
use super::http_common::{block_on_async, send_and_read_body, validate_not_empty};
use isolang::Language;

/// Google 번역기
pub struct GoogleTranslator;

impl GoogleTranslator {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GoogleTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl Translator for GoogleTranslator {
    fn translate(&self, text: &str, source: Language, target: Language) -> TranslationResult {
        validate_not_empty(text)?;
        block_on_async(|| translate_async(text, source, target))
    }

    fn engine_name(&self) -> &'static str {
        "Google Translate"
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// 비동기 번역 함수 (워커에서 호출용)
pub async fn translate_async(
    text: &str,
    source: Language,
    target: Language,
) -> TranslationResult {
    translate_async_with_client(&super::http_common::shared_client(), text, source, target).await
}

/// 공유 Client를 받는 비동기 번역 함수
pub async fn translate_async_with_client(
    client: &reqwest::Client,
    text: &str,
    source: Language,
    target: Language,
) -> TranslationResult {
    validate_not_empty(text)?;

    let source_code = lang_utils::to_google_code(source);
    let target_code = lang_utils::to_google_code(target);

    let encoded_text = url_encode(text);
    let url = format!(
        "https://translate.googleapis.com/translate_a/single?client=gtx&sl={}&tl={}&dt=t&q={}",
        source_code, target_code, encoded_text
    );

    let response = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .send()
        .await
        .map_err(|e| TranslationError::Network(e.to_string()))?;

    let body = send_and_read_body(response).await?;
    parse_google_response(&body)
}

/// URL 인코딩
fn url_encode(s: &str) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(s.len() * 2);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => {
                result.push_str("%20");
            }
            _ => {
                let _ = write!(result, "%{:02X}", byte);
            }
        }
    }
    result
}

/// Google API 응답 파싱
fn parse_google_response(json: &str) -> TranslationResult {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| TranslationError::Parse(e.to_string()))?;

    let mut result = String::with_capacity(json.len() / 4);

    if let Some(outer) = value.as_array() {
        if let Some(first) = outer.first() {
            if let Some(translations) = first.as_array() {
                for item in translations {
                    if let Some(inner) = item.as_array() {
                        if let Some(first_elem) = inner.first() {
                            if let Some(translated) = first_elem.as_str() {
                                result.push_str(translated);
                            }
                        }
                    }
                }
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
