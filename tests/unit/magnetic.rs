use super::*;
use windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_TOPMOST;

#[test]
fn target_style_rejects_shell_and_nonactivating_windows() {
    assert!(is_targetable_extended_style(0));
    assert!(!is_targetable_extended_style(WS_EX_TOOLWINDOW));
    assert!(!is_targetable_extended_style(WS_EX_NOACTIVATE));
    assert!(!is_targetable_extended_style(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW
    ));
}
