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
        assert!(is_modifier_key(key as u32));
    }
    assert!(!is_modifier_key(VK_K as u32));
}

#[test]
fn main_key_keeps_all_pressed_modifiers() {
    let spec = hotkey_from_state(VK_K as u32, true, true, false, false);
    assert_eq!(spec.to_string(), "Ctrl+Shift+K");
    assert!(spec.ctrl);
    assert!(spec.shift);
    assert_eq!(spec.vk, VK_K as u32);
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
    hotkeys.toggle_window = HotkeySpec::new(false, false, false, false, VK_F8 as u32);

    assert!(reset_hotkeys(&mut hotkeys));
    assert_eq!(hotkeys, defaults);
    assert!(!reset_hotkeys(&mut hotkeys));
}
