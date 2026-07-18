//! Win32 번역 완료 통지 어댑터.
//!
//! 번역 워커는 불투명 대상 ID와 [`CompletionNotifier`]만 알고, 이 모듈이
//! `HWND` 변환과 `PostMessageW`를 전담한다.

use std::sync::{Arc, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::constants::WM_TRANSLATION_COMPLETE;
use crate::translation::worker::{
    CompletionNotifier, EngineCredentials, TargetId, TranslationDispatch, TranslationRequest,
    TranslationRequestError, TranslationResponse,
};
use crate::translation::{Language, TranslationEngine};

struct WindowMessageNotifier;

impl CompletionNotifier for WindowMessageNotifier {
    fn notify(&self, target: TargetId, request_id: u64) -> Result<(), String> {
        let hwnd = HWND(target.get() as *mut std::ffi::c_void);
        // SAFETY: 대상 등록 세대 잠금을 보유한 워커가 호출한다. UI 파괴 경로의
        // unregister도 같은 잠금을 통과하므로 해제된 세대에는 게시하지 않는다.
        unsafe {
            PostMessageW(
                Some(hwnd),
                WM_TRANSLATION_COMPLETE,
                WPARAM(request_id as usize),
                LPARAM(0),
            )
        }
        .map_err(|error| error.to_string())
    }
}

static DISPATCH: OnceLock<TranslationDispatch> = OnceLock::new();

fn dispatch() -> &'static TranslationDispatch {
    DISPATCH.get_or_init(|| TranslationDispatch::spawn(Arc::new(WindowMessageNotifier)))
}

fn target(hwnd: HWND) -> TargetId {
    TargetId::new(hwnd.0 as usize)
}

pub(crate) fn request_translation(
    hwnd: HWND,
    text: String,
    engine: TranslationEngine,
    source_lang: Language,
    target_lang: Language,
    credentials: EngineCredentials,
) -> Result<u64, TranslationRequestError> {
    dispatch().request(
        target(hwnd),
        TranslationRequest {
            id: 0,
            text: Arc::from(text),
            engine,
            source_lang,
            target_lang,
            credentials,
        },
    )
}

pub(crate) fn take_response(request_id: u64) -> Option<TranslationResponse> {
    dispatch().take_response(request_id)
}

pub(crate) fn unregister_translation_hwnd(hwnd: HWND) {
    dispatch().unregister(target(hwnd));
}

pub(crate) fn cancel_translation(hwnd: HWND) {
    dispatch().cancel(target(hwnd));
}

pub(crate) fn shutdown() {
    if let Some(dispatch) = DISPATCH.get() {
        dispatch.shutdown();
    }
}
