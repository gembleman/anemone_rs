use std::mem::zeroed;
use std::ptr::write_unaligned;

use crate::app::messages::WM_TRAY_ICON;
use crate::win32::to_wide;
use std::io;
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Shell::{
            NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
        },
        WindowsAndMessaging::{IDI_APPLICATION, LoadIconW, RegisterWindowMessageW},
    },
};

pub struct TrayIcon {
    nid: NOTIFYICONDATAW,
    registered: bool,
}

impl TrayIcon {
    pub fn new() -> Self {
        Self {
            // SAFETY: NOTIFYICONDATAW is a plain-old-data struct with no invariants;
            // zeroing all fields produces a valid initial state.
            nid: unsafe { zeroed() },
            registered: false,
        }
    }

    pub fn create(&mut self, hwnd: HWND, icon_id: u32) -> io::Result<()> {
        // SAFETY: hwnd is a valid window handle from the caller.
        unsafe {
            let hinstance = GetModuleHandleW(std::ptr::null());

            self.nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            self.nid.hWnd = hwnd;
            self.nid.uID = 1;
            self.nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            self.nid.uCallbackMessage = WM_TRAY_ICON;

            // 아이콘 로드 (리소스가 없으면 기본 아이콘 사용)
            let icon = LoadIconW(hinstance as HINSTANCE, icon_id as *const u16);
            self.nid.hIcon = if icon.is_null() {
                LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
            } else {
                icon
            };

            set_sz_tip(&raw mut self.nid.szTip, "아네모네");

            if Shell_NotifyIconW(NIM_ADD, &self.nid) == 0 {
                self.registered = false;
                return Err(io::Error::last_os_error());
            }
            self.registered = true;

            Ok(())
        }
    }

    pub fn remove(&mut self) {
        if self.registered {
            // SAFETY: self.nid was initialized in create() and is still valid.
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &self.nid);
            }
            self.registered = false;
        }
    }

    /// Explorer 재시작 시 트레이 아이콘 복원
    pub fn restore(&mut self) {
        if self.registered {
            // SAFETY: self.nid was initialized in create() and is still valid.
            // Re-adding after Explorer restart to restore the tray icon.
            unsafe {
                if Shell_NotifyIconW(NIM_ADD, &self.nid) == 0 {
                    tracing::warn!("Shell_NotifyIconW(NIM_ADD) failed during restore");
                }
            }
        }
    }
}

/// `szTip`을 null-terminated UTF-16으로 채운다.
///
/// # Safety
/// `ptr`은 쓰기 가능한 `[u16; 128]`을 가리켜야 하며 정렬은 필요 없다.
unsafe fn set_sz_tip(ptr: *mut [u16; 128], tip: &str) {
    const CAP: usize = 128;
    let base = ptr.cast::<u16>();
    let tip_wide = to_wide(tip);
    // 마지막 한 칸은 null 종결자용으로 비워둔다.
    let copy_len = tip_wide.len().min(CAP - 1);
    for (i, &c) in tip_wide.iter().take(copy_len).enumerate() {
        // SAFETY: base 는 [u16; 128] 의 첫 원소를 가리키며 i < 128 이다.
        unsafe { write_unaligned(base.add(i), c) };
    }
    // 나머지는 0 으로 채워 null 종결.
    for i in copy_len..CAP {
        // SAFETY: 위와 동일.
        unsafe { write_unaligned(base.add(i), 0) };
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        self.remove();
    }
}

/// TaskbarCreated 메시지 등록 (Explorer 재시작 감지용)
pub fn register_taskbar_created_message() -> u32 {
    // SAFETY: RegisterWindowMessageW with a valid static string is always safe.
    let message = crate::win32::to_wide("TaskbarCreated");
    unsafe { RegisterWindowMessageW(message.as_ptr()) }
}
