use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(any(test, feature = "benchmark"))]
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::pipeline::{FilePipelineServices, run_with_services};
use super::{FileTranslationError, FileTranslationProgress, FileTranslationRequest};
use crate::translation::EzTransProcessPoolRegistry;

/// 파일 번역 작업을 취소하는 스레드 안전한 핸들.
#[derive(Clone)]
pub struct CancelHandle(pub(super) Arc<AtomicBool>);

impl CancelHandle {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// 취소가 실제로 전파됐는지 확인하는 테스트만 쓴다.
    #[cfg(test)]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// 진행 이벤트 수신과 작업별 취소를 소유하는 파일 번역 task.
pub struct FileTranslationTask {
    cancel: CancelHandle,
    events: Receiver<FileTranslationProgress>,
}

impl FileTranslationTask {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn drain_events(&self) -> Vec<FileTranslationProgress> {
        self.events.try_iter().collect()
    }

    /// UI는 메시지 루프에서 `drain_events`로 훑는다. 이벤트가 올 때까지 막고
    /// 기다리는 이 경로는 테스트·벤치 하네스 전용이다.
    #[cfg(any(test, feature = "benchmark"))]
    pub fn recv_event_timeout(
        &self,
        timeout: Duration,
    ) -> Result<FileTranslationProgress, RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }

    /// 취소 전파를 확인하는 테스트만 쓴다.
    #[cfg(test)]
    pub fn cancel_handle(&self) -> CancelHandle {
        self.cancel.clone()
    }
}

impl Drop for FileTranslationTask {
    fn drop(&mut self) {
        // UI thread에서 join하지 않는다. supervisor가 앱 종료 시 bounded join한다.
        self.cancel();
    }
}

struct WorkerEntry {
    cancel: CancelHandle,
    handle: JoinHandle<()>,
}

/// 잠금을 얻는다. 잠금을 쥔 스레드가 panic해 poison됐더라도 worker 목록 자체는
/// (HashMap insert/remove 같은 panic-safe 연산만 하므로) 여전히 유효하다.
/// 여기서 다시 panic하면 poison이 연쇄돼 이후 모든 supervisor 호출이 죽으므로,
/// 이전 상태를 그대로 복구해 계속 진행한다.
fn lock_workers(
    workers: &Mutex<HashMap<u64, WorkerEntry>>,
) -> std::sync::MutexGuard<'_, HashMap<u64, WorkerEntry>> {
    workers
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 종료 시 회수한 작업과 유예 시간 뒤 분리한 작업 수.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShutdownReport {
    pub joined: usize,
    pub detached: usize,
}

/// 파일 worker의 생성, 취소와 bounded join을 명시적으로 소유한다.
pub struct FileTranslationSupervisor {
    accepting: AtomicBool,
    next_id: AtomicU64,
    workers: Mutex<HashMap<u64, WorkerEntry>>,
    pipeline_services: FilePipelineServices,
}

impl Default for FileTranslationSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTranslationSupervisor {
    pub fn new() -> Self {
        Self::with_http_client(crate::translation::http_common::create_client())
    }

    pub(crate) fn with_http_client(http_client: reqwest::Client) -> Self {
        Self {
            accepting: AtomicBool::new(true),
            next_id: AtomicU64::new(1),
            workers: Mutex::new(HashMap::new()),
            pipeline_services: FilePipelineServices::new(
                http_client,
                Arc::new(EzTransProcessPoolRegistry::new()),
            ),
        }
    }

    pub fn start(
        &self,
        mut job: FileTranslationRequest,
    ) -> Result<FileTranslationTask, FileTranslationError> {
        self.reap_finished();
        let mut workers = lock_workers(&self.workers);
        if !self.accepting.load(Ordering::Acquire) {
            return Err(FileTranslationError::Runtime(
                "파일 번역 서비스가 종료 중입니다".into(),
            ));
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, events) = mpsc::channel();
        let cancel = CancelHandle(Arc::new(AtomicBool::new(false)));
        job.cancel_token = cancel.0.clone();
        let pipeline_services = self.pipeline_services.clone();
        let handle = std::thread::Builder::new()
            .name(format!("anemone-file-{id}"))
            .spawn(move || {
                run_with_services(&job, &pipeline_services, |event| {
                    let _ = sender.send(event);
                })
            })
            .map_err(|error| FileTranslationError::Runtime(error.to_string()))?;
        workers.insert(
            id,
            WorkerEntry {
                cancel: cancel.clone(),
                handle,
            },
        );
        Ok(FileTranslationTask { cancel, events })
    }

    /// 끝난 worker가 회수됐는지 확인하는 테스트만 쓴다.
    #[cfg(test)]
    pub fn active_tasks(&self) -> usize {
        self.reap_finished()
    }

    /// 신규 요청 차단 → 전체 취소 → 유예 시간 내 join 순서로 종료한다.
    pub fn shutdown(&self, timeout: Duration) -> ShutdownReport {
        self.accepting.store(false, Ordering::Release);
        let deadline = Instant::now() + timeout;
        let mut report = ShutdownReport::default();

        loop {
            let finished = {
                let mut workers = lock_workers(&self.workers);
                for entry in workers.values() {
                    entry.cancel.cancel();
                }
                let ids = workers
                    .iter()
                    .filter_map(|(&id, entry)| entry.handle.is_finished().then_some(id))
                    .collect::<Vec<_>>();
                ids.into_iter()
                    .filter_map(|id| workers.remove(&id))
                    .collect::<Vec<_>>()
            };
            for entry in finished {
                let _ = entry.handle.join();
                report.joined += 1;
            }

            let remaining = lock_workers(&self.workers).len();
            if remaining == 0 {
                return report;
            }
            if Instant::now() >= deadline {
                let mut workers = lock_workers(&self.workers);
                report.detached += workers.len();
                workers.clear();
                return report;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn reap_finished(&self) -> usize {
        let finished = {
            let mut workers = lock_workers(&self.workers);
            let ids = workers
                .iter()
                .filter_map(|(&id, entry)| entry.handle.is_finished().then_some(id))
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| workers.remove(&id))
                .collect::<Vec<_>>()
        };
        for entry in finished {
            let _ = entry.handle.join();
        }
        lock_workers(&self.workers).len()
    }
}

impl Drop for FileTranslationSupervisor {
    fn drop(&mut self) {
        let report = self.shutdown(Duration::from_millis(500));
        if report.detached > 0 {
            tracing::warn!(
                detached_tasks = report.detached,
                "file translation shutdown grace elapsed"
            );
        }
    }
}
