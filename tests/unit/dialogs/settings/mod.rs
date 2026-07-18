use super::mask_secret;

#[test]
fn secret_mask_never_contains_the_complete_secret() {
    let masked = mask_secret("super-secret-1234");
    assert_eq!(masked, "••••1234");
    assert!(!masked.contains("super-secret"));
    assert_eq!(mask_secret("abc"), "••••");
}

#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_engine_transition_and_invalid_numeric_input_smoke() {
    use super::{SettingsDialog, ctrl_id};
    use crate::config::Config;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CB_GETCURSEL, CB_SETCURSEL, DestroyWindow, GWL_STYLE, GetDesktopWindow, GetDlgItem,
        GetWindowLongPtrW, IsWindow, SendMessageW, SetWindowTextW, WM_COMMAND, WS_THICKFRAME,
    };
    use windows::core::w;

    struct DialogGuard(Option<HWND>);

    impl DialogGuard {
        fn close(&mut self) {
            if let Some(hwnd) = self.0.take() {
                unsafe {
                    DestroyWindow(hwnd).unwrap();
                }
            }
        }
    }

    impl Drop for DialogGuard {
        fn drop(&mut self) {
            if let Some(hwnd) = self.0.take()
                && unsafe { IsWindow(Some(hwnd)).as_bool() }
            {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
        }
    }

    fn control(dialog: HWND, id: u16) -> HWND {
        unsafe { GetDlgItem(Some(dialog), id as i32).unwrap() }
    }

    fn combo_index(dialog: HWND, id: u16) -> usize {
        unsafe { SendMessageW(control(dialog, id), CB_GETCURSEL, None, None).0 as usize }
    }

    fn select_combo(dialog: HWND, id: u16, index: usize) {
        let combo = control(dialog, id);
        unsafe {
            let _ = SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(index)), None);
            let command = usize::from(id) | (1usize << 16); // CBN_SELCHANGE
            let _ = SendMessageW(
                dialog,
                WM_COMMAND,
                Some(WPARAM(command)),
                Some(LPARAM(combo.0 as isize)),
            );
        }
    }

    fn send_killfocus(dialog: HWND, id: u16) {
        let edit = control(dialog, id);
        unsafe {
            let command = usize::from(id) | (0x0200usize << 16); // EN_KILLFOCUS
            let _ = SendMessageW(
                dialog,
                WM_COMMAND,
                Some(WPARAM(command)),
                Some(LPARAM(edit.0 as isize)),
            );
        }
    }

    let mut initial = Config::default();
    initial.translation.engine = "deepl".into();
    initial.translation.source_lang = "en".into();
    initial.translation.target_lang = "fr".into();
    let config = initial;
    let parent = unsafe { GetDesktopWindow() };
    let hwnd = SettingsDialog::show(parent, config.clone(), None).unwrap();
    let mut dialog = DialogGuard(Some(hwnd));
    assert_ne!(
        unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 } & WS_THICKFRAME.0,
        0,
        "settings dialog must expose the standard resize border"
    );

    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_SOURCE_LANG), 2);
    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_TARGET_LANG), 5);
    select_combo(hwnd, ctrl_id::TRANS_ENGINE, 0);

    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_SOURCE_LANG), 0);
    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_TARGET_LANG), 0);
    let max_tokens = config.translation.llm.max_tokens;
    unsafe {
        SetWindowTextW(control(hwnd, ctrl_id::LLM_MAX_TOKENS_EDIT), w!("invalid")).unwrap();
    }
    send_killfocus(hwnd, ctrl_id::LLM_MAX_TOKENS_EDIT);
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_MAX_TOKENS_EDIT)),
        max_tokens.to_string()
    );
    dialog.close();
}
