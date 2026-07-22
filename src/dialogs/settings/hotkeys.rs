//! 단축키 탭: 각 단축키의 Ctrl/Shift/Alt/Win 체크박스 + 키 콤보박스 처리.
//!
//! (B) 방식(체크박스+콤보박스) 채택: 모달 키 캡처 다이얼로그(A)보다 구현이 단순하고,
//! 리소스 DIALOGEX 기반의 탭 컨트롤 패턴과 자연스럽게 어울리며, 값 검증/충돌 검사를
//! 다른 설정 항목과 동일한 방식(EN_KILLFOCUS/CBN_SELCHANGE 없이 즉시 반영)으로 처리할 수 있다.

use windows::{
    Win32::{Foundation::*, UI::Controls::*, UI::WindowsAndMessaging::*},
    core::*,
};

use super::{SettingsDialog, ctrl_id};
use crate::config::hotkey::{key_index_from_vk, vk_from_key_index};
use crate::config::{HotkeySlot, HotkeySpec};

/// 단축키 한 항목을 구성하는 컨트롤 ID 묶음.
#[derive(Clone, Copy, Debug)]
pub(super) struct HotkeyRowIds {
    pub ctrl: u16,
    pub shift: u16,
    pub alt: u16,
    pub win: u16,
    pub key_combo: u16,
    pub preview: u16,
}

const TOGGLE_WINDOW_IDS: HotkeyRowIds = HotkeyRowIds {
    ctrl: ctrl_id::HOTKEY_TOGGLE_WINDOW_CTRL,
    shift: ctrl_id::HOTKEY_TOGGLE_WINDOW_SHIFT,
    alt: ctrl_id::HOTKEY_TOGGLE_WINDOW_ALT,
    win: ctrl_id::HOTKEY_TOGGLE_WINDOW_WIN,
    key_combo: ctrl_id::HOTKEY_TOGGLE_WINDOW_KEY,
    preview: ctrl_id::HOTKEY_TOGGLE_WINDOW_PREVIEW,
};

const TEXT_SIZE_UP_IDS: HotkeyRowIds = HotkeyRowIds {
    ctrl: ctrl_id::HOTKEY_TEXT_SIZE_UP_CTRL,
    shift: ctrl_id::HOTKEY_TEXT_SIZE_UP_SHIFT,
    alt: ctrl_id::HOTKEY_TEXT_SIZE_UP_ALT,
    win: ctrl_id::HOTKEY_TEXT_SIZE_UP_WIN,
    key_combo: ctrl_id::HOTKEY_TEXT_SIZE_UP_KEY,
    preview: ctrl_id::HOTKEY_TEXT_SIZE_UP_PREVIEW,
};

const TEXT_SIZE_DOWN_IDS: HotkeyRowIds = HotkeyRowIds {
    ctrl: ctrl_id::HOTKEY_TEXT_SIZE_DOWN_CTRL,
    shift: ctrl_id::HOTKEY_TEXT_SIZE_DOWN_SHIFT,
    alt: ctrl_id::HOTKEY_TEXT_SIZE_DOWN_ALT,
    win: ctrl_id::HOTKEY_TEXT_SIZE_DOWN_WIN,
    key_combo: ctrl_id::HOTKEY_TEXT_SIZE_DOWN_KEY,
    preview: ctrl_id::HOTKEY_TEXT_SIZE_DOWN_PREVIEW,
};

const CLIPBOARD_WATCH_IDS: HotkeyRowIds = HotkeyRowIds {
    ctrl: ctrl_id::HOTKEY_CLIPBOARD_WATCH_CTRL,
    shift: ctrl_id::HOTKEY_CLIPBOARD_WATCH_SHIFT,
    alt: ctrl_id::HOTKEY_CLIPBOARD_WATCH_ALT,
    win: ctrl_id::HOTKEY_CLIPBOARD_WATCH_WIN,
    key_combo: ctrl_id::HOTKEY_CLIPBOARD_WATCH_KEY,
    preview: ctrl_id::HOTKEY_CLIPBOARD_WATCH_PREVIEW,
};

