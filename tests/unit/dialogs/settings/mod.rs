use super::{EngineGroup, SettingsDialog, format_deepl_key, mask_secret};
use crate::translation::TranslationEngine;

#[test]
fn trackbar_thumb_notifications_apply_the_reported_position() {
    use windows::Win32::UI::Controls::{TB_ENDTRACK, TB_THUMBPOSITION, TB_THUMBTRACK};

    let wparam = 173usize << 16;
    assert_eq!(
        crate::dialogs::trackbar_thumb_position(TB_THUMBTRACK, wparam),
        Some(173)
    );
    assert_eq!(
        crate::dialogs::trackbar_thumb_position(TB_THUMBPOSITION, wparam),
        Some(173)
    );
    assert_eq!(
        crate::dialogs::trackbar_thumb_position(TB_ENDTRACK, wparam),
        None
    );
}

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
fn settings_tabs_include_information_tab() {
    assert_eq!(super::TAB_INFO, 4);
    assert_eq!(super::TAB_COUNT, 5);
    assert_eq!(super::APP_VERSION, env!("CARGO_PKG_VERSION"));
    assert!(!super::APP_VERSION.is_empty());
}

/// `settings.rc`의 컨트롤 정의에서 나타나는 모든 10진수를 모은다.
///
/// 리소스 문법을 완전히 해석하지 않는다. `EDITTEXT 1004, ...`처럼 ID가 첫
/// 인자인 형태와 `LTEXT "...", 2402, ...`처럼 문자열 뒤에 오는 형태가 섞여
/// 있으므로, 콤마와 공백 양쪽으로 쪼개 숫자만 취한다. 좌표값까지 섞이지만
/// 이 집합은 "선언된 ID의 상위 집합"이면 충분하다 — 목적이 **누락 검출**이라
/// 상위 집합이어도 거짓 통과만 없으면 된다.
fn declared_control_ids(rc: &str) -> std::collections::HashSet<u16> {
    let mut ids = std::collections::HashSet::new();
    for line in rc.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        for token in line.split([',', ' ', '\t']) {
            if let Ok(value) = token.trim().parse::<u16>() {
                ids.insert(value);
            }
        }
    }
    ids
}

