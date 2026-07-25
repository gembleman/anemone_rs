//! 업데이트 확인·다운로드를 처리하는 전용 워커 스레드.
//!
//! `src/translation/worker.rs`의 스레드 구조(전용 스레드 + 자체 tokio 런타임 +
//! `std::sync::mpsc` 명령 큐)를 참고하지만, 번역 파이프라인과는 완전히 독립된
//! 워커다. 번역 요청이 몰려 있어도 업데이트 확인이 막히면 안 되고, 반대로
//! 업데이트 다운로드가 오래 걸려도 번역이 막히면 안 된다.
//!
//! `AppActionSender`(`src/app/action.rs`)는 `Rc<RefCell<..>>`을 들고 있어
//! `!Send`이므로 이 워커 스레드로 넘길 수 없다. 대신
//! `src/app/services.rs`의 `WindowMessageNotifier`와 같은 방식을 쓴다: HWND를
//! `usize`로 옮겨 Send 경계를 넘고, 결과는 공유 슬롯(`Arc<Mutex<Option<..>>>`)에
//! 넣은 뒤 `PostMessageW`로 UI 스레드를 깨운다. UI 스레드는 메시지를 받으면
//! 슬롯에서 결과를 꺼내 간다.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use super::check::UpdateCheck;
use super::download::{DownloadProgress, StagedUpdate};
use super::{AvailableUpdate, UpdateError, Version};

/// 확인 요청이 시작 시 자동으로 걸린 것인지, 사용자가 버튼을 눌러 요청한
/// 것인지 구분한다. UI 정책이 이 둘을 다르게 다룬다 —
/// `src/app/update.rs`의 모듈 문서를 참고.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckTrigger {
    Auto,
    Manual,
}

/// 워커에 보낼 수 있는 요청.
pub(crate) enum UpdateRequest {
    /// 최신 릴리스를 조회한다.
    Check {
        current: Version,
        trigger: CheckTrigger,
    },
    /// asset을 내려받아 `destination`에 저장하고 검증한다.
    Download {
        update: AvailableUpdate,
        destination: PathBuf,
    },
}

/// 워커가 UI 스레드에 돌려주는 결과.
pub(crate) enum UpdateOutcome {
    Check {
        result: Result<UpdateCheck, UpdateError>,
        trigger: CheckTrigger,
    },
    Download(Result<StagedUpdate, UpdateError>),
}

/// UI 스레드가 결과를 꺼내 갈 때까지 보관하는 공유 슬롯.
///
/// 요청은 한 번에 하나만 진행 중이라고 가정하지 않는다 — 여러 결과가 순서대로
/// 도착할 수 있으므로 FIFO 큐로 둔다.
type ResultSlot = Arc<Mutex<Vec<UpdateOutcome>>>;

/// 다운로드 진행도를 UI 스레드에 전하는 슬롯.
///
/// 결과와 달리 최신 값 하나만 의미가 있으므로 큐가 아니라 덮어쓰기다. 청크마다
/// 값이 갱신되는데 UI가 그보다 느리게 읽어도 중간 값을 놓칠 뿐 문제가 없다.
///
/// `Mutex` 대신 원자값을 쓴다. 다운로드 스레드가 청크마다 갱신하므로 UI 스레드의
/// 읽기와 경합하는데, 잠금을 두면 그 경합이 다운로드 루프를 직접 느리게 만든다.
/// `received`와 `total`을 따로 읽어 둘이 한 시점의 짝이 아닐 수 있지만, 진행률
/// 표시에서 한 프레임 어긋나는 것은 무해하다.
#[derive(Debug)]
pub(crate) struct ProgressSlot {
    received: AtomicU64,
    /// `u64::MAX`는 "총량 모름"(Content-Length 없음)을 뜻한다.
    total: AtomicU64,
}

/// `total`이 이 값이면 총량 미상이다.
const TOTAL_UNKNOWN: u64 = u64::MAX;

impl ProgressSlot {
    fn new() -> Self {
        Self {
            received: AtomicU64::new(0),
            total: AtomicU64::new(TOTAL_UNKNOWN),
        }
    }

