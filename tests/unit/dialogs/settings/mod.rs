use super::{EngineGroup, SettingsDialog, format_deepl_key, mask_secret};
use crate::translation::TranslationEngine;

#[test]
fn secret_mask_never_contains_the_complete_secret() {
    let masked = mask_secret("super-secret-1234");
    assert_eq!(masked, "••••1234");
    assert!(!masked.contains("super-secret"));
    assert_eq!(mask_secret("abc"), "••••");
}

#[test]
fn deepl_key_labels_show_the_detected_api_tier() {
    assert_eq!(format_deepl_key("free-secret:fx"), "[무료] ••••t:fx");
    assert_eq!(format_deepl_key("pro-secret"), "[유료] ••••cret");
}

#[test]
fn translation_panel_and_height_follow_the_selected_engine() {
    assert_eq!(
        SettingsDialog::engine_group(TranslationEngine::DeepL),
        Some(EngineGroup::DeepL)
    );
    assert_eq!(
        SettingsDialog::engine_group(TranslationEngine::Papago),
        Some(EngineGroup::Papago)
    );
    assert_eq!(
        SettingsDialog::engine_group(TranslationEngine::Custom),
        Some(EngineGroup::Custom)
    );
    assert_eq!(
        SettingsDialog::engine_group(TranslationEngine::Google),
        None
    );

    assert!(
        SettingsDialog::translation_height_for_engine(TranslationEngine::Papago)
            < SettingsDialog::translation_height_for_engine(TranslationEngine::DeepL)
    );
    assert_eq!(
        SettingsDialog::translation_height_for_engine(TranslationEngine::Papago),
        SettingsDialog::translation_height_for_engine(TranslationEngine::EzTrans)
    );
    assert!(
        SettingsDialog::translation_height_for_engine(TranslationEngine::DeepL)
            < SettingsDialog::translation_height_for_engine(TranslationEngine::Llm)
    );
    assert!(
        SettingsDialog::translation_group_height_for_engine(TranslationEngine::Papago)
            < SettingsDialog::translation_group_height_for_engine(TranslationEngine::DeepL)
    );
    assert!(
        SettingsDialog::translation_group_height_for_engine(TranslationEngine::DeepL)
            < SettingsDialog::translation_group_height_for_engine(TranslationEngine::Llm)
    );
    assert_eq!(
        SettingsDialog::translation_height_for_engine(TranslationEngine::Custom),
        SettingsDialog::translation_height_for_engine(TranslationEngine::Papago)
    );
}

