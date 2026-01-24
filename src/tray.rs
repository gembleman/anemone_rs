use std::mem::zeroed;
use std::ptr::{addr_of_mut, write_unaligned};

use windows::{
    Win32::{
        Foundation::*, System::LibraryLoader::GetModuleHandleW, UI::Shell::*,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

pub const WM_TRAY_ICON: u32 = WM_USER + 1;

pub struct TrayIcon {
    nid: NOTIFYICONDATAW,
    registered: bool,
}

impl TrayIcon {
    pub fn new() -> Self {
        Self {
            nid: unsafe { zeroed() },
            registered: false,
        }
    }

    pub fn create(&mut self, hwnd: HWND, icon_id: u32) -> Result<()> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            self.nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            self.nid.hWnd = hwnd;
            self.nid.uID = 1;
            self.nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            self.nid.uCallbackMessage = WM_TRAY_ICON;

            // 아이콘 로드 (리소스가 없으면 기본 아이콘 사용)
            let icon = LoadIconW(Some(hinstance.into()), PCWSTR(icon_id as *const u16));
            self.nid.hIcon =
                icon.unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default());

            // 툴팁 설정
            let tip = "아네모네";
            let tip_wide: Vec<u16> = tip.encode_utf16().chain(std::iter::once(0)).collect();
            // packed struct 문제 회피: write_unaligned 사용 (정렬되지 않은 주소 접근)
            let sz_tip_ptr = addr_of_mut!(self.nid.szTip) as *mut u16;
            let sz_tip_len = 128; // NOTIFYICONDATAW.szTip의 고정 크기
            let copy_len = tip_wide.len().min(sz_tip_len);
            for i in 0..copy_len {
                write_unaligned(sz_tip_ptr.add(i), tip_wide[i]);
            }

            let _ = Shell_NotifyIconW(NIM_ADD, &self.nid);
            self.registered = true;

            Ok(())
        }
    }

    pub fn remove(&mut self) {
        if self.registered {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &self.nid);
            }
            self.registered = false;
        }
    }

    /// Explorer 재시작 시 트레이 아이콘 복원
    pub fn restore(&mut self) {
        if self.registered {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_ADD, &self.nid);
            }
        }
    }

    #[allow(dead_code)]
    pub fn update_tooltip(&mut self, tip: &str) {
        unsafe {
            let tip_wide: Vec<u16> = tip.encode_utf16().chain(std::iter::once(0)).collect();
            // packed struct 문제 회피: write_unaligned 사용 (정렬되지 않은 주소 접근)
            let sz_tip_ptr = addr_of_mut!(self.nid.szTip) as *mut u16;
            let sz_tip_len = 128; // NOTIFYICONDATAW.szTip의 고정 크기
            let copy_len = tip_wide.len().min(sz_tip_len);
            for i in 0..copy_len {
                write_unaligned(sz_tip_ptr.add(i), tip_wide[i]);
            }
        }

        if self.registered {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &self.nid);
            }
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        self.remove();
    }
}

/// TaskbarCreated 메시지 등록 (Explorer 재시작 감지용)
pub fn register_taskbar_created_message() -> u32 {
    unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }
}
