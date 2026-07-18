use super::{mask_secret, should_persist_trackbar};
use windows::Win32::UI::Controls::{TB_ENDTRACK, TB_LINEDOWN, TB_THUMBTRACK};

#[test]
fn secret_mask_never_contains_the_complete_secret() {
    let masked = mask_secret("super-secret-1234");
    assert_eq!(masked, "••••1234");
    assert!(!masked.contains("super-secret"));
    assert_eq!(mask_secret("abc"), "••••");
}

#[test]
fn persists_trackbar_only_when_tracking_ends() {
    assert!(!should_persist_trackbar(TB_THUMBTRACK));
    assert!(!should_persist_trackbar(TB_LINEDOWN));
    assert!(should_persist_trackbar(TB_ENDTRACK));
}

#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_engine_transition_and_invalid_numeric_input_smoke() {
    use super::{SettingsDialog, ctrl_id};
    use crate::config::Config;
    use crate::translation::TranslationJobSpec;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CB_GETCURSEL, CB_SETCURSEL, DestroyWindow, GetDesktopWindow, GetDlgItem, IsWindow,
        SendMessageW, SetWindowTextW, WM_COMMAND,
    };
    use windows::core::w;

    struct ConfigFileGuard {
        path: PathBuf,
        original: Option<Vec<u8>>,
    }

    impl ConfigFileGuard {
        fn new() -> Self {
            let path = Config::default_config_path().clone();
            assert_eq!(
                path.parent(),
                std::env::current_exe().unwrap().parent(),
                "smoke config must stay beside the test executable"
            );
            let original = std::fs::read(&path).ok();
            Self { path, original }
        }
    }

    impl Drop for ConfigFileGuard {
        fn drop(&mut self) {
            if let Some(original) = &self.original {
                std::fs::write(&self.path, original).unwrap();
            } else if let Err(error) = std::fs::remove_file(&self.path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                panic!("failed to remove smoke config: {error}");
            }
        }
    }

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

    let _config_file = ConfigFileGuard::new();
    let mut initial = Config::default();
    initial.translation.engine = "deepl".into();
    initial.translation.source_lang = "en".into();
    initial.translation.target_lang = "fr".into();
    let config = Rc::new(RefCell::new(initial));
    let parent = unsafe { GetDesktopWindow() };
    let hwnd = SettingsDialog::show(parent, config.clone(), None).unwrap();
    let mut dialog = DialogGuard(Some(hwnd));

    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_SOURCE_LANG), 2);
    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_TARGET_LANG), 5);
    select_combo(hwnd, ctrl_id::TRANS_ENGINE, 0);

    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_SOURCE_LANG), 0);
    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_TARGET_LANG), 0);
    assert_eq!(config.borrow().translation.source_lang, "ja");
    assert_eq!(config.borrow().translation.target_lang, "ko");
    assert!(TranslationJobSpec::from_config(&config.borrow().translation).is_ok());

    let max_tokens = config.borrow().translation.llm.max_tokens;
    let persisted_before_invalid = std::fs::read(Config::default_config_path()).unwrap();
    unsafe {
        SetWindowTextW(control(hwnd, ctrl_id::LLM_MAX_TOKENS_EDIT), w!("invalid")).unwrap();
    }
    send_killfocus(hwnd, ctrl_id::LLM_MAX_TOKENS_EDIT);
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_MAX_TOKENS_EDIT)),
        max_tokens.to_string()
    );
    assert_eq!(
        std::fs::read(Config::default_config_path()).unwrap(),
        persisted_before_invalid,
        "invalid input must not trigger another save"
    );

    dialog.close();
    let persisted = Config::load_from_file(Config::default_config_path()).unwrap();
    assert_eq!(persisted.translation.engine, "eztrans");
    assert_eq!(persisted.translation.source_lang, "ja");
    assert_eq!(persisted.translation.target_lang, "ko");

    let reopened = SettingsDialog::show(parent, config, None).unwrap();
    let mut reopened_dialog = DialogGuard(Some(reopened));
    assert_eq!(combo_index(reopened, ctrl_id::TRANS_ENGINE), 0);
    assert_eq!(combo_index(reopened, ctrl_id::TRANS_SOURCE_LANG), 0);
    assert_eq!(combo_index(reopened, ctrl_id::TRANS_TARGET_LANG), 0);
    reopened_dialog.close();
}
