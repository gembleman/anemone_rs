//! 번역 디스패치 (프로세스 단일 워커)
//!
//! 별도 스레드에서 tokio 런타임을 실행하여 async 번역을 수행한다.
//! UI 스레드를 블로킹하지 않고 Windows 메시지로 결과를 전달한다.
//!
//! 구조:
//! - 프로세스 전역에 워커 스레드/tokio 런타임이 하나만 존재한다.
//! - 호출자(메인 윈도우, 번역 다이얼로그 등)는 `translate(hwnd, …)` 헬퍼
//!   (또는 `dispatch().request(hwnd, …)`) 로 자신의 hwnd 를 함께 전달한다.
//! - 워커는 응답 도착 시 hwnd 별 원자 슬롯(`latest_snapshot`)으로 stale 을 거른 뒤
//!   그 hwnd 에만 `WM_TRANSLATION_COMPLETE` 를 PostMessage 한다. WPARAM 은 `req_id` 다.
//! - 호출자는 메시지 수신 시 `take_response(req_id)` 로 본인 응답만 꺼낸다.
//! - 다이얼로그가 닫힐 때는 `unregister_hwnd(hwnd)` 로 라우팅/대기 응답을 청소한다.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use isolang::Language;
use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::constants::{MAX_RESPONSE_STORAGE, WM_TRANSLATION_COMPLETE};

use super::llm::{LlmCallParams, LlmProvider};
use super::{TranslationEngine, TranslationError, TranslationResult};

/// DeepL 다중 키 폴백 전략
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeepLStrategy {
    /// 첫 키부터 순서대로 사용, 한도 초과(429/456) 시 다음 키로 폴백
    #[default]
    Failover,
    /// 호출마다 키를 순회 (전역 카운터; 프로세스 수명 동안 유지)
    RoundRobin,
}

/// 엔진별 자격증명
///
/// 엔진이 늘어나도 `TranslationRequest`에 옵션 필드가 폭발하지 않도록 enum으로 묶는다.
/// 잘못된 조합(예: EzTrans에 API 키 전달)은 매칭 패턴 단계에서 명확히 드러난다.
#[derive(Debug, Clone, Default)]
pub enum EngineCredentials {
    /// 자격증명 불필요 (EzTrans, Google 비공식)
    #[default]
    None,
    /// DeepL API 키. `keys`는 최소 1개; `strategy`에 따라 폴백/순회.
    DeepL {
        keys: Vec<String>,
        strategy: DeepLStrategy,
    },
    /// Papago Naver 개발자센터 키 쌍
    Papago {
        client_id: String,
        client_secret: String,
    },
    /// LLM 호출 파라미터 일체 (제공자/모델/키/프롬프트/샘플링)
    Llm(LlmCallParams),
}

/// 번역 요청
///
/// `text` 는 `Arc<str>` — 큐를 가로지를 때 본문이 한 번도 복제되지 않는다.
/// EzTrans 의 `spawn_blocking` 같이 본문을 소유해야 하는 경로에서도 `clone()`
/// 은 refcount 증가만 일으켜 0-alloc.
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

/// 큐에 실린 작업 (요청 + 라우팅 대상 hwnd_raw)
struct DispatchJob {
    hwnd_raw: usize,
    req: TranslationRequest,
}

/// 내부 상태
///
/// stale 응답 폐기는 `latest_snapshot` 의 원자 슬롯이 담당하므로 여기서는
/// 단조 증가하는 ID 카운터와 아직 take 되지 않은 응답 큐만 들고 있다.
struct DispatchState {
    next_id: u64,
    /// FIFO 응답 큐. push_back / pop_front amortized O(1). 응답 폭주 시 앞쪽
    /// drain 비용도 일반 Vec 보다 가볍다.
    pending: VecDeque<PendingEntry>,
}

struct PendingEntry {
    req_id: u64,
    hwnd_raw: usize,
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

    fn drop_hwnd(&mut self, hwnd_raw: usize) {
        self.pending.retain(|e| e.hwnd_raw != hwnd_raw);
    }
}

