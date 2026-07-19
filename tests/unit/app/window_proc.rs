use super::{ReentryPolicy, reentry_policy};
use crate::constants::{
    WM_APP_MAGNETIC_TARGET_SELECTED, WM_APP_REFRESH, WM_APP_SET_MAGNETIC, WM_TRANSLATION_COMPLETE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    WM_APP, WM_CLIPBOARDUPDATE, WM_COMMAND, WM_DPICHANGED, WM_NOTIFY, WM_SIZE,
};

#[test]
fn reentry_defers_owned_app_messages() {
    for msg in [
        WM_COMMAND,
        WM_SIZE,
        WM_CLIPBOARDUPDATE,
        WM_APP_REFRESH,
        WM_APP_SET_MAGNETIC,
        WM_APP_MAGNETIC_TARGET_SELECTED,
        WM_TRANSLATION_COMPLETE,
    ] {
        assert_eq!(reentry_policy(msg, 0), ReentryPolicy::DeferOwned);
    }
}

#[test]
fn reentry_copies_dpi_effect_but_never_queues_its_rect_pointer() {
    assert_eq!(
        reentry_policy(WM_DPICHANGED, 0),
        ReentryPolicy::ApplyDpiThenResize
    );
    assert_eq!(reentry_policy(WM_NOTIFY, 0), ReentryPolicy::Default);
}

#[test]
fn taskbar_restart_message_is_deferred_dynamically() {
    let taskbar_message = WM_APP + 99;
    assert_eq!(
        reentry_policy(taskbar_message, taskbar_message),
        ReentryPolicy::DeferOwned
    );
}
