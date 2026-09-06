//! EzTrans 경로 검사를 UI 스레드 밖에서 처리한다.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use super::{SettingsDialog, ctrl_id};
use crate::translation::settings::TranslationSettingsEditor;

pub(super) const WM_EZTRANS_PATH_RESULT: u32 = WM_APP + 0x34;

#[derive(Clone)]
struct PathRequest {
    dictionary: String,
    ehnd: String,
}

struct PathResult {
    request: PathRequest,
    dictionary_invalid: bool,
    ehnd_invalid: bool,
}

type ResultSlot = Arc<Mutex<Vec<PathResult>>>;

pub(super) struct EzTransPathWorker {
    sender: Mutex<Option<Sender<PathRequest>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    results: ResultSlot,
    active: Arc<AtomicBool>,
}

impl EzTransPathWorker {
    pub(super) fn spawn(hwnd: HWND) -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let results = Arc::new(Mutex::new(Vec::new()));
        let worker_results = Arc::clone(&results);
        let active = Arc::new(AtomicBool::new(true));
        let worker_active = Arc::clone(&active);
        let hwnd_raw = hwnd as usize;
        let handle = std::thread::Builder::new()
            .name("anemone-eztrans-path".to_string())
            .spawn(move || Self::worker_thread(rx, worker_results, worker_active, hwnd_raw))
            .map_err(|error| tracing::error!("EzTrans 경로 검사 워커 시작 실패: {error}"))
            .ok()?;
        Some(Self {
            sender: Mutex::new(Some(tx)),
            handle: Mutex::new(Some(handle)),
            results,
            active,
        })
    }

    fn request(&self, request: PathRequest) -> bool {
        self.sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|tx| tx.send(request).is_ok())
    }

    fn drain_results(&self) -> Vec<PathResult> {
        let mut results = self
            .results
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *results)
    }

    pub(super) fn shutdown(&self) {
        self.active.store(false, Ordering::Release);
        *self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        if let Some(handle) = self
            .handle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            && handle.is_finished()
        {
            let _ = handle.join();
        }
    }

    fn worker_thread(
        rx: Receiver<PathRequest>,
        results: ResultSlot,
        active: Arc<AtomicBool>,
        hwnd_raw: usize,
    ) {
        while let Ok(request) = rx.recv() {
            let result = Self::validate(request);
            if !active.load(Ordering::Acquire) {
                break;
            }
            results
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(result);
            if unsafe { PostMessageW(hwnd_raw as HWND, WM_EZTRANS_PATH_RESULT, 0, 0) } == 0 {
                tracing::warn!("EzTrans 경로 검사 결과 알림 실패");
            }
        }
    }

    fn validate(request: PathRequest) -> PathResult {
        let dictionary_invalid = !request.dictionary.trim().is_empty()
            && TranslationSettingsEditor::eztrans_dictionary_invalid(&request.dictionary);
        let ehnd_invalid = !request.ehnd.trim().is_empty()
            && TranslationSettingsEditor::eztrans_ehnd_invalid(&request.ehnd);
        PathResult {
            request,
            dictionary_invalid,
            ehnd_invalid,
        }
    }
}

impl SettingsDialog {
    pub(super) fn request_eztrans_path_validation(&self) {
        if self.eztrans_path_validation_pending.get() {
            return;
        }
        let request = {
            let draft = self.draft.borrow();
            PathRequest {
                dictionary: draft.translation.eztrans_dictionary_path.clone(),
                ehnd: draft.translation.eztrans_ehnd_path.clone(),
            }
        };
        let accepted = {
            let mut slot = self.eztrans_path_worker.borrow_mut();
            if slot.is_none() {
                *slot = EzTransPathWorker::spawn(self.hwnd);
            }
            slot.as_ref().is_some_and(|worker| worker.request(request))
        };
        if accepted {
            self.eztrans_path_validation_pending.set(true);
            self.set_control_text(ctrl_id::EZTRANS_DICTIONARY_WARNING_LABEL, "경로 확인 중...");
            self.set_control_text(ctrl_id::EZTRANS_EHND_WARNING_LABEL, "경로 확인 중...");
        }
    }

    pub(super) fn handle_eztrans_path_result(&self) {
        let results = {
            let slot = self.eztrans_path_worker.borrow();
            match slot.as_ref() {
                Some(worker) => worker.drain_results(),
                None => return,
            }
        };
        self.eztrans_path_validation_pending.set(false);
        let Some(result) = results.into_iter().last() else {
            return;
        };
        let current = {
            let draft = self.draft.borrow();
            (
                draft.translation.eztrans_dictionary_path.clone(),
                draft.translation.eztrans_ehnd_path.clone(),
            )
        };
        if current
            != (
                result.request.dictionary.clone(),
                result.request.ehnd.clone(),
            )
        {
            self.request_eztrans_path_validation();
            return;
        }
        self.apply_eztrans_path_result(&result);
    }

    fn apply_eztrans_path_result(&self, result: &PathResult) {
        let dictionary_empty = result.request.dictionary.trim().is_empty();
        let ehnd_empty = result.request.ehnd.trim().is_empty();
        self.set_control_text(
            ctrl_id::EZTRANS_DICTIONARY_WARNING_LABEL,
            if dictionary_empty {
                "⚠ EzTrans 평면 사전이 선택되지 않았습니다. JisJK.flat.bin을 선택하세요."
            } else if result.dictionary_invalid {
                "⚠ EzTrans 평면 사전이 아닙니다. JisJK.flat.bin을 선택하세요."
            } else {
                ""
            },
        );
        self.set_control_text(
            ctrl_id::EZTRANS_EHND_WARNING_LABEL,
            if ehnd_empty {
                "⚠ EzTrans 필터 폴더가 선택되지 않았습니다. Ehnd 폴더를 선택하세요."
            } else if result.ehnd_invalid {
                "⚠ EzTrans 필터 폴더가 아닙니다. Ehnd 폴더를 선택하세요."
            } else {
                ""
            },
        );
    }

    #[cfg(test)]
    pub(super) fn refresh_eztrans_path_warnings(&self) {
        let request = {
            let draft = self.draft.borrow();
            PathRequest {
                dictionary: draft.translation.eztrans_dictionary_path.clone(),
                ehnd: draft.translation.eztrans_ehnd_path.clone(),
            }
        };
        self.apply_eztrans_path_result(&EzTransPathWorker::validate(request));
    }
}
