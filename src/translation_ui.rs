//! Win32 번역 완료 통지 어댑터.
//!
//! 번역 워커는 불투명 대상 ID와 [`CompletionNotifier`]만 알고, 이 모듈이
//! `HWND` 변환과 `PostMessageW`를 전담한다.

use std::sync::Arc;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::constants::WM_TRANSLATION_COMPLETE;
use crate::translation::PreparedJob;
use crate::translation::worker::{
    CompletionNotifier, TargetId, TranslationDispatch, TranslationRequest, TranslationRequestError,
    TranslationResponse,
};

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
                // request ID는 u64이므로 32-bit WPARAM에 싣지 않는다. message는
                // 대상별 completion queue를 비우라는 신호로만 사용한다.
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
    _service: crate::translation::TranslationService,
    dispatch: TranslationDispatch,
}

impl GuiTranslationHost {
    pub(crate) fn new(service: crate::translation::TranslationService) -> Self {
        let http_client = service.http_client();
        Self {
            _service: service,
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
