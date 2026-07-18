use windows::Win32::{
    Foundation::*,
    System::DataExchange::*,
    System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
    System::Ole::CF_UNICODETEXT,
};
use windows::core::{Error, Result};

pub enum ClipboardUpdate {
    Unchanged,
    Text(String),
    Retry(Error),
    Failed(Error),
}

pub(crate) struct ClipboardGuard;

impl ClipboardGuard {
    pub(crate) fn open(owner: HWND) -> Result<Self> {
        unsafe { OpenClipboard(Some(owner))? };
        Ok(Self)
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

pub(crate) struct OwnedGlobalMemory(Option<HGLOBAL>);

impl OwnedGlobalMemory {
    pub(crate) fn allocate(bytes: usize) -> Result<Self> {
        unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }.map(|handle| Self(Some(handle)))
    }

    pub(crate) fn handle(&self) -> HGLOBAL {
        self.0
            .expect("global memory exists until ownership transfer")
    }

    pub(crate) fn release_to_system(&mut self) {
        self.0 = None;
    }
}

impl Drop for OwnedGlobalMemory {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            unsafe {
                let _ = GlobalFree(Some(handle));
            }
        }
    }
}

pub struct ClipboardWatcher {
    hwnd: HWND,
    watching: bool,
    last_sequence: u32,
    pending_sequence: u32,
    read_failures: u8,
}

impl ClipboardWatcher {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            watching: false,
            last_sequence: 0,
            pending_sequence: 0,
            read_failures: 0,
        }
    }

    pub fn start(&mut self) -> Result<()> {
        if !self.watching {
            // SAFETY: self.hwnd is a valid window handle. AddClipboardFormatListener
            // registers this window for WM_CLIPBOARDUPDATE notifications (Vista+).
            unsafe {
                AddClipboardFormatListener(self.hwnd)?;
            }
            self.watching = true;
            // 등록 직후 동일 sequence의 초기 알림만 무시한다. 초기 알림이 없는
            // 환경에서 사용자의 첫 변경을 버리지 않는다.
            self.last_sequence = unsafe { GetClipboardSequenceNumber() };
            self.pending_sequence = 0;
            self.read_failures = 0;
        }
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if self.watching {
            // SAFETY: self.hwnd is a valid handle previously registered via
            // AddClipboardFormatListener.
            unsafe {
                RemoveClipboardFormatListener(self.hwnd)?;
            }
            self.watching = false;
        }
        Ok(())
    }

    pub fn is_watching(&self) -> bool {
        self.watching
    }

    /// WM_CLIPBOARDUPDATE 처리
    pub fn on_clipboard_update(&mut self) -> ClipboardUpdate {
        let sequence = unsafe { GetClipboardSequenceNumber() };
        if sequence != 0 && sequence == self.last_sequence {
            return ClipboardUpdate::Unchanged;
        }
        if sequence != self.pending_sequence {
            self.pending_sequence = sequence;
            self.read_failures = 0;
        }
        match self.get_text() {
            Ok(text) => {
                self.last_sequence = sequence;
                self.pending_sequence = 0;
                self.read_failures = 0;
                text.map_or(ClipboardUpdate::Unchanged, ClipboardUpdate::Text)
            }
            Err(error) => {
                self.read_failures = self.read_failures.saturating_add(1);
                if self.read_failures < 3 {
                    ClipboardUpdate::Retry(error)
                } else {
                    self.last_sequence = sequence;
                    self.pending_sequence = 0;
                    self.read_failures = 0;
                    ClipboardUpdate::Failed(error)
                }
            }
        }
    }

    /// 클립보드에서 텍스트 읽기
    pub fn get_text(&self) -> Result<Option<String>> {
        // SAFETY: GetClipboardData returns a HANDLE which we wrap into HGLOBAL — both are
        // `*mut c_void` newtypes for the same kernel handle representation. GlobalLock/
        // GlobalUnlock are paired and the returned pointer is valid until GlobalUnlock.
        unsafe {
            let _clipboard = ClipboardGuard::open(self.hwnd)?;

            let format = CF_UNICODETEXT.0 as u32;
            (|| {
                if IsClipboardFormatAvailable(format).is_err() {
                    return Ok(None);
                }

                let handle = GetClipboardData(format)?;
                let hglobal = windows::Win32::Foundation::HGLOBAL(handle.0);
                // 글로벌 메모리의 실제 바이트 길이를 알아내 UTF-16 길이 계산의
                // 안전 상한으로 쓴다. 손상된 데이터에 null 종결자가 없어도
                // OOR 읽기로 발산하지 않게 한다. GlobalSize 가 0 을 반환하면
                // (실패 또는 빈 핸들) 즉시 None.
                let byte_size = GlobalSize(hglobal);
                if byte_size == 0 {
                    return Err(Error::new(
                        E_FAIL,
                        "클립보드 text buffer 크기를 읽을 수 없습니다",
                    ));
                }
                // CF_UNICODETEXT 는 항상 짝수 바이트여야 한다. 홀수면 손상된 데이터
                // 가능성 — UTF-16 마지막 1 바이트는 절단(silent truncation)된다.
                if !byte_size.is_multiple_of(2) {
                    tracing::warn!(
                        "CF_UNICODETEXT 가 홀수 바이트 크기({}); 손상 가능성, 마지막 바이트 무시",
                        byte_size
                    );
                }
                let max_words = byte_size / 2;

                let ptr = GlobalLock(hglobal) as *const u16;
                if ptr.is_null() {
                    return Err(Error::new(
                        E_FAIL,
                        "클립보드 text buffer를 잠글 수 없습니다",
                    ));
                }

                // null-terminated UTF-16 문자열 길이 계산 — max_words 로 상한.
                let mut len = 0;
                while len < max_words && *ptr.add(len) != 0 {
                    len += 1;
                }

                let slice = std::slice::from_raw_parts(ptr, len);
                let text = String::from_utf16_lossy(slice);

                let _ = GlobalUnlock(hglobal);
                Ok(Some(text))
            })()
        }
    }
}

impl Drop for ClipboardWatcher {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            tracing::warn!("RemoveClipboardFormatListener failed during drop: {error}");
        }
    }
}
