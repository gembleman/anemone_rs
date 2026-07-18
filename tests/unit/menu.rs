use super::*;

#[test]
fn popup_returns_commands_without_notifying_owner() {
    let flags = popup_flags().0;
    assert_ne!(flags & TPM_RETURNCMD.0, 0);
    assert_ne!(flags & TPM_NONOTIFY.0, 0);
}
