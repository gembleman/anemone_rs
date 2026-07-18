//! 단일 worker thread에서 비동기 번역을 실행하는 platform 독립 dispatcher.
//!
//! 대상별 최신 요청만 유지하고 서로 다른 대상은 병렬 실행한다. 완료 통지는 외부
//! adapter에 위임하므로 UI handle이나 native message는 다루지 않는다.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::constants::MAX_RESPONSE_STORAGE;
use thiserror::Error;
use tokio::sync::{Semaphore, watch};

use super::llm::{LlmCallParams, LlmProvider};
use super::{Language, TranslationEngine, TranslationError, TranslationResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TranslationRequestError {
    #[error("번역 워커가 종료되어 요청을 받을 수 없습니다.")]
    WorkerUnavailable,
}

/// DeepL 다중 키 폴백 전략
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeepLStrategy {
    /// 첫 키부터 순서대로 사용, 한도 초과(429/456) 시 다음 키로 폴백
    #[default]
    Failover,
    /// 호출마다 키를 순회 (전역 카운터; 프로세스 수명 동안 유지)
    RoundRobin,
}

/// 엔진과 자격 증명의 잘못된 조합을 막는 요청별 인증 정보.
#[derive(Clone, Default)]
pub enum EngineCredentials {
    /// 자격증명 불필요 (EzTrans, Google 비공식)
    #[default]
    None,
    /// DeepL API 키. `keys`는 최소 1개; `strategy`에 따라 폴백/순회.
    DeepL {
        keys: Vec<String>,
        strategy: DeepLStrategy,
    },
    /// Ncloud Papago Application 인증 키 쌍
    Papago {
        client_id: String,
        client_secret: String,
    },
    /// LLM 호출 파라미터 일체 (제공자/모델/키/프롬프트/샘플링)
    Llm(LlmCallParams),
}

impl std::fmt::Debug for EngineCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => formatter.write_str("None"),
            Self::DeepL { keys, strategy } => formatter
                .debug_struct("DeepL")
                .field("key_count", &keys.len())
                .field("strategy", strategy)
                .finish(),
            Self::Papago { .. } => formatter.write_str("Papago(<redacted>)"),
            Self::Llm(parameters) => formatter
                .debug_struct("Llm")
                .field("provider", &parameters.provider)
                .field("model", &parameters.effective_model())
                .finish(),
        }
    }
}

/// 큐와 blocking task가 원문을 복사 없이 공유하는 번역 요청.
#[derive(Debug, Clone)]
pub struct TranslationRequest {
    /// 요청 ID (응답과 매칭용; 디스패치가 할당)
    pub id: u64,
    /// 번역할 텍스트
    pub text: Arc<str>,
    /// 번역 엔진
    pub engine: TranslationEngine,
    /// 소스 언어
    pub source_lang: Language,
    /// 타겟 언어
    pub target_lang: Language,
    /// 엔진별 자격증명
    pub credentials: EngineCredentials,
}

/// 번역 응답
#[derive(Debug, Clone)]
pub struct TranslationResponse {
    /// 번역 결과
    pub result: TranslationResult,
}

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

/// 큐에 실린 작업 (요청 + 라우팅 대상)
struct DispatchJob {
    target: TargetId,
    req: TranslationRequest,
    route: Arc<RouteSlot>,
    cancellation: watch::Receiver<u64>,
}

/// 대상 ID를 재등록해도 이전 작업이 새 소비자에게 전달되지 않게 하는 등록 세대.
struct RouteSlot {
    latest_id: Arc<AtomicU64>,
    cancellation: watch::Sender<u64>,
}

impl RouteSlot {
    fn new() -> Self {
        let (cancellation, _) = watch::channel(0);
        Self {
            latest_id: Arc::new(AtomicU64::new(0)),
            cancellation,
        }
    }

    fn set_latest(&self, id: u64) -> watch::Receiver<u64> {
        self.latest_id.store(id, Ordering::Release);
        self.cancellation.send_replace(id);
        self.cancellation.subscribe()
    }

    fn cancel(&self) {
        self.latest_id.store(0, Ordering::Release);
        self.cancellation.send_replace(0);
    }
}

