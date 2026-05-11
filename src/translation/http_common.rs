//! HTTP 번역 엔진 공통 헬퍼
//!
//! Google/DeepL 등 HTTP 기반 번역 엔진의 공통 패턴을 추출.

use std::sync::OnceLock;
use std::time::Duration;
use super::TranslationError;

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
pub async fn send_and_read_body(
    response: reqwest::Response,
) -> Result<String, TranslationError> {
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

    Ok(body)
}

/// 빈 텍스트 검증
pub fn validate_not_empty(text: &str) -> Result<(), TranslationError> {
    if text.is_empty() {
        return Err(TranslationError::EmptyText);
    }
    Ok(())
}