/// 디스패치와 워커가 공유하는 상태.
///
/// `OnceLock::get_or_init` 초기화 중 워커 스레드가 다시 `dispatch()` 를 호출하면
/// 재진입 대기 위험이 있으므로, 워커에 필요한 상태는 `Arc` 로 직접 넘긴다.
struct DispatchShared {
    state: Mutex<DispatchState>,
    /// hwnd 별 최신 req_id 의 원자 스냅샷 (워커 스레드가 lock 없이 빠르게 확인).
    ///
    /// Mutex lock 구간은 슬롯 lookup/insert/remove 뿐이며, 실제 stale 판정은
    /// 슬롯에서 꺼낸 `Arc<AtomicU64>` 위에서 lock 없이 수행된다. 동시에 떠 있는
    /// hwnd 개수만큼만 자라는 맵이라 `HashMap` 이면 O(1).
    latest_snapshot: Mutex<HashMap<usize, std::sync::Arc<AtomicU64>>>,
}

impl DispatchShared {
    fn new() -> Self {
        Self {
            state: Mutex::new(DispatchState::new()),
            latest_snapshot: Mutex::new(HashMap::new()),
        }
    }

    /// hwnd 의 최신 ID 원자 슬롯 확보 (없으면 생성)
    fn latest_atomic(&self, hwnd_raw: usize) -> std::sync::Arc<AtomicU64> {
        let mut snap = self
            .latest_snapshot
            .lock()
            .expect("latest_snapshot poisoned");
        snap.entry(hwnd_raw)
            .or_insert_with(|| std::sync::Arc::new(AtomicU64::new(0)))
            .clone()
    }

    fn latest_atomic_lookup(&self, hwnd_raw: usize) -> Option<std::sync::Arc<AtomicU64>> {
        let snap = self
            .latest_snapshot
            .lock()
            .expect("latest_snapshot poisoned");
        snap.get(&hwnd_raw).cloned()
    }

    fn drop_latest_atomic(&self, hwnd_raw: usize) {
        let mut snap = self
            .latest_snapshot
            .lock()
            .expect("latest_snapshot poisoned");
        snap.remove(&hwnd_raw);
    }
}

/// 프로세스 단일 디스패치
pub struct TranslationDispatch {
    /// `Option` 인 이유: shutdown 시 `take()` 로 drop 시켜 채널을 닫고
    /// 워커 스레드의 `rx.recv()` 가 `Err` 를 반환해 자연 종료되도록 한다.
    sender: Mutex<Option<Sender<DispatchJob>>>,
    shared: Arc<DispatchShared>,
    /// 워커 스레드 핸들 (shutdown 시 join 용)
    worker: Mutex<Option<JoinHandle<()>>>,
}

static DISPATCH: OnceLock<TranslationDispatch> = OnceLock::new();

/// 전역 디스패치 가져오기 (첫 호출 시 워커 스레드 spawn)
pub fn dispatch() -> &'static TranslationDispatch {
    DISPATCH.get_or_init(TranslationDispatch::spawn)
}

/// 응답 꺼내기 (수신측 핸들러용 단축 함수)
pub fn take_response(req_id: u64) -> Option<TranslationResponse> {
    dispatch().take_response(req_id)
}

/// 호출자가 사라질 때 라우팅/응답 정리
pub fn unregister_hwnd(hwnd: HWND) {
    dispatch().unregister_hwnd(hwnd);
}

/// 프로세스 종료 직전 호출. 워커 스레드를 정상 종료시켜 detached 백그라운드
/// 스레드가 남지 않도록 한다.
///
/// 디스패치가 한 번도 사용되지 않은 경우 (`OnceLock` 미초기화) 에는 아무
/// 작업도 하지 않는다.
pub fn shutdown() {
    if let Some(d) = DISPATCH.get() {
        d.shutdown();
    }
}

impl TranslationDispatch {
    fn spawn() -> Self {
        let (tx, rx) = mpsc::channel::<DispatchJob>();
        let shared = Arc::new(DispatchShared::new());
        let worker_shared = shared.clone();

        // shutdown() 이 join 할 수 있도록 핸들 보관.
        let handle = thread::spawn(move || {
            Self::worker_thread(rx, worker_shared);
        });

        Self {
            sender: Mutex::new(Some(tx)),
            shared,
            worker: Mutex::new(Some(handle)),
        }
    }