/// 단축키 탭의 체크박스/콤보 컨트롤 ID가 `cmd`와 일치하면 해당 슬롯을 반환한다.
fn slot_for_control(cmd: u16) -> Option<HotkeySlot> {
    for (slot, ids) in [
        (HotkeySlot::ToggleWindow, TOGGLE_WINDOW_IDS),
        (HotkeySlot::TextSizeUp, TEXT_SIZE_UP_IDS),
        (HotkeySlot::TextSizeDown, TEXT_SIZE_DOWN_IDS),
        (HotkeySlot::ClipboardWatch, CLIPBOARD_WATCH_IDS),
    ] {
        if [ids.ctrl, ids.shift, ids.alt, ids.win, ids.key_combo].contains(&cmd) {
            return Some(slot);
        }
    }
    None
}

impl SettingsDialog {
    /// 단축키 탭: 콤보박스에 키 목록을 채우고 체크박스/콤보 초기 선택값을 config에서 가져온다.
    pub(super) fn initialize_hotkey_combos(&self) -> Result<()> {
        use crate::config::hotkey::key_names;
        let key_items: Vec<&str> = key_names().collect();

        for (_, spec, ids) in self.hotkey_rows() {
            self.initialize_combo(ids.key_combo, &key_items, 0)?;
            self.set_hotkey_row_controls(ids, spec)?;
        }
        self.refresh_hotkey_previews();
        Ok(())
    }

    /// (slot, 현재 spec, 컨트롤 ID 묶음) 4개를 고정 순서로 순회한다.
    pub(super) fn hotkey_rows(&self) -> [(HotkeySlot, HotkeySpec, HotkeyRowIds); 4] {
        let hotkeys = self.draft.borrow().hotkeys.clone();
        [
            (
                HotkeySlot::ToggleWindow,
                hotkeys.get(HotkeySlot::ToggleWindow),
                TOGGLE_WINDOW_IDS,
            ),
            (
                HotkeySlot::TextSizeUp,
                hotkeys.get(HotkeySlot::TextSizeUp),
                TEXT_SIZE_UP_IDS,
            ),
            (
                HotkeySlot::TextSizeDown,
                hotkeys.get(HotkeySlot::TextSizeDown),
                TEXT_SIZE_DOWN_IDS,
            ),
            (
                HotkeySlot::ClipboardWatch,
                hotkeys.get(HotkeySlot::ClipboardWatch),
                CLIPBOARD_WATCH_IDS,
            ),
        ]
    }

