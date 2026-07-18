//! 사용자 정의 JSON REST 번역 API 어댑터.

use serde_json::Value;

use super::http_common::{send_and_read_body, validate_not_empty};
use super::{Language, TranslationError, TranslationResult, lang_utils};

/// 비밀값을 포함할 수 있으므로 의도적으로 `Debug`를 구현하지 않는다.
#[derive(Clone)]
pub struct CustomApiCallParams {
    pub url: String,
    pub api_key: String,
    pub auth_header: String,
    pub auth_scheme: String,
    pub headers: String,
    pub request_template: String,
    pub response_path: String,
}

impl CustomApiCallParams {
    pub fn validate(&self) -> Result<(), String> {
        let url = reqwest::Url::parse(self.url.trim())
            .map_err(|error| format!("커스텀 API URL이 잘못되었습니다: {error}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("커스텀 API URL은 http 또는 https여야 합니다.".to_string());
        }
        serde_json::from_str::<Value>(&self.request_template)
            .map_err(|error| format!("커스텀 API 요청 템플릿이 올바른 JSON이 아닙니다: {error}"))?;
        if !self.api_key.is_empty() && !self.auth_header.trim().is_empty() {
            reqwest::header::HeaderName::from_bytes(self.auth_header.trim().as_bytes())
                .map_err(|error| format!("커스텀 API 인증 헤더 이름이 잘못되었습니다: {error}"))?;
            let value = auth_value(self);
            reqwest::header::HeaderValue::from_str(&value)
                .map_err(|error| format!("커스텀 API 인증 헤더 값이 잘못되었습니다: {error}"))?;
        }
        build_extra_headers(&self.headers, &[("{api_key}", self.api_key.as_str())])?;
        Ok(())
    }
}

pub async fn translate_async_with_client(
    client: &reqwest::Client,
    text: &str,
    source: Language,
    target: Language,
    params: &CustomApiCallParams,
) -> TranslationResult {
    validate_not_empty(text)?;
    params.validate().map_err(TranslationError::Engine)?;

    let mut body: Value = serde_json::from_str(&params.request_template)
        .map_err(|error| TranslationError::Parse(error.to_string()))?;
    let replacements = [
        ("{text}", text),
        ("{source}", lang_utils::to_code(source)),
        ("{target}", lang_utils::to_code(target)),
        ("{api_key}", params.api_key.as_str()),
    ];
    substitute_json_strings(&mut body, &replacements);

    let mut request = client.post(params.url.trim()).json(&body);
    if !params.api_key.is_empty() && !params.auth_header.trim().is_empty() {
        request = request.header(params.auth_header.trim(), auth_value(params));
    }
    let headers =
        build_extra_headers(&params.headers, &replacements).map_err(TranslationError::Engine)?;
    request = request.headers(headers);
    let response = request
        .send()
        .await
        .map_err(|error| TranslationError::Network(error.to_string()))?;
    let response_body = send_and_read_body(response).await?;
    extract_translation(&response_body, &params.response_path)
}

fn auth_value(params: &CustomApiCallParams) -> String {
    if params.auth_scheme.trim().is_empty() {
        params.api_key.clone()
    } else {
        format!("{} {}", params.auth_scheme.trim(), params.api_key)
    }
}

fn build_extra_headers(
    configured: &str,
    replacements: &[(&str, &str)],
) -> Result<reqwest::header::HeaderMap, String> {
    let values: serde_json::Map<String, Value> = serde_json::from_str(configured)
        .map_err(|error| format!("커스텀 API 추가 헤더가 JSON 객체가 아닙니다: {error}"))?;
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in values {
        let mut value = value
            .as_str()
            .ok_or_else(|| format!("커스텀 API 헤더 값은 문자열이어야 합니다: {name}"))?
            .to_string();
        for (placeholder, replacement) in replacements {
            value = value.replace(placeholder, replacement);
        }
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| format!("커스텀 API 헤더 이름이 잘못되었습니다: {error}"))?;
        let value = reqwest::header::HeaderValue::from_str(&value)
            .map_err(|error| format!("커스텀 API 헤더 값이 잘못되었습니다: {error}"))?;
        headers.insert(name, value);
    }
    Ok(headers)
}

fn substitute_json_strings(value: &mut Value, replacements: &[(&str, &str)]) {
    match value {
        Value::String(text) => {
            for (placeholder, replacement) in replacements {
                *text = text.replace(placeholder, replacement);
            }
        }
        Value::Array(values) => {
            for value in values {
                substitute_json_strings(value, replacements);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                substitute_json_strings(value, replacements);
            }
        }
        _ => {}
    }
}

fn extract_translation(body: &str, configured_path: &str) -> TranslationResult {
    let path = configured_path.trim();
    if path.is_empty() {
        return Ok(body.to_string());
    }

    let value: Value =
        serde_json::from_str(body).map_err(|error| TranslationError::Parse(error.to_string()))?;
    let selected = if path.starts_with('/') {
        value.pointer(path)
    } else {
        let mut current = Some(&value);
        for segment in path.split('.').filter(|segment| !segment.is_empty()) {
            current = current.and_then(|value| match value {
                Value::Array(values) => segment
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| values.get(index)),
                Value::Object(values) => values.get(segment),
                _ => None,
            });
        }
        current
    };

    selected
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            TranslationError::Parse(format!(
                "커스텀 API 응답 경로에서 문자열 번역 결과를 찾을 수 없습니다: {path}"
            ))
        })
}

#[cfg(test)]
#[path = "../../tests/unit/translation/custom.rs"]
mod tests;
