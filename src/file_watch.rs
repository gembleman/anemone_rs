//! 파일 감시 모듈
//!
//! 설정 파일 변경 감지 및 자동 리로드.
//! Win32 API (ReadDirectoryChangesW) 기반 구현.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use windows::Win32::{
    Foundation::*,
    Storage::FileSystem::{
        CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
        FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    },
    System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
    System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject},
};

// WaitForSingleObject 반환값 상수
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT_VALUE: u32 = 0x00000102;

/// 파일 변경 이벤트 타입
#[derive(Debug, Clone, PartialEq)]
pub enum FileChangeEvent {
    ConfigChanged,     // anemone_config.json
    DictionaryChanged, // anedic.txt
    Unknown(String),   // 기타 파일
}

/// 파일 감시자
pub struct FileWatcher {
    watch_path: PathBuf,
    tx: Sender<FileChangeEvent>,
    rx: Receiver<FileChangeEvent>,
    thread_handle: Option<JoinHandle<()>>,
    stop_flag: Arc<AtomicBool>,
    enabled: AtomicBool,
}

impl FileWatcher {
    /// 새 파일 감시자 생성
    pub fn new<P: AsRef<Path>>(watch_path: P) -> std::io::Result<Self> {
        let watch_path = watch_path.as_ref().to_path_buf();

        // 디렉토리 존재 확인
        if !watch_path.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Watch path is not a directory",
            ));
        }

        let (tx, rx) = mpsc::channel();
        let stop_flag = Arc::new(AtomicBool::new(false));

        Ok(Self {
            watch_path,
            tx,
            rx,
            thread_handle: None,
            stop_flag,
            enabled: AtomicBool::new(false),
        })
    }

    /// 감시 시작
    pub fn start(&mut self) -> std::io::Result<()> {
        if self.thread_handle.is_some() {
            return Ok(()); // 이미 실행 중
        }

        self.stop_flag.store(false, Ordering::SeqCst);
        self.enabled.store(true, Ordering::SeqCst);

        let watch_path = self.watch_path.clone();
        let tx = self.tx.clone();
        let stop_flag = self.stop_flag.clone();

        let handle = thread::spawn(move || {
            if let Err(e) = watch_directory(&watch_path, tx, stop_flag) {
                eprintln!("File watcher error: {:?}", e);
            }
        });

        self.thread_handle = Some(handle);
        Ok(())
    }

    /// 감시 중지
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        self.enabled.store(false, Ordering::SeqCst);

        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }

    /// 감시 활성화/비활성화
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    /// 활성화 상태 확인
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// 이벤트 폴링 (비블로킹)
    pub fn poll(&self) -> Option<FileChangeEvent> {
        if !self.is_enabled() {
            return None;
        }

        match self.rx.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }

    /// 모든 대기 중인 이벤트 가져오기
    pub fn drain_events(&self) -> Vec<FileChangeEvent> {
        let mut events = Vec::new();
        while let Some(event) = self.poll() {
            events.push(event);
        }
        events
    }
}

