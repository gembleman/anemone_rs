//! 설정 대화상자 엔진 전환 경로의 Win32 스모크 테스트.
//!
//! 실제 창을 띄워 엔진 콤보 전환과 숫자 입력 검증이 컨트롤 가시성·값에
//! 그대로 반영되는지 확인한다. Win32 데스크톱이 필요해 기본 실행에서는 제외된다.

use super::super::{SettingsDialog, TAB_TRANSLATION, ctrl_id, with_settings_instance};
use crate::translation::TranslationEngine;

#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_engine_transition_and_invalid_numeric_input_smoke() {
    use crate::config::Config;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::Controls::{BST_UNCHECKED, EM_GETPASSWORDCHAR};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BM_CLICK, BM_GETCHECK, CB_GETCURSEL, CB_SETCURSEL, DestroyWindow, GWL_STYLE,
        GetDesktopWindow, GetDlgItem, GetWindowLongPtrW, GetWindowRect, IsWindow, IsWindowVisible,
        SendMessageW, SetWindowTextW, WM_COMMAND, WS_THICKFRAME,
    };

    struct DialogGuard(Option<HWND>);

    impl DialogGuard {
        fn close(&mut self) {
            if let Some(hwnd) = self.0.take() {
                unsafe {
                    assert_ne!(DestroyWindow(hwnd), 0);
                }
            }
        }
    }

    impl Drop for DialogGuard {
        fn drop(&mut self) {
            if let Some(hwnd) = self.0.take()
                && unsafe { IsWindow(hwnd) != 0 }
            {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
        }
    }

    fn control(dialog: HWND, id: u16) -> HWND {
        let hwnd = unsafe { GetDlgItem(dialog, id as i32) };
        assert!(!hwnd.is_null(), "control {id} not found");
        hwnd
    }

    fn combo_index(dialog: HWND, id: u16) -> usize {
        unsafe { SendMessageW(control(dialog, id), CB_GETCURSEL, 0, 0) as usize }
    }

    fn select_combo(dialog: HWND, id: u16, index: usize) {
        let combo = control(dialog, id);
        unsafe {
            let _ = SendMessageW(combo, CB_SETCURSEL, index, 0);
            let command = usize::from(id) | (1usize << 16); // CBN_SELCHANGE
            let _ = SendMessageW(dialog, WM_COMMAND, command, combo as LPARAM);
        }
    }

    fn send_killfocus(dialog: HWND, id: u16) {
        let edit = control(dialog, id);
        unsafe {
            let command = usize::from(id) | (0x0200usize << 16); // EN_KILLFOCUS
            let _ = SendMessageW(dialog, WM_COMMAND, command, edit as LPARAM);
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
        unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 } & WS_THICKFRAME,
        0,
        "settings dialog must expose the standard resize border"
    );
    let apply = control(hwnd, ctrl_id::APPLY);
    let mut dialog_rect = Default::default();
    let mut apply_rect = Default::default();
    unsafe {
        assert_ne!(GetWindowRect(hwnd, &mut dialog_rect), 0);
        assert_ne!(GetWindowRect(apply, &mut apply_rect), 0);
    }
    assert!(unsafe { IsWindowVisible(apply) != 0 });
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
        let text = crate::win32::to_wide("999");
        assert_ne!(
            SetWindowTextW(control(hwnd, ctrl_id::MARGIN_X_EDIT), text.as_ptr()),
            0
        );
    }
    send_killfocus(hwnd, ctrl_id::MARGIN_X_EDIT);
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::MARGIN_X_EDIT)),
        "300"
    );
    with_settings_instance(|instance| {
        assert_eq!(instance.draft.borrow().text_margin_x, 300);
    });

    with_settings_instance(|instance| {
        instance.switch_tab(TAB_TRANSLATION);
    });
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::DEEPL_KEYS_LIST)) != 0 });
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::DEEPL_KEY_TIER_COMBO)) != 0 });
    assert_eq!(combo_index(hwnd, ctrl_id::DEEPL_KEY_TIER_COMBO), 0);
    assert!(!unsafe { IsWindowVisible(control(hwnd, ctrl_id::PAPAGO_ID_EDIT)) != 0 });
    assert!(!unsafe { IsWindowVisible(control(hwnd, ctrl_id::LLM_API_KEY_EDIT)) != 0 });
    let mut deepl_rect = Default::default();
    let mut deepl_group_rect = Default::default();
    let mut deepl_keys_rect = Default::default();
    let mut deepl_key_input_rect = Default::default();
    let mut deepl_last_control_rect = Default::default();
    unsafe {
        assert_ne!(GetWindowRect(hwnd, &mut deepl_rect), 0);
        assert_ne!(
            GetWindowRect(
                control(hwnd, ctrl_id::TRANSLATION_GROUP),
                &mut deepl_group_rect,
            ),
            0
        );
        assert_ne!(
            GetWindowRect(
                control(hwnd, ctrl_id::DEEPL_KEYS_LIST),
                &mut deepl_keys_rect,
            ),
            0
        );
        assert_ne!(
            GetWindowRect(
                control(hwnd, ctrl_id::DEEPL_KEY_ADD_EDIT),
                &mut deepl_key_input_rect,
            ),
            0
        );
        assert_ne!(
            GetWindowRect(control(hwnd, 2216), &mut deepl_last_control_rect),
            0
        );
    }
    assert!(deepl_key_input_rect.top > deepl_keys_rect.bottom);
    assert!(deepl_last_control_rect.bottom < deepl_group_rect.bottom);

    select_combo(
        hwnd,
        ctrl_id::TRANS_ENGINE,
        TranslationEngine::Papago as usize,
    );
    assert!(!unsafe { IsWindowVisible(control(hwnd, ctrl_id::DEEPL_KEYS_LIST)) != 0 });
    assert!(!unsafe { IsWindowVisible(control(hwnd, ctrl_id::DEEPL_KEY_TIER_COMBO)) != 0 });
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::PAPAGO_ID_EDIT)) != 0 });
    let mut papago_rect = Default::default();
    let mut papago_group_rect = Default::default();
    unsafe {
        assert_ne!(GetWindowRect(hwnd, &mut papago_rect), 0);
        assert_ne!(
            GetWindowRect(
                control(hwnd, ctrl_id::TRANSLATION_GROUP),
                &mut papago_group_rect,
            ),
            0
        );
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
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::CUSTOM_API_SELECT)) != 0 });
    assert!(!unsafe { IsWindowVisible(control(hwnd, ctrl_id::PAPAGO_ID_EDIT)) != 0 });
    let mut custom_group_rect = Default::default();
    let mut custom_last_control_rect = Default::default();
    unsafe {
        assert_ne!(
            GetWindowRect(
                control(hwnd, ctrl_id::TRANSLATION_GROUP),
                &mut custom_group_rect,
            ),
            0
        );
        assert_ne!(
            GetWindowRect(control(hwnd, 2242), &mut custom_last_control_rect),
            0
        );
    }
    assert_eq!(
        custom_group_rect.bottom - custom_group_rect.top,
        papago_group_rect.bottom - papago_group_rect.top
    );
    assert!(custom_last_control_rect.bottom < custom_group_rect.bottom);

    select_combo(hwnd, ctrl_id::TRANS_ENGINE, TranslationEngine::Llm as usize);
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::LLM_API_KEY_EDIT)) != 0 });
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::LLM_API_KEY_VISIBLE)) != 0 });
    assert_eq!(
        unsafe {
            SendMessageW(
                control(hwnd, ctrl_id::LLM_API_KEY_VISIBLE),
                BM_GETCHECK,
                0,
                0,
            )
        },
        BST_UNCHECKED as isize
    );
    assert_ne!(
        unsafe {
            SendMessageW(
                control(hwnd, ctrl_id::LLM_API_KEY_EDIT),
                EM_GETPASSWORDCHAR,
                0,
                0,
            )
        },
        0
    );
    unsafe {
        let _ = SendMessageW(control(hwnd, ctrl_id::LLM_API_KEY_VISIBLE), BM_CLICK, 0, 0);
    }
    assert_eq!(
        unsafe {
            SendMessageW(
                control(hwnd, ctrl_id::LLM_API_KEY_EDIT),
                EM_GETPASSWORDCHAR,
                0,
                0,
            )
        },
        0
    );
    unsafe {
        let _ = SendMessageW(control(hwnd, ctrl_id::LLM_API_KEY_VISIBLE), BM_CLICK, 0, 0);
    }
    assert_ne!(
        unsafe {
            SendMessageW(
                control(hwnd, ctrl_id::LLM_API_KEY_EDIT),
                EM_GETPASSWORDCHAR,
                0,
                0,
            )
        },
        0
    );
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_MODEL_EDIT)),
        crate::translation::LlmProvider::OpenAi.default_model()
    );
    assert_eq!(combo_index(hwnd, ctrl_id::LLM_REASONING_EFFORT), 0);
    select_combo(hwnd, ctrl_id::LLM_REASONING_EFFORT, 5);
    with_settings_instance(|instance| {
        assert_eq!(
            instance.draft.borrow().translation.llm.reasoning_effort,
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
                0,
                0,
            )
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
        assert_ne!(
            GetWindowRect(
                control(hwnd, ctrl_id::TRANSLATION_GROUP),
                &mut llm_group_rect,
            ),
            0
        );
        assert_ne!(
            GetWindowRect(
                control(hwnd, ctrl_id::LLM_GLOSSARY_EDIT_BTN),
                &mut llm_last_control_rect,
            ),
            0
        );
    }
    assert!(
        llm_group_rect.bottom - llm_group_rect.top > deepl_group_rect.bottom - deepl_group_rect.top
    );
    assert!(llm_last_control_rect.bottom < llm_group_rect.bottom);

    select_combo(hwnd, ctrl_id::TRANS_ENGINE, 0);

    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_SOURCE_LANG), 0);
    assert_eq!(combo_index(hwnd, ctrl_id::TRANS_TARGET_LANG), 0);
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::EZTRANS_DICTIONARY_EDIT_BTN)) != 0 });
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(
            hwnd,
            ctrl_id::EZTRANS_DICTIONARY_COUNT_LABEL
        )),
        "후처리 사전: 1"
    );
    let max_tokens = config.translation.llm.max_tokens;
    unsafe {
        let text = crate::win32::to_wide("invalid");
        assert_ne!(
            SetWindowTextW(control(hwnd, ctrl_id::LLM_MAX_TOKENS_EDIT), text.as_ptr()),
            0
        );
    }
    send_killfocus(hwnd, ctrl_id::LLM_MAX_TOKENS_EDIT);
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_MAX_TOKENS_EDIT)),
        max_tokens.to_string()
    );
    unsafe {
        let text = crate::win32::to_wide("0.73");
        assert_ne!(
            SetWindowTextW(control(hwnd, ctrl_id::LLM_TEMPERATURE_EDIT), text.as_ptr()),
            0
        );
    }
    send_killfocus(hwnd, ctrl_id::LLM_TEMPERATURE_EDIT);
    assert_eq!(
        unsafe {
            SendMessageW(
                control(hwnd, ctrl_id::LLM_TEMPERATURE_TRACKBAR),
                crate::dialogs::TBM_GETPOS,
                0,
                0,
            )
        },
        73
    );
    unsafe {
        let text = crate::win32::to_wide("invalid");
        assert_ne!(
            SetWindowTextW(control(hwnd, ctrl_id::LLM_TEMPERATURE_EDIT), text.as_ptr()),
            0
        );
    }
    send_killfocus(hwnd, ctrl_id::LLM_TEMPERATURE_EDIT);
    assert_eq!(
        crate::dialogs::helpers::get_window_text(control(hwnd, ctrl_id::LLM_TEMPERATURE_EDIT)),
        "0.73"
    );
    dialog.close();
}
