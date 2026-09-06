//! 설정 파일 저장을 UI 스레드 밖에서 처리한다.

use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use crate::config::Config;

pub(super) struct ConfigSaveWorker {
    sender: Mutex<Option<Sender<Config>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl ConfigSaveWorker {
    pub(super) fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("anemone-config-save".to_string())
            .spawn(move || Self::worker_thread(rx))
            .map_err(|error| tracing::error!("설정 저장 워커를 시작하지 못했습니다: {error}"))
            .ok();
        Self {
            sender: Mutex::new(Some(tx)),
            handle: Mutex::new(handle),
        }
    }

    pub(super) fn request(&self, config: Config) -> bool {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        sender.as_ref().is_some_and(|tx| tx.send(config).is_ok())
    }

    /// 남은 저장을 끝낸 뒤 워커를 닫는다.
    pub(super) fn shutdown(&self) {
        *self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        if let Some(handle) = self
            .handle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = handle.join();
        }
    }

    fn worker_thread(rx: Receiver<Config>) {
        while let Ok(mut config) = rx.recv() {
            // 아직 저장을 시작하지 않은 요청은 최신 값 하나로 합친다.
            for newer in rx.try_iter() {
                config = newer;
            }
            if let Err(error) = config.save() {
                tracing::error!("설정 저장 실패: {error}");
            }
        }
    }
}
