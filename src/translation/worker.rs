//! 번역 워커 스레드
//!
//! 별도 스레드에서 tokio 런타임을 실행하여 async 번역을 수행합니다.
//! UI 스레드를 블로킹하지 않고 Windows 메시지로 결과를 전달합니다.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use isolang::Language;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::Win32::Foundation::{WPARAM, LPARAM};

use crate::constants::{MAX_RESPONSE_STORAGE, WM_TRANSLATION_COMPLETE};

use super::llm::{LlmCallParams, LlmProvider};
use super::{TranslationEngine, TranslationError, TranslationResult};

/// DeepL 다중 키 폴백 전략
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeepLStrategy {
    /// 첫 키부터 순서대로 사용, 한도 초과(429/456) 시 다음 키로 폴백
    #[default]
    Failover,
    /// 호출마다 키를 순회 (전역 카운터; 워커 인스턴스 수명 동안 유지)
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
#[derive(Debug, Clone)]
pub struct TranslationRequest {
    /// 요청 ID (응답과 매칭용)
    pub id: u64,
    /// 번역할 텍스트
    pub text: String,
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

/// 응답 저장소 (thread-safe). `OnceLock` 자체가 `'static` 참조를 돌려주므로
/// `Arc` 없이 `&'static Mutex<_>` 로 충분하다.
static RESPONSE_STORAGE: std::sync::OnceLock<Mutex<Vec<TranslationResponse>>> =
    std::sync::OnceLock::new();

fn get_response_storage() -> &'static Mutex<Vec<TranslationResponse>> {
    RESPONSE_STORAGE.get_or_init(|| Mutex::new(Vec::new()))
}

/// 응답 저장 (MAX_RESPONSE_STORAGE 초과 시 오래된 항목 제거)
pub fn store_response(response: TranslationResponse) {
    if let Ok(mut storage) = get_response_storage().lock() {
        storage.push(response);
        if storage.len() > MAX_RESPONSE_STORAGE {
            let excess = storage.len() - MAX_RESPONSE_STORAGE;
            tracing::warn!(
                "응답 저장소가 최대 크기({})를 초과하여 {}개의 오래된 항목을 제거합니다",
                MAX_RESPONSE_STORAGE,
                excess
            );
            storage.drain(..excess);
        }
    }
}

/// 모든 응답 가져오기
pub fn take_all_responses() -> Vec<TranslationResponse> {
    if let Ok(mut storage) = get_response_storage().lock() {
        std::mem::take(&mut *storage)
    } else {
        Vec::new()
    }
}

/// 번역 워커
pub struct TranslationWorker {
    sender: Sender<TranslationRequest>,
    /// 워커 스레드 핸들 (drop 시 자동 detach)
    _handle: JoinHandle<()>,
    next_id: u64,
    /// 마지막으로 큐에 넣은 요청 ID.
    /// 워커 스레드와 공유하여 처리 도중·완료 시점에 본인이 최신인지 확인할 때 사용.
    /// LLM처럼 응답이 늦게 도착하는 엔진에서 stale 결과를 폐기하기 위함.
    latest_id: Arc<AtomicU64>,
}

impl TranslationWorker {
    /// 새 워커 생성 및 시작
    pub fn spawn(hwnd: HWND) -> Self {
        let (tx, rx) = mpsc::channel::<TranslationRequest>();

        // HWND를 usize로 변환하여 Send 트레이트 문제 해결
        let hwnd_raw = hwnd.0 as usize;
        let latest_id = Arc::new(AtomicU64::new(0));
        let latest_for_worker = latest_id.clone();

        let handle = thread::spawn(move || {
            Self::worker_thread(rx, hwnd_raw, latest_for_worker);
        });

        Self {
            sender: tx,
            _handle: handle,
            next_id: 1,
            latest_id,
        }
    }

    /// 워커 스레드 메인 함수
    fn worker_thread(
        rx: Receiver<TranslationRequest>,
        hwnd_raw: usize,
        latest_id: Arc<AtomicU64>,
    ) {
        // usize를 HWND로 다시 변환
        let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
        // tokio 런타임 생성
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!("Failed to create tokio runtime: {}", e);
                return;
            }
        };

        rt.block_on(async {
            let client = super::http_common::shared_client();
            while let Ok(req) = rx.recv() {
                // 큐에서 꺼낸 시점에 이미 더 최신 요청이 있다면 스킵.
                // (자동 클립보드 번역에서 짧은 간격으로 들이닥치는 텍스트 폭주 대응)
                if req.id < latest_id.load(Ordering::Acquire) {
                    tracing::debug!("요청 #{} 폐기 (더 최신 요청 존재)", req.id);
                    continue;
                }

                let result = Self::translate_async(&req, &client).await;

                // 응답 도착 시점에도 본인이 최신인지 한 번 더 확인.
                // LLM처럼 수 초 걸리는 호출 중 사용자가 더 새 텍스트를 복사했을 수 있음.
                if req.id < latest_id.load(Ordering::Acquire) {
                    tracing::debug!("응답 #{} 폐기 (stale)", req.id);
                    continue;
                }

                let response = TranslationResponse { result };

                // 응답 저장
                store_response(response);

                // UI 스레드에 완료 알림
                // SAFETY: hwnd was reconstructed from a usize that was originally a valid
                // HWND from the UI thread. PostMessageW is safe to call from any thread
                // and only posts the message to the target window's message queue.
                // WM_TRANSLATION_COMPLETE is a custom message with the request ID as WPARAM.
                unsafe {
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
                use std::sync::atomic::{AtomicUsize, Ordering};
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
            match super::deepl::translate_async_with_client(client, text, source, target, key)
                .await
            {
                Ok(s) => return Ok(s),
                Err(e) => {
                    if Self::deepl_should_fallback(&e) && offset + 1 < keys.len() {
                        tracing::warn!(
                            "DeepL 키 #{} 실패(폴백): {}",
                            idx,
                            e
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
            TranslationError::Api { code, .. } => matches!(code, 429 | 456 | 403),
            _ => false,
        }
    }

    /// 비동기 번역 수행 (재시도 포함)
    async fn translate_async(req: &TranslationRequest, client: &reqwest::Client) -> TranslationResult {
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
    async fn translate_once(req: &TranslationRequest, client: &reqwest::Client) -> TranslationResult {
        match req.engine {
            TranslationEngine::EzTrans => {
                let text = req.text.clone();
                let source = req.source_lang;
                let target = req.target_lang;

                match tokio::task::spawn_blocking(move || {
                    super::translate_with_eztrans(&text, source, target)
                })
                .await
                {
                    Ok(result) => result,
                    Err(e) => Err(TranslationError::Engine(format!("EzTrans 실행 오류: {}", e))),
                }
            }
            TranslationEngine::Google => {
                super::google::translate_async_with_client(client, &req.text, req.source_lang, req.target_lang).await
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
                    EngineCredentials::Papago { client_id, client_secret } => {
                        (client_id.as_str(), client_secret.as_str())
                    }
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
                match params.provider {
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
                }
            }
        }
    }

    /// 번역 요청 전송
    pub fn request(&mut self, req: TranslationRequest) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        let mut request = req;
        request.id = id;

        // 새 요청 진입 시점에 "최신 ID" 갱신 → 워커가 이전 요청을 폐기할 수 있다.
        self.latest_id.store(id, Ordering::Release);

        if let Err(e) = self.sender.send(request) {
            tracing::error!("Failed to send translation request: {}", e);
        }

        id
    }

    /// 간편 요청 메서드
    pub fn translate(
        &mut self,
        text: String,
        engine: TranslationEngine,
        source_lang: Language,
        target_lang: Language,
        credentials: EngineCredentials,
    ) -> u64 {
        self.request(TranslationRequest {
            id: 0, // request()에서 할당
            text,
            engine,
            source_lang,
            target_lang,
            credentials,
        })
    }
}