    /// 채널 sender 를 drop 해 워커 스레드를 종료시키고 join.
    ///
    /// 워커는 `rt.block_on(async { while let Ok(job) = rx.recv() })` 로 블록되어
    /// 있어, 모든 sender 가 drop 되면 `Err` 를 받고 루프를 빠져나와 tokio 런타임
    /// (및 그 아래 워커 스레드 풀) 까지 자연 종료된다.
    ///
    /// join 은 짧은 timeout 이 없는 단순 join 이지만, 워커는 즉시 빠져나오므로
    /// 실무상 즉시 끝난다. 만에 하나 join 이 오래 걸리면 프로세스 종료를 막을
    /// 위험이 있으므로 호출자는 충분히 짧은 시간을 기대해야 한다.
    fn shutdown(&self) {
        // 1. sender drop → 워커의 rx.recv() 가 Err 를 반환해 루프 탈출 → tokio
        //    runtime drop → 워커 스레드 자연 종료.
        {
            let mut s = self.sender.lock().expect("dispatch sender poisoned");
            *s = None;
        }
        // 2. 워커 스레드 join (안전망: in-flight HTTP 요청 등으로 runtime drop 이
        //    오래 걸리면 짧은 polling 후 detach. 정상 케이스는 수 ms 안에 끝남).
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

    /// 번역 요청 송신. ID 를 즉시 반환하여 호출자가 응답 매칭에 사용할 수 있다.
    pub fn request(&self, hwnd: HWND, mut req: TranslationRequest) -> u64 {
        let hwnd_raw = hwnd.0 as usize;
        let id = {
            let mut st = self.shared.state.lock().expect("dispatch state poisoned");
            st.assign_id()
        };
        req.id = id;

        // 워커가 lock 없이 stale 판정할 수 있도록 원자 슬롯 갱신
        self.shared
            .latest_atomic(hwnd_raw)
            .store(id, Ordering::Release);

        // shutdown 이 진행됐다면 sender 가 None — 조용히 폐기.
        let send_result = {
            let s = self.sender.lock().expect("dispatch sender poisoned");
            match s.as_ref() {
                Some(tx) => tx.send(DispatchJob { hwnd_raw, req }),
                None => return id,
            }
        };
        if let Err(e) = send_result {
            tracing::error!("Failed to send translation request: {}", e);
        }

        id
    }

    /// 호출자(메인 윈도우 / 다이얼로그)가 자기 응답을 꺼낸다
    pub fn take_response(&self, req_id: u64) -> Option<TranslationResponse> {
        let mut st = self.shared.state.lock().expect("dispatch state poisoned");
        st.take_response(req_id)
    }

    /// 다이얼로그가 닫히는 등 hwnd 가 사라질 때 호출
    pub fn unregister_hwnd(&self, hwnd: HWND) {
        let hwnd_raw = hwnd.0 as usize;
        {
            let mut st = self.shared.state.lock().expect("dispatch state poisoned");
            st.drop_hwnd(hwnd_raw);
        }
        self.shared.drop_latest_atomic(hwnd_raw);
    }

    /// 워커 스레드 진입점
    fn worker_thread(rx: Receiver<DispatchJob>, shared: Arc<DispatchShared>) {
        // current_thread 런타임. 디스패치는 "recv → translate_async await → 다음 recv"
        // 의 엄격한 직렬 처리라 멀티스레드 풀이 불필요하다. spawn_blocking 은
        // current_thread 런타임에서도 별도 blocking pool 로 분리되어 EzTrans 경로가
        // 그대로 동작한다. multi_thread 대비 worker thread 풀 1세트 + 관련 코드 경로
        // 가 빠진다.
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!("Failed to create tokio runtime: {}", e);
                return;
            }
        };