/// 요청 ID와 아직 소비하지 않은 응답을 보관하는 내부 상태.
struct DispatchState {
    next_id: u64,
    /// 아직 소비하지 않은 FIFO 응답.
    pending: VecDeque<PendingEntry>,
}

struct PendingEntry {
    req_id: u64,
    target: TargetId,
    response: TranslationResponse,
}

impl DispatchState {
    fn new() -> Self {
        Self {
            next_id: 1,
            pending: VecDeque::new(),
        }
    }

    fn assign_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn push_response(&mut self, entry: PendingEntry) {
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

    fn take_response(&mut self, req_id: u64) -> Option<TranslationResponse> {
        let idx = self.pending.iter().position(|e| e.req_id == req_id)?;
        self.pending.remove(idx).map(|e| e.response)
    }

    fn drop_response(&mut self, req_id: u64) {
        self.pending.retain(|entry| entry.req_id != req_id);
    }

    fn drop_target(&mut self, target: TargetId) {
        self.pending.retain(|entry| entry.target != target);
    }
}

/// 디스패치와 워커가 공유하는 상태.
///
struct DispatchShared {
    state: Mutex<DispatchState>,
    /// 대상별 등록 세대. Lock은 응답 저장과 unregister 사이의 장벽이기도 하다.
    routes: Mutex<HashMap<usize, Arc<RouteSlot>>>,
    notifier: Arc<dyn CompletionNotifier>,
}

impl DispatchShared {
    fn new(notifier: Arc<dyn CompletionNotifier>) -> Self {
        Self {
            state: Mutex::new(DispatchState::new()),
            routes: Mutex::new(HashMap::new()),
            notifier,
        }
    }

    fn route(&self, target: TargetId) -> Arc<RouteSlot> {
        let mut routes = self.routes.lock().expect("routes poisoned");
        routes
            .entry(target.get())
            .or_insert_with(|| Arc::new(RouteSlot::new()))
            .clone()
    }

    #[cfg(test)]
    fn latest_atomic_lookup(&self, target: TargetId) -> Option<Arc<AtomicU64>> {
        let routes = self.routes.lock().expect("routes poisoned");
        routes
            .get(&target.get())
            .map(|route| route.latest_id.clone())
    }

    fn unregister(&self, target: TargetId) {
        let mut routes = self.routes.lock().expect("routes poisoned");
        if let Some(route) = routes.remove(&target.get()) {
            route.cancel();
        }
        let mut state = self.state.lock().expect("dispatch state poisoned");
        state.drop_target(target);
    }
}

/// 프로세스 단일 디스패치
pub struct TranslationDispatch {
    /// Shutdown 시 `take()`하여 channel을 닫는다.
    sender: Mutex<Option<Sender<DispatchJob>>>,
    shared: Arc<DispatchShared>,
    /// 워커 스레드 핸들 (shutdown 시 join 용)
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl TranslationDispatch {
    pub(crate) fn spawn(notifier: Arc<dyn CompletionNotifier>) -> Self {
        let (tx, rx) = mpsc::channel::<DispatchJob>();
        let shared = Arc::new(DispatchShared::new(notifier));
        let worker_shared = shared.clone();

        // Shutdown에서 join할 수 있도록 보관한다.
        let handle = thread::spawn(move || {
            Self::worker_thread(rx, worker_shared);
        });

        Self {
            sender: Mutex::new(Some(tx)),
            shared,
            worker: Mutex::new(Some(handle)),
        }
    }

    /// Channel을 닫고 실행 중인 task를 취소한 뒤 worker thread를 join한다.
    pub(crate) fn shutdown(&self) {
        // Sender drop이 명령 loop의 task 정리를 시작한다.
        {
            let mut s = self.sender.lock().expect("dispatch sender poisoned");
            *s = None;
        }
        // 제한 시간 안에 join하지 못하면 shutdown이 멈추지 않도록 detach한다.
        let handle = {
            let mut w = self.worker.lock().expect("dispatch worker poisoned");
            w.take()
        };
        if let Some(handle) = handle {
            let start = std::time::Instant::now();
            const MAX_WAIT: Duration = Duration::from_millis(2000);
            while !handle.is_finished() && start.elapsed() < MAX_WAIT {
                std::thread::sleep(Duration::from_millis(10));
            }
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                tracing::warn!(
                    "translation worker did not finish in {MAX_WAIT:?}, leaving detached"
                );
            }
        }
    }

