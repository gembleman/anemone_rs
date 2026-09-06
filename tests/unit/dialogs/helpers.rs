use super::*;

unsafe fn unregister_during_pretranslation(hwnd: HWND, _msg: &MSG) -> bool {
    unregister_resource_dialog(hwnd);
    true
}

#[test]
fn dispatch_allows_a_dialog_to_unregister_during_pretranslation() {
    let hwnd = 1usize as HWND;
    register_resource_dialog(hwnd, unregister_during_pretranslation);

    let msg = MSG::default();
    assert!(unsafe { dispatch_resource_dialog_message(&msg) });
}
