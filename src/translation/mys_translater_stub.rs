use std::sync::atomic::{AtomicU64, Ordering};

use super::Language;
use super::{TranslationError, TranslationResult};

const UNAVAILABLE: &str =
    "이 빌드에는 MyS Translater 엔진이 들어 있지 않습니다. 다른 번역 엔진을 선택해 주세요.";

static NEXT_CALL_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
#[allow(dead_code)]
pub struct MysTranslaterCallParams {
    pub base_url: String,
    pub api_token: String,
}

impl MysTranslaterCallParams {
    pub fn validate(&self) -> Result<(), String> {
        Err(UNAVAILABLE.to_string())
    }
}

/// 엔진이 없는 빌드에는 기본 서버 주소가 없다.
pub fn default_base_url() -> &'static str {
    ""
}

pub(crate) fn next_request_id() -> u64 {
    NEXT_CALL_ID.fetch_add(1, Ordering::Relaxed)
}

pub(crate) async fn translate_async_with_client_for_request(
    _client: &reqwest::Client,
    _text: &str,
    _source: Language,
    _target: Language,
    _params: &MysTranslaterCallParams,
    _request_id: u64,
) -> TranslationResult {
    Err(TranslationError::Engine(UNAVAILABLE.to_string()))
}
