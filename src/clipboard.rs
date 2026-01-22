use windows::Win32::{
    Foundation::*,
    System::DataExchange::*,
    System::Memory::{GlobalLock, GlobalUnlock},
    UI::WindowsAndMessaging::*,
};

const CF_UNICODETEXT: u32 = 13;

pub struct ClipboardWatcher {
    hwnd: HWND,
    next_viewer: HWND,
    watching: bool,
    ignore_next: bool,
}

impl ClipboardWatcher {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            next_viewer: HWND::default(),
            watching: false,
            ignore_next: false,
        }
    }

    pub fn start(&mut self) {
        if !self.watching {
            unsafe {
                self.next_viewer = SetClipboardViewer(self.hwnd).unwrap_or_default();
            }
            self.watching = true;
            self.ignore_next = true; // 초기 알림 무시
        }
    }

    pub fn stop(&mut self) {
        if self.watching {
            unsafe {
                let _ = ChangeClipboardChain(self.hwnd, self.next_viewer);
            }
            self.next_viewer = HWND::default();
            self.watching = false;
        }
    }

    #[allow(dead_code)]
    pub fn restart(&mut self) {
        self.stop();
        self.start();
    }

    #[allow(dead_code)]
    pub fn is_watching(&self) -> bool {
        self.watching
    }

    /// WM_DRAWCLIPBOARD 처리
    pub fn on_draw_clipboard(&mut self) -> Option<String> {
        // 다음 뷰어에게 전달
        if !self.next_viewer.is_invalid() {
            unsafe {
                let _ = SendMessageW(
                    self.next_viewer,
                    WM_DRAWCLIPBOARD,
                    Some(WPARAM(0)),
                    Some(LPARAM(0)),
                );
            }
        }

        // 무시 플래그 체크
        if self.ignore_next {
            self.ignore_next = false;
            return None;
        }

        // 클립보드 텍스트 읽기
        self.get_text()
    }

    /// WM_CHANGECBCHAIN 처리
    pub fn on_change_chain(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let removed = HWND(wparam.0 as *mut _);
        let next = HWND(lparam.0 as *mut _);

        if removed == self.next_viewer {
            self.next_viewer = next;
        } else if !self.next_viewer.is_invalid() {
            unsafe {
                let _ = SendMessageW(
                    self.next_viewer,
                    WM_CHANGECBCHAIN,
                    Some(wparam),
                    Some(lparam),
                );
            }
        }
    }

    /// 클립보드에서 텍스트 읽기
    pub fn get_text(&self) -> Option<String> {
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
    #[allow(dead_code)]
    pub fn ignore_next_change(&mut self) {
        self.ignore_next = true;
    }
}

impl Drop for ClipboardWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}
