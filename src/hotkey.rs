use windows_core::*;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey,
};

use crate::config::{HotkeyConfig, HotkeySlot};

// 핫키 ID
pub mod id {
    pub const TOGGLE_WINDOW: i32 = 1;
    pub const TEXT_SIZE_UP: i32 = 2;
    pub const TEXT_SIZE_DOWN: i32 = 3;
    pub const CLIPBOARD_WATCH: i32 = 4;
}

pub struct HotkeyManager {
    hwnd: HWND,
    registered: Vec<i32>,
}

impl HotkeyManager {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            registered: Vec::new(),
        }
    }

    /// `HotkeyConfig`에 저장된 사용자 지정 단축키를 등록한다.
    /// 하나라도 등록에 실패하면(다른 앱이 이미 점유 등) 이번 호출에서 등록한 것만 롤백한다.
    pub fn register_from_config(&mut self, config: &HotkeyConfig) -> Result<()> {
        let registered_before = self.registered.len();
        let result = (|| {
            for (slot, spec) in config.entries() {
                let hotkey_id = match slot {
                    HotkeySlot::ToggleWindow => id::TOGGLE_WINDOW,
                    HotkeySlot::TextSizeUp => id::TEXT_SIZE_UP,
                    HotkeySlot::TextSizeDown => id::TEXT_SIZE_DOWN,
                    HotkeySlot::ClipboardWatch => id::CLIPBOARD_WATCH,
                };
                self.register(hotkey_id, spec.modifiers(), spec.vk)?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.unregister_from(registered_before);
        }
        result
    }

    pub fn register(&mut self, id: i32, modifiers: HOT_KEY_MODIFIERS, vk: u32) -> Result<()> {
        // SAFETY: self.hwnd is a valid window handle. RegisterHotKey associates the hotkey
        // with this window using a unique id provided by the caller.
        unsafe {
            if RegisterHotKey(self.hwnd, id, modifiers, vk) == 0 {
                return Err(Error::from_thread());
            }
            self.registered.push(id);
            Ok(())
        }
    }

    pub fn unregister_all(&mut self) {
        self.unregister_from(0);
    }

    fn unregister_from(&mut self, start: usize) {
        for &id in self.registered[start..].iter().rev() {
            // SAFETY: self.hwnd is a valid window handle. UnregisterHotKey removes a hotkey
            // previously registered with RegisterHotKey using the same hwnd and id.
            unsafe {
                let _ = UnregisterHotKey(self.hwnd, id);
            }
        }
        self.registered.truncate(start);
    }

    /// Win32 핫키 ID를 설정에서 사용하는 안정적인 슬롯으로 변환한다.
    pub fn slot_for_id(hotkey_id: i32) -> Option<HotkeySlot> {
        match hotkey_id {
            id::TOGGLE_WINDOW => Some(HotkeySlot::ToggleWindow),
            id::TEXT_SIZE_UP => Some(HotkeySlot::TextSizeUp),
            id::TEXT_SIZE_DOWN => Some(HotkeySlot::TextSizeDown),
            id::CLIPBOARD_WATCH => Some(HotkeySlot::ClipboardWatch),
            _ => None,
        }
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.unregister_all();
    }
}
