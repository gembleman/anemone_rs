use super::*;

#[test]
fn hook_session_grays_out_clipboard_watch_without_clearing_its_check() {
    // 후킹 중에는 감시가 자동으로 멈추므로 항목을 눌러도 달라지는 게 없다.
    // 다만 체크 표시는 저장된 설정을 계속 보여 줘야 한다 — detach하면 그 설정으로
    // 돌아가기 때문이다.
    let flags = clipboard_watch_flag(true, true);
    assert_ne!(flags & MF_GRAYED, 0);
    assert_ne!(flags & MF_CHECKED, 0);

    let flags = clipboard_watch_flag(false, true);
    assert_ne!(flags & MF_GRAYED, 0);
    assert_eq!(flags & MF_CHECKED, 0);
}

#[test]
fn clipboard_watch_stays_clickable_without_a_hook_session() {
    assert_eq!(clipboard_watch_flag(true, false), checked_flag(true));
    assert_eq!(clipboard_watch_flag(false, false), checked_flag(false));
    assert_eq!(clipboard_watch_flag(true, false) & MF_GRAYED, 0);
}

#[test]
fn popup_returns_commands_without_notifying_owner() {
    let flags = popup_flags();
    assert_ne!(flags & TPM_RETURNCMD, 0);
    assert_ne!(flags & TPM_NONOTIFY, 0);
}
