//! 단축키 탭의 리스트 표시와 키 조합 캡처.

use windows::{
    Win32::{
        Foundation::*,
        UI::{
            Controls::*,
            Input::KeyboardAndMouse::*,
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    },
    core::*,
};

use super::{SettingsDialog, ctrl_id};
use crate::config::{HotkeyConfig, HotkeySlot, HotkeySpec};
use crate::win32::to_wide;

pub(super) const WM_HOTKEY_CAPTURED: u32 = WM_APP + 0x31;
const HOTKEY_LIST_SUBCLASS_ID: usize = 1;

const HOTKEY_SLOTS: [HotkeySlot; 4] = [
    HotkeySlot::ToggleWindow,
    HotkeySlot::TextSizeUp,
    HotkeySlot::TextSizeDown,
    HotkeySlot::ClipboardWatch,
];

fn slot_from_row(row: usize) -> Option<HotkeySlot> {
    HOTKEY_SLOTS.get(row).copied()
}

fn is_modifier_key(vk: u32) -> bool {
    [
        VK_CONTROL,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_SHIFT,
        VK_LSHIFT,
        VK_RSHIFT,
        VK_MENU,
        VK_LMENU,
        VK_RMENU,
        VK_LWIN,
        VK_RWIN,
    ]
    .iter()
    .any(|candidate| candidate.0 as u32 == vk)
}

fn selected_row(list: HWND) -> Option<usize> {
    let row = unsafe {
        SendMessageW(
            list,
            LVM_GETNEXTITEM,
            Some(WPARAM(usize::MAX)),
            Some(LPARAM(LVNI_SELECTED as isize)),
        )
        .0
    };
    (row >= 0).then_some(row as usize)
}

fn key_is_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk.0 as i32) < 0 }
}

fn hotkey_from_state(vk: u32, ctrl: bool, shift: bool, alt: bool, win: bool) -> HotkeySpec {
    HotkeySpec::new(ctrl, shift, alt, win, vk)
}

fn reset_hotkeys(hotkeys: &mut HotkeyConfig) -> bool {
    let defaults = HotkeyConfig::default();
    if *hotkeys == defaults {
        false
    } else {
        *hotkeys = defaults;
        true
    }
}

fn captured_hotkey(vk: u32) -> HotkeySpec {
    hotkey_from_state(
        vk,
        key_is_down(VK_CONTROL),
        key_is_down(VK_SHIFT),
        key_is_down(VK_MENU),
        key_is_down(VK_LWIN) || key_is_down(VK_RWIN),
    )
}

