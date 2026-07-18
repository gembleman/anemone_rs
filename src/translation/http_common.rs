//! HTTP 번역 엔진 공통 헬퍼
//!
//! Google/DeepL 등 HTTP 기반 번역 엔진의 공통 패턴을 추출.

use super::TranslationError;
use std::sync::OnceLock;
use std::time::Duration;

/// 프로세스 전역 reqwest::Client (connection pool 재사용)
static SHARED_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// 전체 요청 타임아웃.
///
/// LLM 스트리밍이 아닌 단발 응답을 가정. 너무 길게 잡으면 shutdown 시 워커
/// 스레드가 in-flight 요청 때문에 join 되지 못하고 detach 된다 (worker.rs 의
/// `shutdown` 참고). 일반 번역 엔진은 수 초 안에 응답하므로 보수적으로
/// 30 초로 제한해 비정상 행 걸림을 방지한다.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// 커넥션 수립 타임아웃 (DNS + TCP + TLS 핸드셰이크 상한)
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// LLM 백엔드용 per-request 타임아웃.
///
/// 일반 번역 엔진은 30 초로 충분하지만 LLM (특히 GPT-4 클래스 / Claude
/// Opus / Gemini Pro) 은 긴 입력이나 reasoning 으로 1 분 이상 걸리는
/// 경우가 흔하다. `shared_client` 의 기본 30 초를 그대로 적용하면 정상
/// 응답도 timeout 으로 끊어진다. LLM 호출은 `RequestBuilder::timeout()` 으로
/// 이 값을 override 하여 사용한다.
///
/// shutdown 지연 상한은 worker.rs 의 `shutdown` MAX_WAT (2s) 가 아니라
/// in-flight HTTP 요청이 detached 되는 시점이므로, 사용자가 종료 후
/// 프로세스가 백그라운드에 남는 시간은 최대 이 값. 너무 키우면 종료가
/// 답답해진다.
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

/// HTTP 응답을 읽고 상태 코드를 검증한다.
///
/// 성공이면 body 문자열을 반환, 실패면 `TranslationError::Api`를 반환.
pub async fn send_and_read_body(response: reqwest::Response) -> Result<String, TranslationError> {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after);
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
        });
    }

    Ok(body)
}

fn parse_retry_after(value: &str) -> Option<Duration> {
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
mod tests {
    use super::*;

    #[test]
    fn retry_after_seconds_is_bounded() {
        assert_eq!(parse_retry_after("7"), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after("9999"), Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after("invalid"), None);
    }
}
