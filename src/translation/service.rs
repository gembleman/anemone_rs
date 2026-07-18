use std::sync::Arc;

use super::worker::{TranslationDispatch, TranslationRequest};
use super::{PreparedJob, TranslationResult};

/// runtime host와 무관하게 prepared job을 실행하는 번역 서비스.
#[derive(Clone)]
pub struct TranslationService {
    http_client: reqwest::Client,
}

impl Default for TranslationService {
    fn default() -> Self {
        Self::new()
    }
}

impl TranslationService {
    pub fn new() -> Self {
        Self {
            http_client: super::http_common::create_client(),
        }
    }

    pub async fn translate(&self, job: PreparedJob, text: Arc<str>) -> TranslationResult {
        let request = TranslationRequest { id: 0, text, job };
        TranslationDispatch::translate_async(&request, &self.http_client).await
    }

    pub(crate) fn http_client(&self) -> reqwest::Client {
        self.http_client.clone()
    }
}
