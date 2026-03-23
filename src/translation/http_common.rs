//! HTTP 번역 엔진 공통 헬퍼
//!
//! Google/DeepL 등 HTTP 기반 번역 엔진의 공통 패턴을 추출.

use super::{TranslationError, TranslationResult};

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

/// 동기 번역용 tokio Runtime 생성 후 async 함수 실행
pub fn block_on_async<F, Fut>(f: F) -> TranslationResult
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = TranslationResult>,
{
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TranslationError::Engine(format!("런타임 생성 실패: {}", e)))?;
    rt.block_on(f())
}
