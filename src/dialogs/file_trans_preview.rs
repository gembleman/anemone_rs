//! 파일 번역 미리보기의 파일 읽기를 UI 스레드 밖에서 수행한다.

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

pub(super) const WM_FILE_TRANS_PREVIEW: u32 = WM_APP + 0x37;
static NEXT_NOTIFICATION_TOKEN: AtomicU64 = AtomicU64::new(1);

struct Request {
    generation: u64,
    path: PathBuf,
}

pub(super) struct PreviewResult {
    pub generation: u64,
    pub content: String,
}

pub(super) struct PreviewWorker {
    pub(super) token: u64,
    sender: Mutex<Option<Sender<Request>>>,
    results: Arc<Mutex<Vec<PreviewResult>>>,
    active: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl PreviewWorker {
    pub(super) fn spawn(hwnd: HWND) -> Option<Self> {
        let (sender, receiver) = mpsc::channel();
        let results = Arc::new(Mutex::new(Vec::new()));
        let worker_results = Arc::clone(&results);
        let active = Arc::new(AtomicBool::new(true));
        let worker_active = Arc::clone(&active);
        let hwnd_raw = hwnd as usize;
        let token = NEXT_NOTIFICATION_TOKEN.fetch_add(1, Ordering::Relaxed);
        let handle = std::thread::Builder::new()
            .name("anemone-file-preview".into())
            .spawn(move || Self::run(receiver, worker_results, worker_active, hwnd_raw, token))
            .ok()?;
        Some(Self {
            token,
            sender: Mutex::new(Some(sender)),
            results,
            active,
            handle: Mutex::new(Some(handle)),
        })
    }

    pub(super) fn request(&self, generation: u64, path: PathBuf) -> bool {
        self.sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|sender| sender.send(Request { generation, path }).is_ok())
    }

    pub(super) fn drain(&self) -> Vec<PreviewResult> {
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

    fn run(
        receiver: Receiver<Request>,
        results: Arc<Mutex<Vec<PreviewResult>>>,
        active: Arc<AtomicBool>,
        hwnd: usize,
        token: u64,
    ) {
        while let Ok(request) = receiver.recv() {
            let content = crate::file_trans::read_utf8_preview(&request.path, 7, 64 * 1024)
                .unwrap_or_else(|error| format!("! {error}"));
            if !active.load(Ordering::Acquire) {
                break;
            }
            results
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(PreviewResult {
                    generation: request.generation,
                    content,
                });
            // SAFETY: the dialog owns hwnd until it handles this result.
            let _ = unsafe { PostMessageW(hwnd as HWND, WM_FILE_TRANS_PREVIEW, token as usize, 0) };
        }
    }
}
