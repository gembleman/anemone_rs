//! Google Translate API (비공식 웹 API)
//!
//! Google Translate 비공식 웹 API를 사용한 번역
//! 비동기 HTTPS 요청으로 UI 블로킹 없이 번역 수행

use super::{TranslationError, TranslationResult, Translator, lang_utils};
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
        if text.is_empty() {
            return Err(TranslationError::EmptyText);
        }

        // 동기 번역은 blocking으로 수행 (하위 호환용)
        // 실제 사용은 translate_async 권장
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| TranslationError::Engine(format!("런타임 생성 실패: {}", e)))?;

        rt.block_on(translate_async(text, source, target))
    }

    fn engine_name(&self) -> &'static str {
        "Google Translate"
    }

    fn is_available(&self) -> bool {
        // 네트워크 연결 확인 없이 항상 true
        // 실제 번역 시 오류 처리
        true
    }
}

/// 비동기 번역 함수 (워커에서 호출용)
pub async fn translate_async(
    text: &str,
    source: Language,
    target: Language,
) -> TranslationResult {
    if text.is_empty() {
        return Err(TranslationError::EmptyText);
    }

    let source_code = lang_utils::to_google_code(source);
    let target_code = lang_utils::to_google_code(target);

    call_google_api(text, source_code, target_code).await
}

/// Google Translate API 호출 (HTTPS)
async fn call_google_api(
    text: &str,
    source_lang: &str,
    target_lang: &str,
) -> TranslationResult {
    // URL 인코딩
    let encoded_text = url_encode(text);

    // Google Translate 비공식 API URL
    let url = format!(
        "https://translate.googleapis.com/translate_a/single?client=gtx&sl={}&tl={}&dt=t&q={}",
        source_lang, target_lang, encoded_text
    );

    // HTTP 요청
    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .send()
        .await
        .map_err(|e| TranslationError::Network(e.to_string()))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| TranslationError::Network(e.to_string()))?;

    if !status.is_success() {
        return Err(TranslationError::Api {
            code: status.as_u16(),
            message: body,
        });
    }

    // 응답 파싱
    parse_google_response(&body)
}

/// URL 인코딩
fn url_encode(s: &str) -> String {
    let mut result = String::new();
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => {
                result.push_str("%20");
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

/// Google API 응답 파싱
fn parse_google_response(json: &str) -> TranslationResult {
    // Google Translate API 응답 형식:
    // [[["번역결과","원문",null,null,10],...],...]
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| TranslationError::Parse(e.to_string()))?;

    let mut result = String::new();

    // 첫 번째 배열이 번역 결과들
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
