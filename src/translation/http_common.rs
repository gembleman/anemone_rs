//! HTTP 번역 engine 공통 client, timeout, response 처리.

use super::TranslationError;
use std::sync::Once;
use std::time::Duration;

static INSTALL_RUSTLS_PROVIDER: Once = Once::new();

/// 일반 번역 요청이 worker 종료를 오래 막지 않게 하는 전체 timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// 커넥션 수립 타임아웃 (DNS + TCP + TLS 핸드셰이크 상한)
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// 긴 생성 응답을 허용하되 종료 지연을 제한하는 LLM 요청별 timeout.
pub const LLM_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// 정상 응답은 번역 결과로 충분한 크기만 허용하고, 오류 본문은 사용자 메시지와
/// 로그에 필요한 작은 범위로 더 엄격히 제한한다.
const SUCCESS_BODY_LIMIT: usize = 8 * 1024 * 1024;
const ERROR_BODY_LIMIT: usize = 64 * 1024;

pub fn create_client() -> reqwest::Client {
    INSTALL_RUSTLS_PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .unwrap_or_else(|error| {
            tracing::warn!("reqwest 클라이언트 빌드 실패, 기본값 사용: {error}");
            reqwest::Client::new()
        })
}

/// 성공 body를 반환하고 오류 status를 `TranslationError::Api`로 바꾼다.
pub async fn send_and_read_body(
    mut response: reqwest::Response,
) -> Result<String, TranslationError> {
    let status = response.status();
    let retry_after = parse_retry_after(response.headers());
    let limit = if status.is_success() {
        SUCCESS_BODY_LIMIT
    } else {
        ERROR_BODY_LIMIT
    };
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(TranslationError::ResponseTooLarge { limit });
    }

    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(limit as u64) as usize,
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| TranslationError::Network(e.to_string()))?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(TranslationError::ResponseTooLarge { limit });
        }
        bytes.extend_from_slice(&chunk);
    }
    let body = String::from_utf8(bytes)
        .map_err(|error| TranslationError::Parse(format!("응답이 UTF-8이 아닙니다: {error}")))?;

    if !status.is_success() {
        if status.as_u16() == 429 {
            return Err(TranslationError::RateLimited {
                code: status.as_u16(),
                message: body,
                retry_after,
            });
        }
        return Err(TranslationError::Api {
            code: status.as_u16(),
            message: body,
            retry_after,
        });
    }

    Ok(body)
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after_value)
}

fn parse_retry_after_value(value: &str) -> Option<Duration> {
    const MAX_RETRY_AFTER: Duration = Duration::from_secs(120);
    let duration = if let Ok(seconds) = value.trim().parse::<u64>() {
        Duration::from_secs(seconds)
    } else {
        httpdate::parse_http_date(value)
            .ok()?
            .duration_since(std::time::SystemTime::now())
            .unwrap_or_default()
    };
    Some(duration.min(MAX_RETRY_AFTER))
}

/// 빈 텍스트 검증
pub fn validate_not_empty(text: &str) -> Result<(), TranslationError> {
    if text.is_empty() {
        return Err(TranslationError::EmptyText);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/translation/http_common.rs"]
mod tests;