    /// 번역 요청을 보내고 응답 매칭용 ID를 즉시 반환한다.
    pub(crate) fn request(
        &self,
        target: TargetId,
        mut req: TranslationRequest,
    ) -> Result<u64, TranslationRequestError> {
        let id = {
            let mut st = self.shared.state.lock().expect("dispatch state poisoned");
            st.assign_id()
        };
        req.id = id;

        let route = self.shared.route(target);
        let cancellation = route.set_latest(id);

        // Shutdown 중인 요청은 조용히 버린다.
        let send_result = {
            let s = self.sender.lock().expect("dispatch sender poisoned");
            match s.as_ref() {
                Some(tx) => tx.send(DispatchJob {
                    target,
                    req,
                    route: route.clone(),
                    cancellation,
                }),
                None => {
                    if route
                        .latest_id
                        .compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        route.cancellation.send_replace(0);
                    }
                    return Err(TranslationRequestError::WorkerUnavailable);
                }
            }
        };
        if let Err(error) = send_result {
            if route
                .latest_id
                .compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                route.cancellation.send_replace(0);
            }
            tracing::error!("Failed to send translation request: {error}");
            return Err(TranslationRequestError::WorkerUnavailable);
        }

        Ok(id)
    }

    /// 호출자가 자신의 응답을 꺼낸다.
    pub(crate) fn take_response(&self, req_id: u64) -> Option<TranslationResponse> {
        let mut st = self.shared.state.lock().expect("dispatch state poisoned");
        st.take_response(req_id)
    }

    /// 소비자가 사라질 때 라우팅과 대기 응답을 함께 정리한다.
    pub(crate) fn unregister(&self, target: TargetId) {
        self.shared.unregister(target);
    }

    pub(crate) fn cancel(&self, target: TargetId) {
        self.shared.unregister(target);
    }

    /// 워커 스레드 진입점
    fn worker_thread(rx: Receiver<DispatchJob>, shared: Arc<DispatchShared>) {
        // 명령 수신은 직렬화하고 실제 번역은 제한된 동시성으로 실행한다.
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!("Failed to create tokio runtime: {}", e);
                return;
            }
        };

        let client = super::http_common::shared_client();
        let http_limit = Arc::new(Semaphore::new(4));

        while let Ok(job) = rx.recv() {
            if job.route.latest_id.load(Ordering::Acquire) != job.req.id {
                tracing::debug!("요청 #{} 폐기 (더 최신 요청 존재)", job.req.id);
                continue;
            }

            let task_shared = shared.clone();
            let task_client = client.clone();
            let task_limit = http_limit.clone();
            rt.spawn(async move {
                Self::run_job(job, task_shared, task_client, task_limit).await;
            });
        }

        // 종료 시 실행 중인 작업을 제한 시간만 기다린다.
        rt.shutdown_timeout(Duration::from_secs(2));
    }

    async fn run_job(
        job: DispatchJob,
        shared: Arc<DispatchShared>,
        client: reqwest::Client,
        http_limit: Arc<Semaphore>,
    ) {
        let DispatchJob {
            target,
            req,
            route,
            mut cancellation,
        } = job;

        let translate = async {
            let _permit = if req.engine == TranslationEngine::EzTrans {
                None
            } else {
                match http_limit.acquire_owned().await {
                    Ok(permit) => Some(permit),
                    Err(_) => return Err(TranslationError::Engine("HTTP 스케줄러 종료".into())),
                }
            };
            Self::translate_async(&req, &client).await
        };

        let result = tokio::select! {
            result = translate => result,
            () = Self::wait_until_superseded(&mut cancellation, req.id) => {
                tracing::debug!("실행 중 요청 #{} 취소 (최신 요청/등록 해제)", req.id);
                return;
            }
        };

        Self::route_response(&shared, target, &route, req.id, result);
    }

    async fn wait_until_superseded(cancellation: &mut watch::Receiver<u64>, req_id: u64) {
        loop {
            if *cancellation.borrow() != req_id {
                return;
            }
            if cancellation.changed().await.is_err() {
                return;
            }
        }
    }

    fn route_response(
        shared: &DispatchShared,
        target: TargetId,
        route: &Arc<RouteSlot>,
        req_id: u64,
        result: TranslationResult,
    ) {
        // 같은 lock 안에서 세대 확인, 응답 저장, 통지를 끝내 TOCTOU를 막는다.
        let routes = shared.routes.lock().expect("routes poisoned");
        let Some(current) = routes.get(&target.get()) else {
            tracing::debug!("응답 #{} 폐기 (대상 등록 해제)", req_id);
            return;
        };
        if !Arc::ptr_eq(current, route) || route.latest_id.load(Ordering::Acquire) != req_id {
            tracing::debug!("응답 #{} 폐기 (stale 등록 세대/요청)", req_id);
            return;
        }

        {
            let mut state = shared.state.lock().expect("dispatch state poisoned");
            state.push_response(PendingEntry {
                req_id,
                target,
                response: TranslationResponse { result },
            });
        }

        if let Err(error) = shared.notifier.notify(target, req_id) {
            let mut state = shared.state.lock().expect("dispatch state poisoned");
            state.drop_response(req_id);
            tracing::warn!("번역 완료 통지 실패 (#{}): {}", req_id, error);
        }
        drop(routes);
    }

    /// 최대 재시도 횟수
    const MAX_RETRIES: u32 = 3;
    /// 초기 재시도 대기 시간 (밀리초)
    const INITIAL_BACKOFF_MS: u64 = 500;

    /// DeepL key를 전략에 따라 선택하고 한도/인증 오류면 다음 key로 넘어간다.
    async fn translate_deepl_multi_key(
        client: &reqwest::Client,
        text: &str,
        source: Language,
        target: Language,
        keys: &[String],
        strategy: DeepLStrategy,
    ) -> TranslationResult {
        if keys.is_empty() || keys.iter().all(|k| k.is_empty()) {
            return Err(TranslationError::MissingApiKey);
        }

        // 비어 있는 키는 건너뛰고 시작 오프셋만 결정한다.
        let start = match strategy {
            DeepLStrategy::Failover => 0,
            DeepLStrategy::RoundRobin => {
                use std::sync::atomic::AtomicUsize;
                static COUNTER: AtomicUsize = AtomicUsize::new(0);
                COUNTER.fetch_add(1, Ordering::Relaxed) % keys.len()
            }
        };

        let mut last_err: Option<TranslationError> = None;
        for offset in 0..keys.len() {
            let idx = (start + offset) % keys.len();
            let key = &keys[idx];
            if key.is_empty() {
                continue;
            }
            match super::deepl::translate_async_with_client(client, text, source, target, key).await
            {
                Ok(s) => return Ok(s),
                Err(e) => {
                    if Self::deepl_should_fallback(&e) && offset + 1 < keys.len() {
                        tracing::warn!(
                            key_index = idx,
                            category = e.log_category(),
                            status_code = ?e.log_status_code(),
                            "DeepL key failed; trying fallback"
                        );
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_err.unwrap_or(TranslationError::MissingApiKey))
    }

    /// 한도 초과/인증 실패는 다음 키로 폴백, 그 외(파싱/네트워크 등)는 즉시 반환
    fn deepl_should_fallback(err: &TranslationError) -> bool {
        match err {
            TranslationError::Api { code, .. } | TranslationError::RateLimited { code, .. } => {
                matches!(code, 429 | 456 | 403)
            }
            _ => false,
        }
    }

    /// 재시도를 포함한 비동기 번역. 별도 runtime에서도 직접 호출할 수 있다.
    pub(crate) async fn translate_async(
        req: &TranslationRequest,
        client: &reqwest::Client,
    ) -> TranslationResult {
        let mut last_err = None;

        let length = req.text.chars().count();
        let max = req.engine.max_input_chars();
        if length > max {
            return Err(TranslationError::InputTooLong {
                engine: req.engine.to_str(),
                length,
                max,
            });
        }

        for attempt in 0..=Self::MAX_RETRIES {
            if attempt > 0 {
                let server_delay = Self::retry_delay(last_err.as_ref(), attempt);
                let jitter = Duration::from_millis(
                    (req.id.wrapping_mul(37).wrapping_add(attempt as u64 * 101)) % 251,
                );
                let delay = server_delay.saturating_add(jitter);
                tracing::warn!(
                    attempt,
                    max_retries = Self::MAX_RETRIES,
                    delay_ms = delay.as_millis(),
                    "translation retry scheduled"
                );
                tokio::time::sleep(delay).await;
            }

            match Self::translate_once(req, client).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    if e.is_retryable() && attempt < Self::MAX_RETRIES {
                        tracing::warn!(
                            category = e.log_category(),
                            status_code = ?e.log_status_code(),
                            "retryable translation failure"
                        );
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| TranslationError::Engine("알 수 없는 오류".to_string())))
    }

    fn retry_delay(error: Option<&TranslationError>, attempt: u32) -> Duration {
        error
            .and_then(TranslationError::retry_after)
            .unwrap_or_else(|| {
                Duration::from_millis(Self::INITIAL_BACKOFF_MS * 2u64.pow(attempt - 1))
            })
    }

    /// 단일 번역 시도
    async fn translate_once(
        req: &TranslationRequest,
        client: &reqwest::Client,
    ) -> TranslationResult {
        match req.engine {
            TranslationEngine::EzTrans => {
                // Arc clone으로 본문 복사 없이 blocking task에 넘긴다.
                let text = Arc::clone(&req.text);
                let source = req.source_lang;
                let target = req.target_lang;

                match tokio::task::spawn_blocking(move || {
                    super::translate_with_eztrans(&text, source, target)
                })
                .await
                {
                    Ok(result) => result,
                    Err(e) => Err(TranslationError::Engine(format!(
                        "EzTrans 실행 오류: {}",
                        e
                    ))),
                }
            }
            TranslationEngine::Google => {
                super::google::translate_async_with_client(
                    client,
                    &req.text,
                    req.source_lang,
                    req.target_lang,
                )
                .await
            }
            TranslationEngine::DeepL => {
                let (keys, strategy) = match &req.credentials {
                    EngineCredentials::DeepL { keys, strategy } => (keys.as_slice(), *strategy),
                    _ => (&[][..], DeepLStrategy::Failover),
                };
                Self::translate_deepl_multi_key(
                    client,
                    &req.text,
                    req.source_lang,
                    req.target_lang,
                    keys,
                    strategy,
                )
                .await
            }
            TranslationEngine::Papago => {
                let (client_id, client_secret) = match &req.credentials {
                    EngineCredentials::Papago {
                        client_id,
                        client_secret,
                    } => (client_id.as_str(), client_secret.as_str()),
                    _ => ("", ""),
                };
                super::papago::translate_async_with_client(
                    client,
                    &req.text,
                    req.source_lang,
                    req.target_lang,
                    client_id,
                    client_secret,
                )
                .await
            }
            TranslationEngine::Llm => {
                let params = match &req.credentials {
                    EngineCredentials::Llm(p) => p,
                    _ => return Err(TranslationError::EngineNotInitialized("LLM")),
                };
                let result = match params.provider {
                    LlmProvider::OpenAi | LlmProvider::Grok | LlmProvider::OpenRouter => {
                        super::llm::openai_compat::translate_async_with_client(
                            client,
                            &req.text,
                            req.source_lang,
                            req.target_lang,
                            params,
                        )
                        .await
                    }
                    LlmProvider::Anthropic => {
                        super::llm::anthropic::translate_async_with_client(
                            client,
                            &req.text,
                            req.source_lang,
                            req.target_lang,
                            params,
                        )
                        .await
                    }
                    LlmProvider::Gemini => {
                        super::llm::gemini::translate_async_with_client(
                            client,
                            &req.text,
                            req.source_lang,
                            req.target_lang,
                            params,
                        )
                        .await
                    }
                };
                if let Ok(ref output) = result {
                    super::llm::usage::record(req.text.len(), output.len());
                }
                result
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/translation/worker.rs"]
mod tests;