    fn store(&self, progress: DownloadProgress) {
        self.total
            .store(progress.total.unwrap_or(TOTAL_UNKNOWN), Ordering::Relaxed);
        self.received.store(progress.received, Ordering::Relaxed);
    }

    /// 현재 진행도를 읽는다. 다운로드 중이 아니면 직전 값이 남아 있으므로,
    /// 호출자는 다운로드가 진행 중일 때만 의미 있게 해석해야 한다.
    pub(crate) fn load(&self) -> DownloadProgress {
        let total = match self.total.load(Ordering::Relaxed) {
            TOTAL_UNKNOWN => None,
            value => Some(value),
        };
        DownloadProgress {
            received: self.received.load(Ordering::Relaxed),
            total,
        }
    }

    /// 새 다운로드를 시작하기 전에 이전 값을 지운다.
    fn reset(&self) {
        self.received.store(0, Ordering::Relaxed);
        self.total.store(TOTAL_UNKNOWN, Ordering::Relaxed);
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum UpdateRequestError {
    #[error("업데이트 워커가 종료되어 요청을 받을 수 없습니다.")]
    WorkerUnavailable,
}

/// 업데이트 워커와 그 수명을 소유하는 핸들.
///
/// UI 스레드는 절대 블로킹하지 않는다: 요청은 채널로 비동기 전달되고, 결과는
/// [`ResultSlot`]에 쌓였다가 `PostMessageW`로 깨어난 UI 스레드가 꺼내 간다.
pub(crate) struct UpdateWorker {
    sender: Mutex<Option<Sender<UpdateRequest>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    results: ResultSlot,
    progress: Arc<ProgressSlot>,
}

impl UpdateWorker {
    /// 워커 스레드를 띄운다.
    ///
    /// `hwnd`는 결과 도착을 알릴 창이다. HWND 자체는 `!Send`이므로 포인터 값을
    /// `usize`로 옮겨 스레드 경계를 넘고, 워커 스레드 안에서 다시 `HWND`로
    /// 복원해 `PostMessageW`를 호출한다.
    ///
    /// `progress_message`는 다운로드 진행도가 갱신됐을 때 게시할 메시지다.
    /// 결과용 `message`와 구분해, UI가 결과 큐를 비우지 않고도 진행도만 다시
    /// 그릴 수 있게 한다.
    pub(crate) fn spawn(hwnd: HWND, message: u32, progress_message: u32) -> Self {
        let (tx, rx) = mpsc::channel::<UpdateRequest>();
        let results: ResultSlot = Arc::new(Mutex::new(Vec::new()));
        let worker_results = results.clone();
        let progress = Arc::new(ProgressSlot::new());
        let worker_progress = progress.clone();
        let hwnd_raw = hwnd.0 as usize;

        let handle = thread::Builder::new()
            .name("anemone-update".to_string())
            .spawn(move || {
                Self::worker_thread(
                    rx,
                    worker_results,
                    worker_progress,
                    hwnd_raw,
                    message,
                    progress_message,
                );
            })
            .map_err(|error| {
                tracing::error!("업데이트 워커 스레드를 시작하지 못했습니다: {error}");
            })
            .ok();

        Self {
            sender: Mutex::new(Some(tx)),
            handle: Mutex::new(handle),
            results,
            progress,
        }
    }

    /// 현재 다운로드 진행도. `WM_UPDATE_PROGRESS`를 받은 UI가 호출한다.
    pub(crate) fn progress(&self) -> DownloadProgress {
        self.progress.load()
    }

    /// 요청을 큐에 넣고 즉시 반환한다. UI 스레드를 블로킹하지 않는다.
    pub(crate) fn request(&self, request: UpdateRequest) -> Result<(), UpdateRequestError> {
        let sender = self.sender.lock().expect("update sender poisoned");
        match sender.as_ref() {
            Some(tx) => tx
                .send(request)
                .map_err(|_| UpdateRequestError::WorkerUnavailable),
            None => Err(UpdateRequestError::WorkerUnavailable),
        }
    }

    /// 도착한 결과를 모두 꺼낸다. 도착 순서를 보존한다.
    pub(crate) fn drain_results(&self) -> Vec<UpdateOutcome> {
        let mut results = self.results.lock().expect("update results poisoned");
        std::mem::take(&mut *results)
    }

    /// 채널을 닫고 워커 스레드를 제한 시간 안에 join한다.
    ///
    /// `src/translation/worker.rs::TranslationDispatch::shutdown`과 동일하게,
    /// 제한 시간 안에 끝나지 않으면 detach하고 넘어가 종료가 멈추지 않게 한다.
    pub(crate) fn shutdown(&self) {
        {
            let mut sender = self.sender.lock().expect("update sender poisoned");
            *sender = None;
        }
        let handle = {
            let mut handle = self.handle.lock().expect("update worker handle poisoned");
            handle.take()
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
                tracing::warn!("update worker did not finish in {MAX_WAIT:?}, leaving detached");
            }
        }
    }

    fn worker_thread(
        rx: Receiver<UpdateRequest>,
        results: ResultSlot,
        progress: Arc<ProgressSlot>,
        hwnd_raw: usize,
        message: u32,
        progress_message: u32,
    ) {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(error) => {
                tracing::error!("업데이트 워커의 tokio 런타임을 만들지 못했습니다: {error}");
                return;
            }
        };

        while let Ok(request) = rx.recv() {
            let outcome = rt.block_on(Self::handle_request(
                request,
                &progress,
                hwnd_raw,
                progress_message,
            ));
            {
                let mut results = results.lock().expect("update results poisoned");
                results.push(outcome);
            }
            Self::notify(hwnd_raw, message);
        }
    }

