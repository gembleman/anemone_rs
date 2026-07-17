use windows::Win32::{
    Foundation::*,
    System::DataExchange::*,
    System::Memory::{GlobalLock, GlobalSize, GlobalUnlock},
    System::Ole::CF_UNICODETEXT,
};

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
        // returns a HANDLE which we wrap into HGLOBAL — both are `*mut c_void` newtypes for
        // the same kernel handle representation. GlobalLock/GlobalUnlock are paired and the
        // returned pointer is valid until GlobalUnlock.
        unsafe {
            if OpenClipboard(Some(self.hwnd)).is_err() {
                return None;
            }

            let format = CF_UNICODETEXT.0 as u32;
            let result = (|| {
                if IsClipboardFormatAvailable(format).is_err() {
                    return None;
                }

                let handle = GetClipboardData(format).ok()?;
                let hglobal = windows::Win32::Foundation::HGLOBAL(handle.0);
                // 글로벌 메모리의 실제 바이트 길이를 알아내 UTF-16 길이 계산의
                // 안전 상한으로 쓴다. 손상된 데이터에 null 종결자가 없어도
                // OOR 읽기로 발산하지 않게 한다. GlobalSize 가 0 을 반환하면
                // (실패 또는 빈 핸들) 즉시 None.
                let byte_size = GlobalSize(hglobal);
                if byte_size == 0 {
                    return None;
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
                    return None;
                }

                // null-terminated UTF-16 문자열 길이 계산 — max_words 로 상한.
                let mut len = 0;
                while len < max_words && *ptr.add(len) != 0 {
                    len += 1;
                }

                let slice = std::slice::from_raw_parts(ptr, len);
                let text = String::from_utf16_lossy(slice);

                let _ = GlobalUnlock(hglobal);
                Some(text)
            })();

            let _ = CloseClipboard();
            result
        }
    }
}

impl Drop for ClipboardWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}
