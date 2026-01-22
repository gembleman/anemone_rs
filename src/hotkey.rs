use windows::{
    core::*,
    Win32::{
        Foundation::*,
        UI::Input::KeyboardAndMouse::*,
    },
};

use crate::menu;

// 핫키 ID
pub mod id {
    pub const TOGGLE_WINDOW: i32 = 1;
    pub const TEXT_SIZE_UP: i32 = 2;
    pub const TEXT_SIZE_DOWN: i32 = 3;
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

    pub fn register_defaults(&mut self) -> Result<()> {
        // Ctrl+Shift+A: 윈도우 토글
        self.register(
            id::TOGGLE_WINDOW,
            MOD_CONTROL | MOD_SHIFT,
            VK_A.0 as u32,
        )?;

        // Ctrl+Shift+Up: 텍스트 크기 증가
        self.register(
            id::TEXT_SIZE_UP,
            MOD_CONTROL | MOD_SHIFT,
            VK_UP.0 as u32,
        )?;

        // Ctrl+Shift+Down: 텍스트 크기 감소
        self.register(
            id::TEXT_SIZE_DOWN,
            MOD_CONTROL | MOD_SHIFT,
            VK_DOWN.0 as u32,
        )?;

        Ok(())
    }

    pub fn register(&mut self, id: i32, modifiers: HOT_KEY_MODIFIERS, vk: u32) -> Result<()> {
        unsafe {
            RegisterHotKey(Some(self.hwnd), id, modifiers, vk)?;
            self.registered.push(id);
            Ok(())
        }
    }

    pub fn unregister(&mut self, id: i32) {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), id);
            self.registered.retain(|&x| x != id);
        }
    }

    pub fn unregister_all(&mut self) {
        for &id in &self.registered.clone() {
            self.unregister(id);
        }
    }

    /// 핫키 ID를 메뉴 명령 ID로 변환
    pub fn to_menu_command(hotkey_id: i32) -> Option<u16> {
        match hotkey_id {
            id::TOGGLE_WINDOW => Some(menu::id::WINDOW_SHOW),
            id::TEXT_SIZE_UP => Some(menu::id::TEXT_SIZE_UP),
            id::TEXT_SIZE_DOWN => Some(menu::id::TEXT_SIZE_DOWN),
            _ => None,
        }
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.unregister_all();
    }
}