/// 리스트가 포커스를 가진 동안 수정자 키 입력을 소비하고, 비수정자 키가 내려오는
/// 순간의 전체 조합을 부모 설정 창에 전달한다.
unsafe extern "system" fn hotkey_list_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    unsafe {
        match msg {
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let Some(row) = selected_row(hwnd) else {
                    return DefSubclassProc(hwnd, msg, wparam, lparam);
                };
                let vk = wparam.0 as u32;
                // Ctrl -> Shift -> K 순으로 들어와도 수정자만으로는 절대 확정하지 않는다.
                // 자동 반복 역시 최초 입력 한 번만 반영한다.
                if !is_modifier_key(vk)
                    && (lparam.0 & (1 << 30)) == 0
                    && let Ok(parent) = GetParent(hwnd)
                {
                    let _ = SendMessageW(
                        parent,
                        WM_HOTKEY_CAPTURED,
                        Some(WPARAM(row)),
                        Some(LPARAM(vk as isize)),
                    );
                }
                LRESULT(0)
            }
            WM_KEYUP | WM_SYSKEYUP | WM_CHAR | WM_SYSCHAR if selected_row(hwnd).is_some() => {
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(hotkey_list_subclass_proc), subclass_id);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

impl SettingsDialog {
    pub(super) fn initialize_hotkey_list(&self) -> Result<()> {
        let list = self.control(ctrl_id::HOTKEYS_LIST)?;
        unsafe {
            let styles = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER;
            let _ = SendMessageW(
                list,
                LVM_SETEXTENDEDLISTVIEWSTYLE,
                Some(WPARAM(styles as usize)),
                Some(LPARAM(styles as isize)),
            );

            let mut client = RECT::default();
            GetClientRect(list, &mut client)?;
            let width = (client.right - client.left).max(2);
            self.insert_hotkey_column(list, 0, "기능", width * 48 / 100)?;
            self.insert_hotkey_column(list, 1, "단축키", width * 52 / 100)?;

            for (row, slot) in HOTKEY_SLOTS.iter().enumerate() {
                self.insert_hotkey_row(list, row, slot.label())?;
            }
            self.refresh_hotkey_list();

            if !SetWindowSubclass(
                list,
                Some(hotkey_list_subclass_proc),
                HOTKEY_LIST_SUBCLASS_ID,
                0,
            )
            .as_bool()
            {
                return Err(Error::new(
                    E_FAIL,
                    "단축키 입력 처리를 초기화할 수 없습니다",
                ));
            }
        }
        Ok(())
    }

    unsafe fn insert_hotkey_column(
        &self,
        list: HWND,
        index: usize,
        title: &str,
        width: i32,
    ) -> Result<()> {
        let mut text = to_wide(title);
        let column = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH | LVCF_SUBITEM,
            cx: width,
            pszText: PWSTR(text.as_mut_ptr()),
            iSubItem: index as i32,
            ..Default::default()
        };
        let result = unsafe {
            SendMessageW(
                list,
                LVM_INSERTCOLUMNW,
                Some(WPARAM(index)),
                Some(LPARAM(&column as *const LVCOLUMNW as isize)),
            )
            .0
        };
        if result < 0 {
            Err(Error::new(E_FAIL, "단축키 목록 열을 만들 수 없습니다"))
        } else {
            Ok(())
        }
    }

    unsafe fn insert_hotkey_row(&self, list: HWND, row: usize, label: &str) -> Result<()> {
        let mut text = to_wide(label);
        let item = LVITEMW {
            mask: LVIF_TEXT,
            iItem: row as i32,
            pszText: PWSTR(text.as_mut_ptr()),
            ..Default::default()
        };
        let result = unsafe {
            SendMessageW(
                list,
                LVM_INSERTITEMW,
                Some(WPARAM(0)),
                Some(LPARAM(&item as *const LVITEMW as isize)),
            )
            .0
        };
        if result < 0 {
            Err(Error::new(E_FAIL, "단축키 목록 항목을 만들 수 없습니다"))
        } else {
            Ok(())
        }
    }

    pub(super) fn handle_hotkey_capture(&mut self, row: usize, vk: u32) {
        let Some(slot) = slot_from_row(row) else {
            return;
        };
        let new_spec = captured_hotkey(vk);

        if !new_spec.is_valid() {
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "단축키 오류",
                "이 키는 단축키로 지원되지 않습니다.",
            );
            return;
        }

        let mut hotkeys = self.draft.borrow().hotkeys.clone();
        hotkeys.set(slot, new_spec);
        if let Some((a, b)) = hotkeys.find_conflict() {
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "단축키 충돌",
                &format!(
                    "'{}'와(과) '{}'에 같은 단축키({new_spec})가 배정되었습니다.\n다른 조합을 입력해주세요.",
                    a.label(),
                    b.label()
                ),
            );
            return;
        }

        self.draft.borrow_mut().hotkeys = hotkeys;
        self.has_unapplied_changes.set(true);
        self.refresh_hotkey_list();
    }

    pub(super) fn reset_hotkeys_to_default(&mut self) {
        let changed = reset_hotkeys(&mut self.draft.borrow_mut().hotkeys);
        if changed {
            self.has_unapplied_changes.set(true);
            self.refresh_hotkey_list();
        }
    }

    pub(super) fn refresh_hotkey_list(&self) {
        let Ok(list) = self.control(ctrl_id::HOTKEYS_LIST) else {
            return;
        };
        let hotkeys = self.draft.borrow().hotkeys.clone();
        for (row, slot) in HOTKEY_SLOTS.iter().enumerate() {
            let mut text = to_wide(&hotkeys.get(*slot).to_string());
            let item = LVITEMW {
                iSubItem: 1,
                pszText: PWSTR(text.as_mut_ptr()),
                ..Default::default()
            };
            unsafe {
                let _ = SendMessageW(
                    list,
                    LVM_SETITEMTEXTW,
                    Some(WPARAM(row)),
                    Some(LPARAM(&item as *const LVITEMW as isize)),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_keys_are_not_complete_hotkeys() {
        for key in [
            VK_CONTROL,
            VK_LCONTROL,
            VK_RCONTROL,
            VK_SHIFT,
            VK_LSHIFT,
            VK_RSHIFT,
            VK_MENU,
            VK_LMENU,
            VK_RMENU,
            VK_LWIN,
            VK_RWIN,
        ] {
            assert!(is_modifier_key(key.0 as u32));
        }
        assert!(!is_modifier_key(VK_K.0 as u32));
    }

    #[test]
    fn main_key_keeps_all_pressed_modifiers() {
        let spec = hotkey_from_state(VK_K.0 as u32, true, true, false, false);
        assert_eq!(spec.to_string(), "Ctrl+Shift+K");
        assert!(spec.ctrl);
        assert!(spec.shift);
        assert_eq!(spec.vk, VK_K.0 as u32);
    }

    #[test]
    fn list_rows_map_to_stable_hotkey_slots() {
        assert_eq!(slot_from_row(0), Some(HotkeySlot::ToggleWindow));
        assert_eq!(slot_from_row(3), Some(HotkeySlot::ClipboardWatch));
        assert_eq!(slot_from_row(4), None);
    }

    #[test]
    fn reset_hotkeys_restores_defaults_only_when_needed() {
        let defaults = HotkeyConfig::default();
        let mut hotkeys = defaults.clone();
        hotkeys.toggle_window = HotkeySpec::new(false, false, false, false, VK_F8.0 as u32);

        assert!(reset_hotkeys(&mut hotkeys));
        assert_eq!(hotkeys, defaults);
        assert!(!reset_hotkeys(&mut hotkeys));
    }
}