    /// 지정한 행의 체크박스/콤보를 `spec` 값으로 맞춘다 (초기화 및 복원에 사용).
    pub(super) fn set_hotkey_row_controls(&self, ids: HotkeyRowIds, spec: HotkeySpec) -> Result<()> {
        self.set_checked(ids.ctrl, spec.ctrl)?;
        self.set_checked(ids.shift, spec.shift)?;
        self.set_checked(ids.alt, spec.alt)?;
        self.set_checked(ids.win, spec.win)?;
        if let Some(index) = key_index_from_vk(spec.vk) {
            let combo = self.control(ids.key_combo)?;
            unsafe {
                let _ = SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(index)), Some(LPARAM(0)));
            }
        }
        Ok(())
    }

    fn checked_by_id(&self, id: u16) -> Result<bool> {
        let control = self.control(id)?;
        // SAFETY: control is a valid checkbox HWND from GetDlgItem.
        let state = unsafe { SendMessageW(control, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))) };
        Ok(state.0 as u32 == BST_CHECKED.0)
    }

    /// 현재 UI 상태(체크박스+콤보)로부터 `HotkeySpec`을 읽어온다.
    fn read_hotkey_row(&self, ids: HotkeyRowIds) -> Result<HotkeySpec> {
        let combo = self.control(ids.key_combo)?;
        // SAFETY: combo is a valid combobox HWND from GetDlgItem.
        let sel = unsafe { SendMessageW(combo, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))) }.0
            as usize;
        let vk = vk_from_key_index(sel).unwrap_or(0);
        Ok(HotkeySpec::new(
            self.checked_by_id(ids.ctrl)?,
            self.checked_by_id(ids.shift)?,
            self.checked_by_id(ids.alt)?,
            self.checked_by_id(ids.win)?,
            vk,
        ))
    }

    /// 각 행의 미리보기 라벨("Ctrl+Shift+A")을 현재 draft 값으로 갱신한다.
    pub(super) fn refresh_hotkey_previews(&self) {
        for (_, spec, ids) in self.hotkey_rows() {
            let text = if spec.is_valid() {
                spec.to_string()
            } else {
                "(설정되지 않음)".to_string()
            };
            self.set_control_text(ids.preview, &text);
        }
    }

    /// 단축키 탭의 체크박스/콤보 명령을 처리한다. 자신의 컨트롤이 아니거나 무시해도 되는
    /// 알림(예: 콤보의 CBN_DROPDOWN)이면 `false`를 반환해 다른 핸들러가 계속 처리하게 한다.
    pub(super) fn handle_hotkey_command(&mut self, cmd: u16, notify_code: u32) -> bool {
        let Some(slot) = slot_for_control(cmd) else {
            return false;
        };
        // 체크박스는 BN_CLICKED(0), 키 콤보는 CBN_SELCHANGE만 값 변경으로 취급한다.
        if notify_code != BN_CLICKED && notify_code != CBN_SELCHANGE {
            return true;
        }
        let ids = self
            .hotkey_rows()
            .into_iter()
            .find(|(row_slot, _, _)| *row_slot == slot)
            .map(|(_, _, ids)| ids)
            .expect("hotkey_rows covers every HotkeySlot");

        let new_spec = match self.read_hotkey_row(ids) {
            Ok(spec) => spec,
            Err(error) => {
                tracing::warn!("단축키 컨트롤을 읽을 수 없습니다: {error}");
                return true;
            }
        };

        if !new_spec.is_valid() {
            // 수정자가 하나도 없는 조합은 시스템 전역 단축키로 부적절하다.
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "단축키 오류",
                "단축키는 Ctrl/Shift/Alt/Win 중 하나 이상을 포함해야 합니다.",
            );
            self.restore_hotkey_row(slot);
            return true;
        }

        let mut hotkeys = self.draft.borrow().hotkeys.clone();
        hotkeys.set(slot, new_spec);
        if let Some((a, b)) = hotkeys.find_conflict() {
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "단축키 충돌",
                &format!(
                    "'{}'와(과) '{}'에 같은 단축키({new_spec})가 배정되었습니다.\n다른 조합을 선택해주세요.",
                    a.label(),
                    b.label()
                ),
            );
            self.restore_hotkey_row(slot);
            return true;
        }

        self.draft.borrow_mut().hotkeys = hotkeys;
        self.has_unapplied_changes.set(true);
        self.refresh_hotkey_previews();
        true
    }

    /// 충돌/검증 실패 시 UI를 draft에 저장된 마지막 유효값으로 되돌린다.
    fn restore_hotkey_row(&self, slot: HotkeySlot) {
        let spec = self.draft.borrow().hotkeys.get(slot);
        if let Some((_, _, ids)) = self
            .hotkey_rows()
            .into_iter()
            .find(|(row_slot, _, _)| *row_slot == slot)
        {
            let _ = self.set_hotkey_row_controls(ids, spec);
        }
        // 체크박스/콤보만 되돌리고 끝내면 직전 실패한 값의 미리보기 텍스트가 남는다.
        self.refresh_hotkey_previews();
    }
}
