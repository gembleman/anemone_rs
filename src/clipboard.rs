use std::io;

use windows_sys::Win32::{
    Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND},
    System::DataExchange::{
        AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
        GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
        RemoveClipboardFormatListener, SetClipboardData,
    },
    System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
    System::Ole::CF_UNICODETEXT,
};

pub enum ClipboardUpdate {
    Unchanged,
    Text(String),
    /// clipboard_max_length 초과 — UTF-16 단위 수로 변환 전에 확정된 경우.
    TooLong,
    Retry(io::Error),
    Failed(io::Error),
}

/// `get_text`의 정상 결과. 오류는 `Result`의 `Err`로, 포맷 부재는 `None`으로 구분한다.
pub enum TextRead {
    None,
    Text(String),
    TooLong,
}

pub(crate) struct ClipboardGuard;

impl ClipboardGuard {
    pub(crate) fn open(owner: HWND) -> io::Result<Self> {
        if unsafe { OpenClipboard(owner) } == 0 {
            return Err(io::Error::last_os_error());
        }
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
    pub(crate) fn allocate(bytes: usize) -> io::Result<Self> {
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) };
        if handle.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(Some(handle)))
        }
    }

    /// 소유 중인 핸들. 이미 `release_to_system`으로 소유권을 넘겼거나 drop된
    /// 뒤에는 `None` — 호출부는 이를 클립보드 설정 포기로 처리해야 한다.
    pub(crate) fn handle(&self) -> Option<HGLOBAL> {
        self.0
    }

    pub(crate) fn release_to_system(&mut self) {
        self.0 = None;
    }
}

impl Drop for OwnedGlobalMemory {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            unsafe {
                let _ = GlobalFree(handle);
            }
        }
    }
}