    async fn handle_request(
        request: UpdateRequest,
        progress: &ProgressSlot,
        hwnd_raw: usize,
        progress_message: u32,
    ) -> UpdateOutcome {
        match request {
            UpdateRequest::Check { current, trigger } => UpdateOutcome::Check {
                result: super::check::fetch_latest(&current).await,
                trigger,
            },
            UpdateRequest::Download {
                update,
                destination,
            } => {
                progress.reset();
                // 청크마다 PostMessageW를 보내면 UI 메시지 큐가 잠긴다. 슬롯은
                // 매번 갱신하되 알림은 시간으로 제한한다 — UI는 알림을 받을 때
                // 슬롯에서 최신 값을 읽으므로 중간 알림이 빠져도 손해가 없다.
                const NOTIFY_INTERVAL: Duration = Duration::from_millis(100);
                let mut last_notify: Option<std::time::Instant> = None;
                let on_progress = |value: super::download::DownloadProgress| {
                    progress.store(value);
                    let now = std::time::Instant::now();
                    let due = last_notify
                        .is_none_or(|previous| now.duration_since(previous) >= NOTIFY_INTERVAL);
                    if due {
                        last_notify = Some(now);
                        Self::notify(hwnd_raw, progress_message);
                    }
                };

                let result = super::download::download(&update, destination, on_progress).await;
                // 마지막 상태를 반드시 한 번 더 알린다. 위 제한 때문에 100%가
                // 통째로 누락된 채 끝날 수 있다.
                Self::notify(hwnd_raw, progress_message);
                UpdateOutcome::Download(result)
            }
        }
    }

    /// 결과가 준비됐음을 UI 스레드에 알린다.
    fn notify(hwnd_raw: usize, message: u32) {
        let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
        // SAFETY: UI 스레드가 소유한 창은 App이 종료될 때까지 유효하다.
        // `shutdown()`은 App 소멸 경로에서 호출되므로, 그 뒤에는 이 워커에게
        // 더 이상 새 요청이 들어오지 않고 이 알림도 발생하지 않는다.
        let result = unsafe { PostMessageW(Some(hwnd), message, WPARAM(0), LPARAM(0)) };
        if let Err(error) = result {
            tracing::warn!("업데이트 결과 알림을 게시하지 못했습니다: {error}");
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/update/worker.rs"]
mod tests;