impl Drop for FileWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 디렉토리 감시 스레드 함수
fn watch_directory(
    path: &Path,
    tx: Sender<FileChangeEvent>,
    stop_flag: Arc<AtomicBool>,
) -> std::io::Result<()> {
    unsafe {
        // 디렉토리 핸들 열기
        let path_wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let handle = CreateFileW(
            windows::core::PCWSTR(path_wide.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            None,
        )?;

        if handle.is_invalid() {
            return Err(std::io::Error::last_os_error());
        }

        // 이벤트 핸들 생성
        let event = CreateEventW(None, true, false, None)?;

        // 버퍼 및 오버랩 구조체
        let mut buffer = [0u8; 4096];
        let mut overlapped = OVERLAPPED {
            hEvent: event,
            ..Default::default()
        };

        loop {
            if stop_flag.load(Ordering::SeqCst) {
                break;
            }

            // 비동기 디렉토리 변경 감시 시작
            let result = ReadDirectoryChangesW(
                handle,
                buffer.as_mut_ptr() as *mut _,
                buffer.len() as u32,
                false, // 비재귀
                FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME,
                None,
                Some(&mut overlapped),
                None,
            );

            if result.is_err() {
                break;
            }

            // 이벤트 대기 (1초 타임아웃)
            let wait_result = WaitForSingleObject(event, 1000);

            if stop_flag.load(Ordering::SeqCst) {
                break;
            }

            match wait_result.0 {
                WAIT_OBJECT_0 => {
                    // 변경 감지됨
                    let mut bytes_returned = 0u32;
                    if GetOverlappedResult(handle, &overlapped, &mut bytes_returned, false).is_ok()
                        && bytes_returned > 0
                    {
                        // 변경된 파일 파싱
                        parse_file_notify_information(&buffer[..bytes_returned as usize], &tx);
                    }

                    // 이벤트 리셋
                    let _ = ResetEvent(event);
                }
                WAIT_TIMEOUT_VALUE => {
                    // 타임아웃 - 계속 감시
                    let _ = CancelIoEx(handle, Some(&overlapped));
                    let _ = ResetEvent(event);
                }
                _ => {
                    // 에러
                    break;
                }
            }
        }

        // 정리
        let _ = CancelIoEx(handle, None);
        let _ = CloseHandle(event);
        let _ = CloseHandle(handle);

        Ok(())
    }
}

use std::os::windows::ffi::OsStrExt;

/// FILE_NOTIFY_INFORMATION 파싱
fn parse_file_notify_information(buffer: &[u8], tx: &Sender<FileChangeEvent>) {
    let mut offset = 0usize;

    while offset < buffer.len() {
        if offset + 12 > buffer.len() {
            break;
        }

        // FILE_NOTIFY_INFORMATION 구조체 읽기
        let next_offset = u32::from_le_bytes([
            buffer[offset],
            buffer[offset + 1],
            buffer[offset + 2],
            buffer[offset + 3],
        ]) as usize;

        let _action = u32::from_le_bytes([
            buffer[offset + 4],
            buffer[offset + 5],
            buffer[offset + 6],
            buffer[offset + 7],
        ]);

        let name_len = u32::from_le_bytes([
            buffer[offset + 8],
            buffer[offset + 9],
            buffer[offset + 10],
            buffer[offset + 11],
        ]) as usize;

        // 파일명 추출 (UTF-16)
        let name_start = offset + 12;
        let name_end = name_start + name_len;

        if name_end <= buffer.len() {
            let name_bytes = &buffer[name_start..name_end];
            let name_u16: Vec<u16> = name_bytes
                .chunks_exact(2)
                .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
                .collect();
            let filename = String::from_utf16_lossy(&name_u16);

            // 파일명에 따라 이벤트 분류
            let event = classify_file_change(&filename);
            let _ = tx.send(event);
        }

        // 다음 항목으로
        if next_offset == 0 {
            break;
        }
        offset += next_offset;
    }
}

/// 파일명에 따라 변경 이벤트 분류
fn classify_file_change(filename: &str) -> FileChangeEvent {
    let lower = filename.to_lowercase();

    if lower.contains("anemone_config") || lower.ends_with(".json") {
        FileChangeEvent::ConfigChanged
    } else if lower.contains("anedic") || lower == "anedic.txt" {
        FileChangeEvent::DictionaryChanged
    } else {
        FileChangeEvent::Unknown(filename.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_file_change() {
        assert_eq!(
            classify_file_change("anemone_config.json"),
            FileChangeEvent::ConfigChanged
        );
        assert_eq!(
            classify_file_change("anedic.txt"),
            FileChangeEvent::DictionaryChanged
        );
        assert_eq!(
            classify_file_change("other.txt"),
            FileChangeEvent::Unknown("other.txt".to_string())
        );
    }
}
