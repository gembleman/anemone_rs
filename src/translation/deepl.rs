//! DeepL API 번역 엔진
//!
//! DeepL 공식 API를 사용한 번역 (API 키 필요)
//! 비동기 HTTP 요청으로 UI 블로킹 없이 번역 수행

use super::{TranslationError, TranslationResult, Translator, lang_utils};
use super::http_common::{block_on_async, send_and_read_body, validate_not_empty};
use isolang::Language;

/// DeepL 번역기
pub struct DeepLTranslator {
    api_key: String,
}

impl DeepLTranslator {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    /// API 키 설정
    pub fn set_api_key(&mut self, api_key: String) {
        self.api_key = api_key;
    }

    /// API 키 가져오기
    pub fn api_key(&self) -> &str {
        &self.api_key
    }
}

impl Translator for DeepLTranslator {
    fn translate(&self, text: &str, source: Language, target: Language) -> TranslationResult {
        validate_not_empty(text)?;
        if self.api_key.is_empty() {
            return Err(TranslationError::MissingApiKey);
        }
        let api_key = self.api_key.clone();
        block_on_async(|| translate_async(text, source, target, &api_key))
    }

    fn engine_name(&self) -> &'static str {
        "DeepL"
    }

    fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }
}

/// 비동기 번역 함수 (워커에서 호출용)
pub async fn translate_async(
    text: &str,
    source: Language,
    target: Language,
    api_key: &str,
) -> TranslationResult {
    translate_async_with_client(&super::http_common::shared_client(), text, source, target, api_key).await
}

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

    let source_code = lang_utils::to_deepl_code(source);
    let target_code = lang_utils::to_deepl_code(target);

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
    if let Some(translations) = value.get("translations") {
        if let Some(first) = translations.get(0) {
            if let Some(text) = first.get("text") {
                if let Some(s) = text.as_str() {
                    return Ok(s.to_string());
                }
            }
        }
    }

    // 에러 메시지 확인
    if let Some(message) = value.get("message") {
        if let Some(s) = message.as_str() {
            return Err(TranslationError::Api {
                code: 0,
                message: s.to_string(),
            });
        }
    }

    Err(TranslationError::Parse(
        "번역 결과를 찾을 수 없습니다.".to_string(),
    ))
}
