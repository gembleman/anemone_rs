//! Papago (네이버) 번역 엔진
//!
//! Naver 개발자센터 Papago N2MT API. Client ID/Secret 필요.

use super::http_common::{block_on_async, send_and_read_body, shared_client, validate_not_empty};
use super::{TranslationError, TranslationResult, Translator, lang_utils};
use isolang::Language;

const ENDPOINT: &str = "https://openapi.naver.com/v1/papago/n2mt";

/// Papago 번역기
pub struct PapagoTranslator {
    client_id: String,
    client_secret: String,
}

impl PapagoTranslator {
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client_id,
            client_secret,
        }
    }

    pub fn set_credentials(&mut self, client_id: String, client_secret: String) {
        self.client_id = client_id;
        self.client_secret = client_secret;
    }
}

impl Translator for PapagoTranslator {
    fn translate(&self, text: &str, source: Language, target: Language) -> TranslationResult {
        validate_not_empty(text)?;
        if self.client_id.is_empty() || self.client_secret.is_empty() {
            return Err(TranslationError::MissingApiKey);
        }
        let id = self.client_id.clone();
        let secret = self.client_secret.clone();
        block_on_async(move || async move {
            translate_async_with_client(&shared_client(), text, source, target, &id, &secret).await
        })
    }

    fn engine_name(&self) -> &'static str {
        "Papago"
    }

    fn is_available(&self) -> bool {
        !self.client_id.is_empty() && !self.client_secret.is_empty()
    }
}

/// 공유 Client를 받는 비동기 번역 함수
pub async fn translate_async_with_client(
    client: &reqwest::Client,
    text: &str,
    source: Language,
    target: Language,
    client_id: &str,
    client_secret: &str,
) -> TranslationResult {
    validate_not_empty(text)?;

    if client_id.is_empty() || client_secret.is_empty() {
        return Err(TranslationError::MissingApiKey);
    }

    let source_code = lang_utils::to_papago_code(source);
    let target_code = lang_utils::to_papago_code(target);

    let params = [
        ("source", source_code),
        ("target", target_code),
        ("text", text),
    ];

    let response = client
        .post(ENDPOINT)
        .header("X-Naver-Client-Id", client_id)
        .header("X-Naver-Client-Secret", client_secret)
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
        });
    }

    Err(TranslationError::Parse(
        "Papago 응답에서 번역 결과를 찾을 수 없습니다.".to_string(),
    ))
}