#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_engine_transition_and_invalid_numeric_input_smoke() {
    use super::ctrl_id;
    use crate::config::Config;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CB_GETCURSEL, CB_SETCURSEL, DestroyWindow, GWL_STYLE, GetDesktopWindow, GetDlgItem,
        GetWindowLongPtrW, GetWindowRect, IsWindow, IsWindowVisible, SendMessageW, SetWindowTextW,
        WM_COMMAND, WS_THICKFRAME,
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
    let apply = control(hwnd, ctrl_id::APPLY);
    let mut dialog_rect = Default::default();
    let mut apply_rect = Default::default();
    unsafe {
        GetWindowRect(hwnd, &mut dialog_rect).unwrap();
        GetWindowRect(apply, &mut apply_rect).unwrap();
    }
    assert!(unsafe { IsWindowVisible(apply).as_bool() });
    assert!(apply_rect.left >= dialog_rect.left);
    assert!(apply_rect.top >= dialog_rect.top);
    assert!(apply_rect.right <= dialog_rect.right);
    assert!(apply_rect.bottom <= dialog_rect.bottom);
    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_SOURCE_LANG), 2);
    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_TARGET_LANG), 5);

    super::SETTINGS_INSTANCE.with(|slot| {
        let instance = slot.borrow().as_ref().expect("settings instance").clone();
        instance.borrow_mut().switch_tab(super::TAB_TRANSLATION);
    });
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::DEEPL_KEYS_LIST)).as_bool() });
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::DEEPL_KEY_TIER_COMBO)).as_bool() });
    assert_eq!(combo_index(hwnd, ctrl_id::DEEPL_KEY_TIER_COMBO), 0);
    assert!(!unsafe { IsWindowVisible(control(hwnd, ctrl_id::PAPAGO_ID_EDIT)).as_bool() });
    assert!(!unsafe { IsWindowVisible(control(hwnd, ctrl_id::LLM_API_KEY_EDIT)).as_bool() });
    let mut deepl_rect = Default::default();
    let mut deepl_group_rect = Default::default();
    let mut deepl_keys_rect = Default::default();
    let mut deepl_key_input_rect = Default::default();
    let mut deepl_last_control_rect = Default::default();
    unsafe {
        GetWindowRect(hwnd, &mut deepl_rect).unwrap();
        GetWindowRect(
            control(hwnd, ctrl_id::TRANSLATION_GROUP),
            &mut deepl_group_rect,
        )
        .unwrap();
        GetWindowRect(
            control(hwnd, ctrl_id::DEEPL_KEYS_LIST),
            &mut deepl_keys_rect,
        )
        .unwrap();
        GetWindowRect(
            control(hwnd, ctrl_id::DEEPL_KEY_ADD_EDIT),
            &mut deepl_key_input_rect,
        )
        .unwrap();
        GetWindowRect(
            control(hwnd, ctrl_id::DEEPL_KEY_REMOVE_BTN),
            &mut deepl_last_control_rect,
        )
        .unwrap();
    }
    assert!(deepl_key_input_rect.top > deepl_keys_rect.bottom);
    assert!(deepl_last_control_rect.bottom < deepl_group_rect.bottom);

    select_combo(
        hwnd,
        ctrl_id::TRANS_ENGINE,
        TranslationEngine::Papago as usize,
    );
    assert!(!unsafe { IsWindowVisible(control(hwnd, ctrl_id::DEEPL_KEYS_LIST)).as_bool() });
    assert!(!unsafe { IsWindowVisible(control(hwnd, ctrl_id::DEEPL_KEY_TIER_COMBO)).as_bool() });
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::PAPAGO_ID_EDIT)).as_bool() });
    let mut papago_rect = Default::default();
    let mut papago_group_rect = Default::default();
    unsafe {
        GetWindowRect(hwnd, &mut papago_rect).unwrap();
        GetWindowRect(
            control(hwnd, ctrl_id::TRANSLATION_GROUP),
            &mut papago_group_rect,
        )
        .unwrap();
    }
    assert!(papago_rect.bottom - papago_rect.top < deepl_rect.bottom - deepl_rect.top);
    assert!(
        papago_group_rect.bottom - papago_group_rect.top
            < deepl_group_rect.bottom - deepl_group_rect.top
    );

    select_combo(
        hwnd,
        ctrl_id::TRANS_ENGINE,
        TranslationEngine::Custom as usize,
    );
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::CUSTOM_API_SELECT)).as_bool() });
    assert!(!unsafe { IsWindowVisible(control(hwnd, ctrl_id::PAPAGO_ID_EDIT)).as_bool() });
    let mut custom_group_rect = Default::default();
    let mut custom_last_control_rect = Default::default();
    unsafe {
        GetWindowRect(
            control(hwnd, ctrl_id::TRANSLATION_GROUP),
            &mut custom_group_rect,
        )
        .unwrap();
        GetWindowRect(control(hwnd, 2242), &mut custom_last_control_rect).unwrap();
    }
    assert_eq!(
        custom_group_rect.bottom - custom_group_rect.top,
        papago_group_rect.bottom - papago_group_rect.top
    );
    assert!(custom_last_control_rect.bottom < custom_group_rect.bottom);

    select_combo(hwnd, ctrl_id::TRANS_ENGINE, TranslationEngine::Llm as usize);
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::LLM_API_KEY_EDIT)).as_bool() });
    let mut llm_group_rect = Default::default();
    let mut llm_last_control_rect = Default::default();
    unsafe {
        GetWindowRect(
            control(hwnd, ctrl_id::TRANSLATION_GROUP),
            &mut llm_group_rect,
        )
        .unwrap();
        GetWindowRect(control(hwnd, 2239), &mut llm_last_control_rect).unwrap();
    }
    assert!(
        llm_group_rect.bottom - llm_group_rect.top > deepl_group_rect.bottom - deepl_group_rect.top
    );
    assert!(llm_last_control_rect.bottom < llm_group_rect.bottom);

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