        rt.block_on(async {
            let client = super::http_common::shared_client();

            while let Ok(job) = rx.recv() {
                let DispatchJob { hwnd_raw, req } = job;

                // 큐에서 꺼낸 시점에 같은 hwnd 의 더 최신 요청이 있으면 스킵.
                // (자동 클립보드 번역의 텍스트 폭주 대응)
                let latest = shared
                    .latest_atomic_lookup(hwnd_raw)
                    .map(|a| a.load(Ordering::Acquire))
                    .unwrap_or(0);
                if req.id < latest {
                    tracing::debug!("요청 #{} 폐기 (더 최신 요청 존재)", req.id);
                    continue;
                }

                let result = Self::translate_async(&req, &client).await;

                // 응답 도착 시점에도 stale 재확인. LLM 같은 느린 엔진 대응.
                let latest = shared
                    .latest_atomic_lookup(hwnd_raw)
                    .map(|a| a.load(Ordering::Acquire))
                    .unwrap_or(0);
                if req.id < latest {
                    tracing::debug!("응답 #{} 폐기 (stale)", req.id);
                    continue;
                }

                // hwnd 가 unregister 된 경우 (다이얼로그 폐기 등) 도 폐기
                if shared.latest_atomic_lookup(hwnd_raw).is_none() {
                    tracing::debug!("응답 #{} 폐기 (대상 hwnd 등록 해제)", req.id);
                    continue;
                }

                // 응답 저장 후 PostMessage
                {
                    let mut st = shared.state.lock().expect("dispatch state poisoned");
                    st.push_response(PendingEntry {
                        req_id: req.id,
                        hwnd_raw,
                        response: TranslationResponse { result },
                    });
                }

                // SAFETY: hwnd_raw 는 호출자가 등록 시 넘긴 원래의 HWND 비트 표현이며,
                // unregister_hwnd 가 호출되지 않은 한 윈도우는 살아 있다. 위에서
                // latest_atomic_lookup 으로 그 유효성을 확인했다. PostMessageW 는
                // 모든 스레드에서 호출 안전하다.
                unsafe {
                    let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
                    let _ = PostMessageW(
                        Some(hwnd),
                        WM_TRANSLATION_COMPLETE,
                        WPARAM(req.id as usize),
                        LPARAM(0),
                    );
                }
            }
        });
    }

    /// 최대 재시도 횟수
    const MAX_RETRIES: u32 = 3;
    /// 초기 재시도 대기 시간 (밀리초)
    const INITIAL_BACKOFF_MS: u64 = 500;

    /// DeepL 다중 키 호출 (failover / round-robin)
    ///
    /// 한도 초과(HTTP 429, 456) 또는 인증 실패(403) 발생 시 다음 키로 재시도한다.
    /// 모든 키가 실패하면 마지막 에러를 반환한다.
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
                        tracing::warn!("DeepL 키 #{} 실패(폴백): {}", idx, e);
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
            TranslationError::Api { code, .. } => matches!(code, 429 | 456 | 403),
            _ => false,
        }
    }

    /// 비동기 번역 수행 (재시도 포함)
    ///
    /// 디스패치 큐를 우회해 직접 한 건을 번역할 때도 쓸 수 있도록 `pub(crate)` 노출.
    /// 파일 번역처럼 라인 단위 동기 호출이 필요한 곳에서 자체 tokio 런타임 위에
    /// 이 함수를 `block_on` 하는 식으로 재사용한다.
    pub(crate) async fn translate_async(
        req: &TranslationRequest,
        client: &reqwest::Client,
    ) -> TranslationResult {
        let mut last_err = None;

        for attempt in 0..=Self::MAX_RETRIES {
            if attempt > 0 {
                let delay = Self::INITIAL_BACKOFF_MS * 2u64.pow(attempt - 1);
                tracing::warn!(
                    "번역 재시도 ({}/{}), {}ms 후...",
                    attempt,
                    Self::MAX_RETRIES,
                    delay
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }

            match Self::translate_once(req, client).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    if e.is_retryable() && attempt < Self::MAX_RETRIES {
                        tracing::warn!("재시도 가능한 에러: {}", e);
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| TranslationError::Engine("알 수 없는 오류".to_string())))
    }

    /// 단일 번역 시도
    async fn translate_once(
        req: &TranslationRequest,
        client: &reqwest::Client,
    ) -> TranslationResult {
        match req.engine {
            TranslationEngine::EzTrans => {
                // Arc<str> 의 clone 은 refcount 증가만 — 큰 본문도 0-alloc 으로 blocking
                // 태스크에 이동시킨다.
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

/// 호출자 측 간편 헬퍼: 가장 흔한 패턴(번역 요청 한 줄로 보내기)을 한 함수로.
///
/// `text` 는 호출자가 이미 가지고 있는 `String` 을 그대로 받아 내부에서
/// `Arc<str>` 로 한 번 옮긴다 (`Arc::<str>::from(String)` 은 buffer 재사용 —
/// 추가 alloc 없음). 이후 큐/워커/엔진 호출 경로는 모두 share-by-refcount.
pub fn translate(
    hwnd: HWND,
    text: String,
    engine: TranslationEngine,
    source_lang: Language,
    target_lang: Language,
    credentials: EngineCredentials,
) -> u64 {
    dispatch().request(
        hwnd,
        TranslationRequest {
            id: 0, // dispatch 에서 할당
            text: Arc::from(text),
            engine,
            source_lang,
            target_lang,
            credentials,
        },
    )
}
