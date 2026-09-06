//! DeepL API 번역 엔진
//!
//! DeepL 공식 API 비동기 번역. 워커 스레드에서 호출되는 async 함수만 제공한다.

use super::Language;
use super::http_common::{send_and_read_body, validate_not_empty};
use super::{TranslationError, TranslationResult, lang_utils};
use reqwest::header::{AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};

const DEEPL_PRO_TRANSLATE_URL: &str = "https://api.deepl.com/v2/translate";
const DEEPL_FREE_TRANSLATE_URL: &str = "https://api-free.deepl.com/v2/translate";
const DEEPL_REQUEST_BODY_LIMIT: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeepLApiTier {
    Free,
    Pro,
}

impl DeepLApiTier {
    pub(crate) fn from_api_key(api_key: &str) -> Self {
        if api_key.trim().ends_with(":fx") {
            Self::Free
        } else {
            Self::Pro
        }
    }

    pub(crate) const fn display_name(self) -> &'static str {
        match self {
            Self::Free => "무료",
            Self::Pro => "유료",
        }
    }
}

#[derive(Serialize)]
struct DeepLRequest<'a> {
    text: [&'a str; 1],
    source_lang: &'static str,
    target_lang: &'static str,
}

#[derive(Deserialize)]
struct DeepLResponse {
    translations: Vec<DeepLTranslation>,
}

#[derive(Deserialize)]
struct DeepLTranslation {
    text: String,
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

    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(TranslationError::MissingApiKey);
    }

    let source_code = lang_utils::to_deepl_code(source)?;
    let target_code = lang_utils::to_deepl_code(target)?;

    let request = build_deepl_request(
        client,
        deepl_translate_url(api_key),
        text,
        source_code,
        target_code,
        api_key,
    )?;
    let response = client
        .execute(request)
        .await
        .map_err(|e| TranslationError::Network(e.to_string()))?;

    let body = send_and_read_body(response).await?;
    parse_deepl_response(&body)
}

fn deepl_translate_url(api_key: &str) -> &'static str {
    match DeepLApiTier::from_api_key(api_key) {
        DeepLApiTier::Free => DEEPL_FREE_TRANSLATE_URL,
        DeepLApiTier::Pro => DEEPL_PRO_TRANSLATE_URL,
    }
}

fn build_deepl_request(
    client: &reqwest::Client,
    url: &str,
    text: &str,
    source_lang: &'static str,
    target_lang: &'static str,
    api_key: &str,
) -> Result<reqwest::Request, TranslationError> {
    let body = DeepLRequest {
        text: [text],
        source_lang,
        target_lang,
    };
    let request = client
        .post(url)
        .header(AUTHORIZATION, format!("DeepL-Auth-Key {api_key}"))
        .header(USER_AGENT, concat!("AnemoneRS/", env!("CARGO_PKG_VERSION")))
        .json(&body)
        .build()
        .map_err(|error| TranslationError::Network(error.to_string()))?;

    let body_length = request
        .body()
        .and_then(reqwest::Body::as_bytes)
        .map_or(0, <[u8]>::len);
    if body_length > DEEPL_REQUEST_BODY_LIMIT {
        return Err(TranslationError::RequestTooLarge {
            engine: "DeepL",
            bytes: body_length,
            max_bytes: DEEPL_REQUEST_BODY_LIMIT,
        });
    }

    Ok(request)
}

/// DeepL API 응답 파싱
fn parse_deepl_response(json: &str) -> TranslationResult {
    let response: DeepLResponse =
        serde_json::from_str(json).map_err(|e| TranslationError::Parse(e.to_string()))?;
    response
        .translations
        .into_iter()
        .next()
        .map(|translation| translation.text)
        .ok_or_else(|| TranslationError::Parse("번역 결과를 찾을 수 없습니다.".to_string()))
}

#[cfg(test)]
#[path = "../../tests/unit/translation/deepl.rs"]
mod tests;
