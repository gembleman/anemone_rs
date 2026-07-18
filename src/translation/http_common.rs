//! HTTP 번역 engine 공통 client, timeout, response 처리.

use super::TranslationError;
use std::sync::OnceLock;
use std::time::Duration;

/// 프로세스 전역 reqwest::Client (connection pool 재사용)
static SHARED_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// 일반 번역 요청이 worker 종료를 오래 막지 않게 하는 전체 timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// 커넥션 수립 타임아웃 (DNS + TCP + TLS 핸드셰이크 상한)
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// 긴 생성 응답을 허용하되 종료 지연을 제한하는 LLM 요청별 timeout.
pub const LLM_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

pub fn shared_client() -> reqwest::Client {
    SHARED_CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .connect_timeout(CONNECT_TIMEOUT)
                .build()
                .unwrap_or_else(|e| {
                    tracing::warn!("reqwest 클라이언트 빌드 실패, 기본값 사용: {}", e);
                    reqwest::Client::new()
                })
        })
        .clone()
}

/// 성공 body를 반환하고 오류 status를 `TranslationError::Api`로 바꾼다.
pub async fn send_and_read_body(response: reqwest::Response) -> Result<String, TranslationError> {
    let status = response.status();
    let retry_after = parse_retry_after(response.headers());
    let body = response
        .text()
        .await
        .map_err(|e| TranslationError::Network(e.to_string()))?;

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