/// 컨트롤 ID 상수와 `settings.rc`가 어긋나면 `GetDlgItem`이 실패하고
/// `register_ids`가 그 오류를 전파해 **설정 창 전체가 열리지 않는다.**
///
/// Win32 데스크톱이 필요한 스모크 테스트는 모두 `#[ignore]`라 CI에서 돌지
/// 않으므로, 리소스 텍스트를 직접 대조해 같은 사고를 막는다.
#[test]
fn every_registered_control_id_exists_in_the_resource_script() {
    use super::ctrl_id;

    const SETTINGS_RC: &str = include_str!("../../../../resources/settings.rc");
    let declared = declared_control_ids(SETTINGS_RC);

    let groups: &[(&str, &[u16])] = &[
        ("APPEARANCE_STATIC_IDS", ctrl_id::APPEARANCE_STATIC_IDS),
        ("DISPLAY_STATIC_IDS", ctrl_id::DISPLAY_STATIC_IDS),
        ("TRANSLATION_STATIC_IDS", ctrl_id::TRANSLATION_STATIC_IDS),
        ("HOTKEYS_STATIC_IDS", ctrl_id::HOTKEYS_STATIC_IDS),
        ("INFO_STATIC_IDS", ctrl_id::INFO_STATIC_IDS),
        ("EZTRANS_STATIC_IDS", ctrl_id::EZTRANS_STATIC_IDS),
        ("DEEPL_STATIC_IDS", ctrl_id::DEEPL_STATIC_IDS),
        ("PAPAGO_STATIC_IDS", ctrl_id::PAPAGO_STATIC_IDS),
        ("LLM_STATIC_IDS", ctrl_id::LLM_STATIC_IDS),
        ("CUSTOM_STATIC_IDS", ctrl_id::CUSTOM_STATIC_IDS),
        ("APPEARANCE_IDS", super::init::APPEARANCE_IDS),
        ("DISPLAY_IDS", super::init::DISPLAY_IDS),
        ("TRANSLATION_IDS", super::init::TRANSLATION_IDS),
        ("HOTKEYS_IDS", super::init::HOTKEYS_IDS),
        ("INFO_IDS", super::init::INFO_IDS),
    ];

    let mut missing = Vec::new();
    for (name, ids) in groups {
        for &id in *ids {
            if !declared.contains(&id) {
                missing.push(format!("{name}: {id}"));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "settings.rc에 없는 컨트롤 ID가 등록되어 설정 창이 열리지 않습니다: {missing:?}"
    );
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
    assert!(
        SettingsDialog::translation_height_for_engine(TranslationEngine::Papago)
            < SettingsDialog::translation_height_for_engine(TranslationEngine::EzTrans)
    );
    assert!(
        SettingsDialog::translation_height_for_engine(TranslationEngine::DeepL)
            < SettingsDialog::translation_height_for_engine(TranslationEngine::Llm)
    );
    assert_eq!(
        SettingsDialog::translation_height_for_engine(TranslationEngine::DeepL),
        365
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
        SettingsDialog::translation_group_height_for_engine(TranslationEngine::DeepL),
        136
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
    use windows::Win32::UI::Controls::{BST_UNCHECKED, EM_GETPASSWORDCHAR};
    use windows::Win32::UI::WindowsAndMessaging::{
        BM_CLICK, BM_GETCHECK, CB_GETCURSEL, CB_SETCURSEL, DestroyWindow, GWL_STYLE,
        GetDesktopWindow, GetDlgItem, GetWindowLongPtrW, GetWindowRect, IsWindow, IsWindowVisible,
        SendMessageW, SetWindowTextW, WM_COMMAND, WS_THICKFRAME,
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
    initial.translation.eztrans_postprocess_dictionary =
        vec![crate::config::EzTransPostprocessEntry {
            source: "번역 결과".into(),
            target: "후처리 결과".into(),
        }];
    initial.translation.llm.api_key = "openai-key".into();
    initial.translation.llm.temperature = 0.21;
    initial
        .translation
        .llm
        .set_provider(crate::translation::LlmProvider::Anthropic);
    initial.translation.llm.model = "anthropic-model".into();
    initial.translation.llm.api_key = "anthropic-key".into();
    initial.translation.llm.system_prompt = "anthropic prompt".into();
    initial.translation.llm.temperature = 0.72;
    initial.translation.llm.max_tokens = 4_002;
    initial.translation.llm.reasoning_effort = Some(crate::translation::llm::ReasoningEffort::Low);
    initial.translation.llm.debounce_ms = 402;
    initial.translation.llm.glossary = vec![crate::config::LlmGlossaryEntry {
        source: "Claude".into(),
        target: "클로드".into(),
    }];
    initial
        .translation
        .llm
        .set_provider(crate::translation::LlmProvider::OpenAi);
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

    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::MARGIN_X_EDIT)),
        config.text_margin_x.to_string()
    );
    unsafe {
        SetWindowTextW(control(hwnd, ctrl_id::MARGIN_X_EDIT), w!("999")).unwrap();
    }
    send_killfocus(hwnd, ctrl_id::MARGIN_X_EDIT);
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::MARGIN_X_EDIT)),
        "300"
    );
    super::SETTINGS_INSTANCE.with(|slot| {
        let instance = slot.borrow().as_ref().expect("settings instance").clone();
        assert_eq!(instance.borrow().draft.borrow().text_margin_x, 300);
    });

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
        GetWindowRect(control(hwnd, 2216), &mut deepl_last_control_rect).unwrap();
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
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::LLM_API_KEY_VISIBLE)).as_bool() });
    assert_eq!(
        unsafe {
            SendMessageW(
                control(hwnd, ctrl_id::LLM_API_KEY_VISIBLE),
                BM_GETCHECK,
                None,
                None,
            )
            .0
        },
        BST_UNCHECKED.0 as isize
    );
    assert_ne!(
        unsafe {
            SendMessageW(
                control(hwnd, ctrl_id::LLM_API_KEY_EDIT),
                EM_GETPASSWORDCHAR,
                None,
                None,
            )
            .0
        },
        0
    );
    unsafe {
        let _ = SendMessageW(
            control(hwnd, ctrl_id::LLM_API_KEY_VISIBLE),
            BM_CLICK,
            None,
            None,
        );
    }
    assert_eq!(
        unsafe {
            SendMessageW(
                control(hwnd, ctrl_id::LLM_API_KEY_EDIT),
                EM_GETPASSWORDCHAR,
                None,
                None,
            )
            .0
        },
        0
    );
    unsafe {
        let _ = SendMessageW(
            control(hwnd, ctrl_id::LLM_API_KEY_VISIBLE),
            BM_CLICK,
            None,
            None,
        );
    }
    assert_ne!(
        unsafe {
            SendMessageW(
                control(hwnd, ctrl_id::LLM_API_KEY_EDIT),
                EM_GETPASSWORDCHAR,
                None,
                None,
            )
            .0
        },
        0
    );
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_MODEL_EDIT)),
        crate::translation::LlmProvider::OpenAi.default_model()
    );
    assert_eq!(combo_index(hwnd, ctrl_id::LLM_REASONING_EFFORT), 0);
    select_combo(hwnd, ctrl_id::LLM_REASONING_EFFORT, 5);
    super::SETTINGS_INSTANCE.with(|slot| {
        let instance = slot.borrow().as_ref().expect("settings instance").clone();
        assert_eq!(
            instance
                .borrow()
                .draft
                .borrow()
                .translation
                .llm
                .reasoning_effort,
            Some(crate::translation::llm::ReasoningEffort::High)
        );
    });

    select_combo(
        hwnd,
        ctrl_id::LLM_PROVIDER,
        crate::translation::LlmProvider::Anthropic as usize,
    );
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_MODEL_EDIT)),
        "anthropic-model"
    );
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_API_KEY_EDIT)),
        "anthropic-key"
    );
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_SYSTEM_PROMPT_EDIT)),
        "anthropic prompt"
    );
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_TEMPERATURE_EDIT)),
        "0.72"
    );
    assert_eq!(
        unsafe {
            SendMessageW(
                control(hwnd, ctrl_id::LLM_TEMPERATURE_TRACKBAR),
                crate::dialogs::TBM_GETPOS,
                None,
                None,
            )
            .0
        },
        72
    );
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_MAX_TOKENS_EDIT)),
        "4002"
    );
    assert_eq!(combo_index(hwnd, ctrl_id::LLM_REASONING_EFFORT), 3);
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_DEBOUNCE_EDIT)),
        "402"
    );
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_GLOSSARY_COUNT_LABEL)),
        "사전 항목: 1"
    );

    select_combo(
        hwnd,
        ctrl_id::LLM_PROVIDER,
        crate::translation::LlmProvider::OpenAi as usize,
    );
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_API_KEY_EDIT)),
        "openai-key"
    );
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_TEMPERATURE_EDIT)),
        "0.21"
    );
    assert_eq!(combo_index(hwnd, ctrl_id::LLM_REASONING_EFFORT), 5);
    let mut llm_group_rect = Default::default();
    let mut llm_last_control_rect = Default::default();
    unsafe {
        GetWindowRect(
            control(hwnd, ctrl_id::TRANSLATION_GROUP),
            &mut llm_group_rect,
        )
        .unwrap();
        GetWindowRect(
            control(hwnd, ctrl_id::LLM_GLOSSARY_EDIT_BTN),
            &mut llm_last_control_rect,
        )
        .unwrap();
    }
    assert!(
        llm_group_rect.bottom - llm_group_rect.top > deepl_group_rect.bottom - deepl_group_rect.top
    );
    assert!(llm_last_control_rect.bottom < llm_group_rect.bottom);

    select_combo(hwnd, ctrl_id::TRANS_ENGINE, 0);

    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_SOURCE_LANG), 0);
    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_TARGET_LANG), 0);
    assert!(unsafe {
        IsWindowVisible(control(hwnd, ctrl_id::EZTRANS_DICTIONARY_EDIT_BTN)).as_bool()
    });
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(
            hwnd,
            ctrl_id::EZTRANS_DICTIONARY_COUNT_LABEL
        )),
        "후처리 사전: 1"
    );
    let max_tokens = config.translation.llm.max_tokens;
    unsafe {
        SetWindowTextW(control(hwnd, ctrl_id::LLM_MAX_TOKENS_EDIT), w!("invalid")).unwrap();
    }
    send_killfocus(hwnd, ctrl_id::LLM_MAX_TOKENS_EDIT);
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_MAX_TOKENS_EDIT)),
        max_tokens.to_string()
    );
    unsafe {
        SetWindowTextW(control(hwnd, ctrl_id::LLM_TEMPERATURE_EDIT), w!("0.73")).unwrap();
    }
    send_killfocus(hwnd, ctrl_id::LLM_TEMPERATURE_EDIT);
    assert_eq!(
        unsafe {
            SendMessageW(
                control(hwnd, ctrl_id::LLM_TEMPERATURE_TRACKBAR),
                crate::dialogs::TBM_GETPOS,
                None,
                None,
            )
            .0
        },
        73
    );
    unsafe {
        SetWindowTextW(control(hwnd, ctrl_id::LLM_TEMPERATURE_EDIT), w!("invalid")).unwrap();
    }
    send_killfocus(hwnd, ctrl_id::LLM_TEMPERATURE_EDIT);
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_TEMPERATURE_EDIT)),
        "0.73"
    );
    dialog.close();
}

