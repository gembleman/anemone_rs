//! 요청 큐 관리, 스레드/런타임 수명주기와 완료 응답 라우팅.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::sync::{Semaphore, watch};

use super::state::{CompletionNotifier, DispatchShared, PendingEntry, RouteSlot, TargetId};
use super::{TranslationRequest, TranslationRequestError, TranslationResponse};
use crate::translation::{TranslationError, TranslationResult};

/// 큐에 실린 작업 (요청 + 라우팅 대상)
pub(super) struct DispatchJob {
    target: TargetId,
    req: TranslationRequest,
    route: Arc<RouteSlot>,
    cancellation: watch::Receiver<u64>,
}

/// 프로세스 단일 디스패치
pub struct TranslationDispatch {
    /// Shutdown 시 `take()`하여 channel을 닫는다.
    /// poison 복구 안전: 잠금을 쥔 채로는 `Option` 대입/`take()`만 수행하므로
    /// panic이 나도 "절반만 비운 Sender" 같은 중간 상태가 생길 수 없다.
    pub(super) sender: Mutex<Option<Sender<DispatchJob>>>,
    pub(super) shared: Arc<DispatchShared>,
    /// 워커 스레드 핸들 (shutdown 시 join 용)
    /// poison 복구 안전: `sender`와 동일하게 `Option::take()`만 수행한다.
    pub(super) worker: Mutex<Option<JoinHandle<()>>>,
}

impl TranslationDispatch {
    pub(crate) fn spawn(
        notifier: Arc<dyn CompletionNotifier>,
        http_client: reqwest::Client,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<DispatchJob>();
        let shared = Arc::new(DispatchShared::new(notifier));
        let worker_shared = shared.clone();

        // Shutdown에서 join할 수 있도록 보관한다.
        let handle = std::thread::spawn(move || {
            Self::worker_thread(rx, worker_shared, http_client);
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
            let mut s = super::lock_or_recover(&self.sender);
            *s = None;
        }
        // 제한 시간 안에 join하지 못하면 shutdown이 멈추지 않도록 detach한다.
        let handle = {
            let mut w = super::lock_or_recover(&self.worker);
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
            let mut st = super::lock_or_recover(&self.shared.state);
            st.assign_id()
        };
        req.id = id;

        let route = self.shared.route(target);
        let cancellation = route.set_latest(id);

        // Shutdown 중인 요청은 조용히 버린다.
        let send_result = {
            let s = super::lock_or_recover(&self.sender);
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
    #[cfg(test)]
    pub(crate) fn take_response(&self, req_id: u64) -> Option<TranslationResponse> {
        let mut st = super::lock_or_recover(&self.shared.state);
        st.take_response(req_id)
    }

    /// HWND처럼 pointer-width인 대상에는 request ID를 message payload로 싣지 않고,
    /// 대상별 완료 큐에서 원래 64-bit ID와 응답을 함께 꺼낸다.
    pub(crate) fn take_response_for_target(
        &self,
        target: TargetId,
    ) -> Option<(u64, TranslationResponse)> {
        let mut st = super::lock_or_recover(&self.shared.state);
        st.take_response_for_target(target)
    }

    /// 소비자가 사라질 때 라우팅과 대기 응답을 함께 정리한다.
    pub(crate) fn unregister(&self, target: TargetId) {
        self.shared.unregister(target);
    }

    pub(crate) fn cancel(&self, target: TargetId) {
        self.shared.unregister(target);
    }

    /// 워커 스레드 진입점
    fn worker_thread(
        rx: Receiver<DispatchJob>,
        shared: Arc<DispatchShared>,
        client: reqwest::Client,
    ) {
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
            let _permit = if req.job.engine().is_blocking() {
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

    pub(super) async fn wait_until_superseded(
        cancellation: &mut watch::Receiver<u64>,
        req_id: u64,
    ) {
        loop {
            if *cancellation.borrow() != req_id {
                return;
            }
            if cancellation.changed().await.is_err() {
                return;
            }
        }
    }

    pub(super) fn route_response(
        shared: &DispatchShared,
        target: TargetId,
        route: &Arc<RouteSlot>,
        req_id: u64,
        result: TranslationResult,
    ) {
        // 같은 lock 안에서 세대 확인, 응답 저장, 통지를 끝내 TOCTOU를 막는다.
        let routes = super::lock_or_recover(&shared.routes);
        let Some(current) = routes.get(&target.get()) else {
            tracing::debug!("응답 #{} 폐기 (대상 등록 해제)", req_id);
            return;
        };
        if !Arc::ptr_eq(current, route) || route.latest_id.load(Ordering::Acquire) != req_id {
            tracing::debug!("응답 #{} 폐기 (stale 등록 세대/요청)", req_id);
            return;
        }

        {
            let mut state = super::lock_or_recover(&shared.state);
            state.push_response(PendingEntry {
                req_id,
                target,
                response: TranslationResponse { result },
            });
        }

        if let Err(error) = shared.notifier.notify(target, req_id) {
            let mut state = super::lock_or_recover(&shared.state);
            state.drop_response(req_id);
            tracing::warn!("번역 완료 통지 실패 (#{}): {}", req_id, error);
        }
        drop(routes);
    }
}
