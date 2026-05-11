use windows::Win32::{
    Foundation::*,
    System::DataExchange::*,
    System::Memory::{GlobalLock, GlobalUnlock},
};

use crate::constants::CF_UNICODETEXT;

pub struct ClipboardWatcher {
    hwnd: HWND,
    watching: bool,
    ignore_next: bool,
}

impl ClipboardWatcher {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            watching: false,
            ignore_next: false,
        }
    }

    pub fn start(&mut self) {
        if !self.watching {
            // SAFETY: self.hwnd is a valid window handle. AddClipboardFormatListener
            // registers this window for WM_CLIPBOARDUPDATE notifications (Vista+).
            unsafe {
                if let Err(e) = AddClipboardFormatListener(self.hwnd) {
                    tracing::warn!("AddClipboardFormatListener failed: {e}");
                    return;
                }
            }
            self.watching = true;
            // 일부 환경에서 등록 직후 초기 WM_CLIPBOARDUPDATE 가 들어올 수 있어
            // 첫 알림은 무시한다.
            self.ignore_next = true;
        }
    }

    pub fn stop(&mut self) {
        if self.watching {
            // SAFETY: self.hwnd is a valid handle previously registered via
            // AddClipboardFormatListener.
            unsafe {
                let _ = RemoveClipboardFormatListener(self.hwnd);
            }
            self.watching = false;
        }
    }

    pub fn restart(&mut self) {
        self.stop();
        self.start();
    }

    pub fn is_watching(&self) -> bool {
        self.watching
    }

    /// WM_CLIPBOARDUPDATE 처리
    pub fn on_clipboard_update(&mut self) -> Option<String> {
        if self.ignore_next {
            self.ignore_next = false;
            return None;
        }
        self.get_text()
    }

    /// 클립보드에서 텍스트 읽기
    pub fn get_text(&self) -> Option<String> {
        // SAFETY: OpenClipboard/CloseClipboard are called in matched pairs. GetClipboardData
        // returns a valid handle when CF_UNICODETEXT is available. GlobalLock/GlobalUnlock
        // are called in pairs. The transmute converts HANDLE to HGLOBAL which have the same
        // representation. The pointer from GlobalLock is valid until GlobalUnlock.
        unsafe {
            if OpenClipboard(Some(self.hwnd)).is_err() {
                return None;
            }

            let result = (|| {
                if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
                    return None;
                }

                let handle = GetClipboardData(CF_UNICODETEXT).ok()?;
                let ptr = GlobalLock(std::mem::transmute(handle)) as *const u16;
                if ptr.is_null() {
                    return None;
                }

                // null-terminated UTF-16 문자열 길이 계산
                let mut len = 0;
                while *ptr.add(len) != 0 {
                    len += 1;
                }

                let slice = std::slice::from_raw_parts(ptr, len);
                let text = String::from_utf16_lossy(slice);

                let _ = GlobalUnlock(std::mem::transmute(handle));
                Some(text)
            })();

            let _ = CloseClipboard();
            result
        }
    }

    /// 다음 클립보드 변경 무시
    pub fn ignore_next_change(&mut self) {
        self.ignore_next = true;
    }
}

impl Drop for ClipboardWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}