#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_display_tab_keeps_cache_controls_visible() {
    use super::ctrl_id;
    use crate::config::Config;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, GetDesktopWindow, GetDlgItem, GetWindowRect, IsWindow, IsWindowVisible,
    };

    struct DialogGuard(HWND);

    impl Drop for DialogGuard {
        fn drop(&mut self) {
            if unsafe { IsWindow(Some(self.0)).as_bool() } {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
    }

    fn control(dialog: HWND, id: u16) -> HWND {
        unsafe { GetDlgItem(Some(dialog), id as i32).unwrap() }
    }

    let hwnd =
        SettingsDialog::show(unsafe { GetDesktopWindow() }, Config::default(), None).unwrap();
    let _dialog = DialogGuard(hwnd);
    super::SETTINGS_INSTANCE.with(|slot| {
        let instance = slot.borrow().as_ref().expect("settings instance").clone();
        instance.borrow_mut().switch_tab(super::TAB_DISPLAY);
    });

    let cache_clear = control(hwnd, ctrl_id::CLIPBOARD_CACHE_CLEAR);
    let mut group_rect = Default::default();
    let mut cache_clear_rect = Default::default();
    let mut tab_rect = Default::default();
    let mut apply_rect = Default::default();
    unsafe {
        GetWindowRect(control(hwnd, 2101), &mut group_rect).unwrap();
        GetWindowRect(cache_clear, &mut cache_clear_rect).unwrap();
        GetWindowRect(control(hwnd, ctrl_id::TAB_CONTROL), &mut tab_rect).unwrap();
        GetWindowRect(control(hwnd, ctrl_id::APPLY), &mut apply_rect).unwrap();
    }

    assert!(unsafe { IsWindowVisible(cache_clear).as_bool() });
    assert!(cache_clear_rect.bottom < group_rect.bottom);
    assert!(cache_clear_rect.bottom < tab_rect.bottom);
    assert!(group_rect.bottom < apply_rect.top);
}

#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_hotkeys_tab_layout_smoke() {
    use super::ctrl_id;
    use crate::config::Config;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, GetDesktopWindow, GetDlgItem, GetWindowRect, IsWindow, IsWindowVisible,
        SendMessageW, WM_COMMAND,
    };

    struct DialogGuard(HWND);

    impl Drop for DialogGuard {
        fn drop(&mut self) {
            if unsafe { IsWindow(Some(self.0)).as_bool() } {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
    }

    fn control(dialog: HWND, id: u16) -> HWND {
        unsafe { GetDlgItem(Some(dialog), id as i32).unwrap() }
    }

    let mut config = Config::default();
    config.hotkeys.toggle_window = "F8".parse().unwrap();
    let hwnd = SettingsDialog::show(unsafe { GetDesktopWindow() }, config, None).unwrap();
    let _dialog = DialogGuard(hwnd);
    super::SETTINGS_INSTANCE.with(|slot| {
        let instance = slot.borrow().as_ref().expect("settings instance").clone();
        instance.borrow_mut().switch_tab(super::TAB_HOTKEYS);
    });

    let mut list_rect = Default::default();
    let mut reset_rect = Default::default();
    let mut tab_rect = Default::default();
    let mut apply_rect = Default::default();
    unsafe {
        GetWindowRect(control(hwnd, ctrl_id::HOTKEYS_LIST), &mut list_rect).unwrap();
        GetWindowRect(control(hwnd, ctrl_id::HOTKEYS_RESET), &mut reset_rect).unwrap();
        GetWindowRect(control(hwnd, ctrl_id::TAB_CONTROL), &mut tab_rect).unwrap();
        GetWindowRect(control(hwnd, ctrl_id::APPLY), &mut apply_rect).unwrap();
    }

    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::HOTKEYS_LIST)).as_bool() });
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::HOTKEYS_RESET)).as_bool() });
    assert!(list_rect.bottom <= tab_rect.bottom);
    assert!(reset_rect.top > list_rect.bottom);
    assert!(reset_rect.bottom <= tab_rect.bottom);
    assert!(list_rect.bottom < apply_rect.top);

    let reset = control(hwnd, ctrl_id::HOTKEYS_RESET);
    unsafe {
        let _ = SendMessageW(
            hwnd,
            WM_COMMAND,
            Some(WPARAM(usize::from(ctrl_id::HOTKEYS_RESET))),
            Some(LPARAM(reset.0 as isize)),
        );
    }
    super::SETTINGS_INSTANCE.with(|slot| {
        let instance = slot.borrow().as_ref().expect("settings instance").clone();
        let instance = instance.borrow();
        assert_eq!(instance.draft.borrow().hotkeys, Default::default());
        assert!(instance.has_unapplied_changes.get());
    });
}
