//! GUI 애플리케이션의 장수명 서비스와 Win32 번역 완료 어댑터.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::file_trans::FileTranslationSupervisor;
use crate::translation::worker::{
    CompletionNotifier, TargetId, TranslationDispatch, TranslationRequest, TranslationRequestError,
    TranslationResponse,
};
use crate::translation::{PreparedJob, TranslationService};
use crate::update::worker::{UpdateOutcome, UpdateRequest, UpdateRequestError, UpdateWorker};

use super::messages::{WM_TRANSLATION_COMPLETE, WM_UPDATE_RESULT};
use super::translation_cache::TranslationCacheStore;

/// GUI bootstrap에서 생성해 App과 dialog에 주입하는 장수명 서비스 집합.
pub(crate) struct AppServices {
    pub translation_ui: Rc<GuiTranslationHost>,
    pub file_translation: Rc<FileTranslationSupervisor>,
    pub translation_cache: Rc<TranslationCacheStore>,
    pub update: Rc<UpdateWorker>,
}

impl AppServices {
    pub fn new(hwnd: HWND) -> Self {
        let translation = TranslationService::new();
        let http_client = translation.http_client();
        let translation_ui = Rc::new(GuiTranslationHost::new(http_client.clone()));
        let file_translation = Rc::new(FileTranslationSupervisor::with_http_client(http_client));
        let translation_cache =
            Rc::new(TranslationCacheStore::open(&crate::runtime::cache_db_file()));
        let update = Rc::new(UpdateWorker::spawn(hwnd, WM_UPDATE_RESULT));
        Self {
            translation_ui,
            file_translation,
            translation_cache,
            update,
        }
    }

    pub fn shutdown(&self) {
        self.translation_ui.shutdown();
        let report = self.file_translation.shutdown(Duration::from_secs(2));
        if report.detached > 0 {
            tracing::warn!(
                detached_tasks = report.detached,
                "file translation tasks exceeded shutdown grace"
            );
        }
        self.update.shutdown();
    }
}

/// 업데이트 확인/다운로드 요청 편의 함수. 워커가 사라졌으면 조용히 로그만 남긴다
/// — 자동 확인 실패로 사용자를 방해하지 않는다는 정책과 같은 이유다.
impl AppServices {
    pub(crate) fn request_update_check(&self, current: crate::update::Version) {
        if let Err(UpdateRequestError::WorkerUnavailable) =
            self.update.request(UpdateRequest::Check { current })
        {
            tracing::warn!("업데이트 워커가 종료되어 확인 요청을 보낼 수 없습니다");
        }
    }

    /// 다운로드를 트리거하는 UI는 다음 작업에서 붙는다. 그때까지 워커의 Download
    /// 요청 경로는 이 메서드를 통해서만 호출되므로 미리 준비해 둔다.
    #[allow(dead_code)]
    pub(crate) fn request_update_download(
        &self,
        update: crate::update::AvailableUpdate,
        destination: std::path::PathBuf,
    ) {
        if let Err(UpdateRequestError::WorkerUnavailable) =
            self.update.request(UpdateRequest::Download {
                update,
                destination,
            })
        {
            tracing::warn!("업데이트 워커가 종료되어 다운로드 요청을 보낼 수 없습니다");
        }
    }

    pub(crate) fn drain_update_results(&self) -> Vec<UpdateOutcome> {
        self.update.drain_results()
    }
}

struct WindowMessageNotifier;

impl CompletionNotifier for WindowMessageNotifier {
    fn notify(&self, target: TargetId, _request_id: u64) -> Result<(), String> {
        let hwnd = HWND(target.get() as *mut std::ffi::c_void);
        // SAFETY: 대상 등록 세대 잠금을 보유한 워커가 호출한다. UI 파괴 경로의
        // unregister도 같은 잠금을 통과하므로 해제된 세대에는 게시하지 않는다.
        unsafe {
            PostMessageW(
                Some(hwnd),
                WM_TRANSLATION_COMPLETE,
                // message는 대상별 completion queue를 비우라는 신호로만 사용한다.
                WPARAM(0),
                LPARAM(0),
            )
        }
        .map_err(|error| error.to_string())
    }
}

fn target(hwnd: HWND) -> TargetId {
    TargetId::new(hwnd.0 as usize)
}

/// Win32 완료 통지와 dispatcher 수명을 명시적으로 소유하는 GUI 번역 서비스.
pub(crate) struct GuiTranslationHost {
    dispatch: TranslationDispatch,
}

impl GuiTranslationHost {
    fn new(http_client: reqwest::Client) -> Self {
        Self {
            dispatch: TranslationDispatch::spawn(Arc::new(WindowMessageNotifier), http_client),
        }
    }

    pub(crate) fn request(
        &self,
        hwnd: HWND,
        text: Arc<str>,
        job: PreparedJob,
    ) -> Result<u64, TranslationRequestError> {
        self.dispatch
            .request(target(hwnd), TranslationRequest { id: 0, text, job })
    }

    pub(crate) fn take_response(&self, hwnd: HWND) -> Option<(u64, TranslationResponse)> {
        self.dispatch.take_response_for_target(target(hwnd))
    }

    pub(crate) fn unregister(&self, hwnd: HWND) {
        self.dispatch.unregister(target(hwnd));
    }

    pub(crate) fn cancel(&self, hwnd: HWND) {
        self.dispatch.cancel(target(hwnd));
    }

    pub(crate) fn shutdown(&self) {
        self.dispatch.shutdown();
    }
}