/// 유니코드 텍스트를 클립보드에 올린다.
///
/// 성공하면 `GlobalAlloc` 블록의 소유권이 시스템으로 넘어가고, 중간에 실패하면
/// `OwnedGlobalMemory`의 Drop이 블록을 되돌려 준다.
pub(crate) fn set_text(owner: HWND, text: &str) -> io::Result<()> {
    let wide = crate::win32::to_wide(text);
    let _clipboard = ClipboardGuard::open(owner)?;
    // SAFETY: ClipboardGuard가 방금 연 클립보드에 대해서만 호출한다.
    if unsafe { EmptyClipboard() } == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut memory = OwnedGlobalMemory::allocate(wide.len() * 2)?;
    let Some(handle) = memory.handle() else {
        return Err(io::Error::other(
            "clipboard global memory handle을 사용할 수 없습니다",
        ));
    };
    // SAFETY: handle은 방금 할당한 wide.len() * 2바이트짜리 GMEM_MOVEABLE 블록이고,
    // lock이 성공한 구간에서만 그 크기만큼 그대로 채운다.
    unsafe {
        let ptr = GlobalLock(handle).cast::<u16>();
        if ptr.is_null() {
            return Err(io::Error::last_os_error());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        let _ = GlobalUnlock(handle);
        if SetClipboardData(CF_UNICODETEXT as u32, handle as HANDLE).is_null() {
            return Err(io::Error::last_os_error());
        }
    }
    memory.release_to_system();
    Ok(())
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

    pub fn start(&mut self) -> io::Result<()> {
        if !self.watching {
            // SAFETY: self.hwnd is a valid window handle. AddClipboardFormatListener
            // registers this window for WM_CLIPBOARDUPDATE notifications (Vista+).
            if unsafe { AddClipboardFormatListener(self.hwnd) } == 0 {
                return Err(io::Error::last_os_error());
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

    pub fn stop(&mut self) -> io::Result<()> {
        if self.watching {
            // SAFETY: self.hwnd is a valid handle previously registered via
            // AddClipboardFormatListener.
            if unsafe { RemoveClipboardFormatListener(self.hwnd) } == 0 {
                return Err(io::Error::last_os_error());
            }
            self.watching = false;
        }
        Ok(())
    }

    /// 창이 파괴되는 동안 listener를 해제한다.
    ///
    /// `WM_DESTROY` 이후에는 `hwnd`가 더 이상 유효하지 않으므로, Win32 해제 호출의
    /// 성공 여부와 관계없이 watcher를 종료 상태로 만든다. 창 파괴 자체가 남은 listener
    /// 등록을 무효화하므로 `Drop`에서 죽은 handle로 다시 해제할 필요가 없다.
    pub fn stop_for_window_destroy(&mut self) -> io::Result<()> {
        let result = self.stop();
        self.watching = false;
        result
    }

    pub fn is_watching(&self) -> bool {
        self.watching
    }

    /// WM_CLIPBOARDUPDATE 처리
    pub fn on_clipboard_update(&mut self, max_len: usize) -> ClipboardUpdate {
        let sequence = unsafe { GetClipboardSequenceNumber() };
        if sequence != 0 && sequence == self.last_sequence {
            return ClipboardUpdate::Unchanged;
        }
        if sequence != self.pending_sequence {
            self.pending_sequence = sequence;
            self.read_failures = 0;
        }
        match self.get_text(max_len) {
            Ok(TextRead::None) => {
                self.last_sequence = sequence;
                self.pending_sequence = 0;
                self.read_failures = 0;
                ClipboardUpdate::Unchanged
            }
            Ok(TextRead::Text(text)) => {
                self.last_sequence = sequence;
                self.pending_sequence = 0;
                self.read_failures = 0;
                ClipboardUpdate::Text(text)
            }
            Ok(TextRead::TooLong) => {
                self.last_sequence = sequence;
                self.pending_sequence = 0;
                self.read_failures = 0;
                ClipboardUpdate::TooLong
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

    /// 클립보드에서 텍스트 읽기.
    ///
    /// UTF-16 단위 수 `u`와 char 수 `c`는 `c ≤ u ≤ 2c`가 성립하므로,
    /// `u > max_len * 2`면 변환 없이 확실히 초과다. 거대 클립보드(파일 전체
    /// 복사 등)를 `String`으로 만들지 않도록 `max_len > 0`일 때 그 경우를
    /// 미리 거른다.
    pub fn get_text(&self, max_len: usize) -> io::Result<TextRead> {
        let _clipboard = ClipboardGuard::open(self.hwnd)?;

        let format = CF_UNICODETEXT as u32;
        (|| {
            // SAFETY: the clipboard is open for this thread until `_clipboard` drops.
            if unsafe { IsClipboardFormatAvailable(format) } == 0 {
                return Ok(TextRead::None);
            }

            // SAFETY: GetClipboardData returns a HANDLE which we wrap into HGLOBAL — both are
            // `*mut c_void` newtypes for the same kernel handle representation.
            let handle: HANDLE = unsafe { GetClipboardData(format) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let hglobal: HGLOBAL = handle;
            // 글로벌 메모리의 실제 바이트 길이를 알아내 UTF-16 길이 계산의
            // 안전 상한으로 쓴다. 손상된 데이터에 null 종결자가 없어도
            // OOR 읽기로 발산하지 않게 한다. GlobalSize 가 0 을 반환하면
            // (실패 또는 빈 핸들) 즉시 None.
            // SAFETY: hglobal is the non-null handle the clipboard just returned.
            let byte_size = unsafe { GlobalSize(hglobal) };
            if byte_size == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
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

            // SAFETY: every GlobalLock below is paired with a GlobalUnlock on each exit
            // path, and the returned pointer stays valid until that GlobalUnlock.
            let ptr = unsafe { GlobalLock(hglobal) } as *const u16;
            if ptr.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "클립보드 text buffer를 잠글 수 없습니다",
                ));
            }

            // null-terminated UTF-16 문자열 길이 계산 — max_words 로 상한.
            let mut len = 0;
            // SAFETY: the buffer is locked and `len < max_words` keeps every read inside
            // the byte range GlobalSize reported.
            while len < max_words && unsafe { *ptr.add(len) } != 0 {
                len += 1;
            }

            // UTF-16 단위 수(u)와 char 수(c)는 c ≤ u ≤ 2c이므로, u > max_len × 2면
            // 변환 없이 확실히 초과다 — 거대 클립보드를 String으로 만들지 않는다.
            if max_len > 0 && len > max_len.saturating_mul(2) {
                // SAFETY: unlocks the buffer locked above before leaving the closure.
                unsafe {
                    let _ = GlobalUnlock(hglobal);
                }
                return Ok(TextRead::TooLong);
            }

            // SAFETY: [ptr, ptr + len) lies inside the locked buffer — the loop above
            // stopped at or before max_words — and nothing unlocks it before the copy.
            let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
            let text = String::from_utf16_lossy(slice);

            // SAFETY: pairs with the GlobalLock above; `slice` is no longer used.
            unsafe {
                let _ = GlobalUnlock(hglobal);
            }
            Ok(TextRead::Text(text))
        })()
    }
}

impl Drop for ClipboardWatcher {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            tracing::warn!("RemoveClipboardFormatListener failed during drop: {error}");
        }
    }
}
