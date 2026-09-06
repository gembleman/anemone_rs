//! 디스패치 큐/응답 저장소와 대상별 라우팅 세대를 관리하는 공유 상태.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::watch;

pub(crate) const MAX_RESPONSE_STORAGE: usize = 100;

/// 번역 완료 이벤트가 돌아갈 불투명 대상 ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TargetId(usize);

impl TargetId {
    pub(crate) const fn new(raw: usize) -> Self {
        Self(raw)
    }

    pub(crate) const fn get(self) -> usize {
        self.0
    }
}

/// 완료 이벤트를 UI 또는 다른 소비자에게 전달하는 플랫폼 독립 경계.
pub(crate) trait CompletionNotifier: Send + Sync {
    fn notify(&self, target: TargetId, request_id: u64) -> Result<(), String>;
}

/// 대상 ID를 재등록해도 이전 작업이 새 소비자에게 전달되지 않게 하는 등록 세대.
pub(super) struct RouteSlot {
    pub(super) latest_id: Arc<AtomicU64>,
    pub(super) cancellation: watch::Sender<u64>,
}

impl RouteSlot {
    pub(super) fn new() -> Self {
        let (cancellation, _) = watch::channel(0);
        Self {
            latest_id: Arc::new(AtomicU64::new(0)),
            cancellation,
        }
    }

    pub(super) fn set_latest(&self, id: u64) -> watch::Receiver<u64> {
        self.latest_id.store(id, Ordering::Release);
        self.cancellation.send_replace(id);
        self.cancellation.subscribe()
    }

    pub(super) fn cancel(&self) {
        self.latest_id.store(0, Ordering::Release);
        self.cancellation.send_replace(0);
    }
}

/// 요청 ID와 아직 소비하지 않은 응답을 보관하는 내부 상태.
pub(super) struct DispatchState {
    next_id: u64,
    /// 아직 소비하지 않은 FIFO 응답.
    pub(super) pending: VecDeque<PendingEntry>,
}

pub(super) struct PendingEntry {
    pub(super) req_id: u64,
    pub(super) target: TargetId,
    pub(super) response: super::TranslationResponse,
}

impl DispatchState {
    fn new() -> Self {
        Self {
            next_id: 1,
            pending: VecDeque::new(),
        }
    }

    pub(super) fn assign_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub(super) fn push_response(&mut self, entry: PendingEntry) {
        self.pending.push_back(entry);
        if self.pending.len() > MAX_RESPONSE_STORAGE {
            let excess = self.pending.len() - MAX_RESPONSE_STORAGE;
            tracing::warn!(
                "응답 저장소가 최대 크기({})를 초과하여 {}개의 오래된 항목을 제거합니다",
                MAX_RESPONSE_STORAGE,
                excess
            );
            self.pending.drain(..excess);
        }
    }

    #[cfg(test)]
    pub(super) fn take_response(&mut self, req_id: u64) -> Option<super::TranslationResponse> {
        let idx = self.pending.iter().position(|e| e.req_id == req_id)?;
        self.pending.remove(idx).map(|e| e.response)
    }

    pub(super) fn take_response_for_target(
        &mut self,
        target: TargetId,
    ) -> Option<(u64, super::TranslationResponse)> {
        let idx = self
            .pending
            .iter()
            .position(|entry| entry.target == target)?;
        self.pending
            .remove(idx)
            .map(|entry| (entry.req_id, entry.response))
    }

    pub(super) fn drop_response(&mut self, req_id: u64) {
        self.pending.retain(|entry| entry.req_id != req_id);
    }

    pub(super) fn drop_target(&mut self, target: TargetId) {
        self.pending.retain(|entry| entry.target != target);
    }
}

/// 디스패치와 워커가 공유하는 상태.
pub(super) struct DispatchShared {
    /// poison 복구 안전: 잠금을 쥔 채로는 `next_id` 증가와 `VecDeque`
    /// push_back/drain/retain 같은 표준 연산만 수행하고, 사용자 콜백(notifier
    /// 등)은 이 lock 밖에서 호출하므로 panic이 중간에 불변식을 깨뜨릴 수 없다.
    pub(super) state: Mutex<DispatchState>,
    /// 대상별 등록 세대. Lock은 응답 저장과 unregister 사이의 장벽이기도 하다.
    /// poison 복구 안전: `HashMap`의 insert/remove/get만 수행하는 단순 연산이라
    /// 중간 상태로 poison될 수 없다.
    pub(super) routes: Mutex<HashMap<usize, Arc<RouteSlot>>>,
    pub(super) notifier: Arc<dyn CompletionNotifier>,
}

impl DispatchShared {
    pub(super) fn new(notifier: Arc<dyn CompletionNotifier>) -> Self {
        Self {
            state: Mutex::new(DispatchState::new()),
            routes: Mutex::new(HashMap::new()),
            notifier,
        }
    }

    pub(super) fn route(&self, target: TargetId) -> Arc<RouteSlot> {
        let mut routes = super::lock_or_recover(&self.routes);
        routes
            .entry(target.get())
            .or_insert_with(|| Arc::new(RouteSlot::new()))
            .clone()
    }

    #[cfg(test)]
    pub(super) fn latest_atomic_lookup(&self, target: TargetId) -> Option<Arc<AtomicU64>> {
        let routes = super::lock_or_recover(&self.routes);
        routes
            .get(&target.get())
            .map(|route| route.latest_id.clone())
    }

    pub(super) fn unregister(&self, target: TargetId) {
        let mut routes = super::lock_or_recover(&self.routes);
        if let Some(route) = routes.remove(&target.get()) {
            route.cancel();
        }
        let mut state = super::lock_or_recover(&self.state);
        state.drop_target(target);
    }
}
