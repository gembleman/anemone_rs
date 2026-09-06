//! 단일 worker thread에서 비동기 번역을 실행하는 platform 독립 dispatcher.
//!
//! 대상별 최신 요청만 유지하고 서로 다른 대상은 병렬 실행한다. 완료 통지는 외부
//! adapter에 위임하므로 UI handle이나 native message는 다루지 않는다.

mod dispatch;
mod state;
mod translate;

use std::sync::{Mutex, MutexGuard};

use std::sync::Arc;

use thiserror::Error;

use super::PreparedJob;
pub(crate) use state::{CompletionNotifier, TargetId};

pub(crate) use dispatch::TranslationDispatch;

/// Poison된 뮤텍스도 이전 상태 그대로 복구해 계속 진행한다.
///
/// `worker` 모듈이 다루는 뮤텍스(요청 큐 상태, 라우팅 테이블, sender/worker
/// 핸들)는 모두 잠금을 쥔 채 `Option`/`HashMap`/`VecDeque` 같은 표준 컬렉션
/// 연산만 수행하고 사용자 콜백이나 I/O를 호출하지 않는다. 따라서 그 안에서
/// panic이 나더라도 자료구조가 불변식이 깨진 중간 상태로 남지 않으므로,
/// poison을 그대로 무시하고 내부 값을 재사용해도 안전하다.
pub(super) fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TranslationRequestError {
    #[error("번역 워커가 종료되어 요청을 받을 수 없습니다.")]
    WorkerUnavailable,
}

/// 큐와 blocking task가 원문을 복사 없이 공유하는 번역 요청.
#[derive(Debug, Clone)]
pub struct TranslationRequest {
    /// 요청 ID (응답과 매칭용; 디스패치가 할당)
    pub id: u64,
    /// 번역할 텍스트
    pub text: Arc<str>,
    /// 검증된 엔진, 자격증명과 언어쌍.
    pub job: PreparedJob,
}

/// 번역 응답
#[derive(Debug, Clone)]
pub struct TranslationResponse {
    /// 번역 결과
    pub result: super::TranslationResult,
}

#[cfg(test)]
#[path = "../../../tests/unit/translation/worker/dispatch.rs"]
mod dispatch_tests;

#[cfg(test)]
#[path = "../../../tests/unit/translation/worker/translate.rs"]
mod translate_tests;
