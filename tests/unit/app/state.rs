use super::{
    AppAction, AppCommand, AppModel, AppState, ClientSize, ClipboardDebounce, Effect,
    HookSessionInfo, MagneticAction, OverlayNotice, PendingTranslation,
    clipboard_capture_is_paused, correlate_translation, magnetic_action, should_watch_clipboard,
    waits_for_pending_translation,
};
use crate::app::backlog::{BacklogFilter, BacklogStore, LogEntry};
use crate::app::commands::{command_from_hotkey_id, command_from_menu_id};
use crate::config::Config;
use crate::dialogs::models::SettingsDraft;
use crate::hotkey;
use crate::menu;
use crate::translation::CacheKey;

fn test_cache_key() -> CacheKey {
    CacheKey {
        engine_id: "test".to_string(),
        source_lang: "ja",
        target_lang: "ko",
        original: "test".to_string(),
    }
}

#[test]
fn clipboard_debounce_keeps_only_the_last_submission() {
    let mut debounce = ClipboardDebounce::default();
    for index in 0..20 {
        debounce.submit(format!("text-{index}"));
    }
    assert_eq!(debounce.take().as_deref(), Some("text-19"));
    assert!(debounce.take().is_none());
}

#[test]
fn magnetic_notices_use_the_requested_copy() {
    assert_eq!(
        OverlayNotice::SelectMagneticTarget.text(),
        "따라다닐 창을 선택해주세요."
    );
    assert_eq!(
        OverlayNotice::MagneticTargetAttached.text(),
        "해당 창을 따라다닐게요! >.<"
    );
}

#[test]
fn manual_translation_dialog_temporarily_suspends_clipboard_capture() {
    assert!(should_watch_clipboard(true, false));
    assert!(!should_watch_clipboard(true, true));
    assert!(!should_watch_clipboard(false, false));
    assert!(!should_watch_clipboard(false, true));
}

#[test]
fn all_non_idle_ui_states_pause_clipboard_capture() {
    assert!(!clipboard_capture_is_paused(
        false, false, false, false, false, false
    ));
    for active_reason in 0..6 {
        let mut reasons = [false; 6];
        reasons[active_reason] = true;
        assert!(clipboard_capture_is_paused(
            reasons[0], reasons[1], reasons[2], reasons[3], reasons[4], reasons[5]
        ));
    }
}

#[test]
fn an_active_hook_session_pauses_clipboard_capture() {
    // 후킹 중에는 게임 텍스트만 번역 경로를 쓴다 — 다른 창에서 복사한 텍스트가
    // 후킹 문장을 덮어쓰거나 그 게임 신원으로 유료 엔진에 나가면 안 된다.
    assert!(clipboard_capture_is_paused(
        false, false, false, false, false, true
    ));
}

#[test]
fn translate_dialog_close_is_forwarded_as_a_platform_effect() {
    let mut model = app_model();
    assert_eq!(
        model.update(AppAction::SettingsDialogClosed),
        vec![Effect::SettingsDialogClosed]
    );
    assert_eq!(
        model.update(AppAction::TranslateDialogClosed(17)),
        vec![Effect::TranslateDialogClosed(17)]
    );
    assert_eq!(
        model.update(AppAction::FileTransDialogClosed(23)),
        vec![Effect::FileTransDialogClosed(23)]
    );
}

#[test]
fn menu_ids_map_to_distinct_commands() {
    let ids = [
        menu::id::WINDOW_SHOW,
        menu::id::CLICK_THROUGH,
        menu::id::CLIPBOARD_WATCH,
        menu::id::BACKGROUND_TOGGLE,
        menu::id::BORDER_TOGGLE,
        menu::id::MAGNETIC_MODE,
        menu::id::SETTINGS,
        menu::id::TRANSLATE,
        menu::id::BACKLOG,
        menu::id::FILE_TRANS,
        menu::id::TEXT_SIZE_UP,
        menu::id::TEXT_SIZE_DOWN,
        menu::id::EXIT,
    ];

    let mut commands = ids
        .into_iter()
        .map(|id| command_from_menu_id(id).expect("known menu id"))
        .collect::<Vec<_>>();
    commands.sort_by_key(|command| *command as u8);
    commands.dedup();
    assert_eq!(commands.len(), ids.len());
    assert_eq!(command_from_menu_id(u16::MAX), None);
}

#[test]
fn hotkey_ids_map_directly_to_app_commands() {
    assert_eq!(
        command_from_hotkey_id(hotkey::id::TOGGLE_WINDOW),
        Some(AppCommand::WindowShow)
    );
    assert_eq!(
        command_from_hotkey_id(hotkey::id::TEXT_SIZE_UP),
        Some(AppCommand::TextSizeUp)
    );
    assert_eq!(
        command_from_hotkey_id(hotkey::id::TEXT_SIZE_DOWN),
        Some(AppCommand::TextSizeDown)
    );
    assert_eq!(
        command_from_hotkey_id(hotkey::id::CLIPBOARD_WATCH),
        Some(AppCommand::ClipboardWatch)
    );
    assert_eq!(command_from_hotkey_id(i32::MAX), None);
}

#[test]
fn magnetic_transition_is_idempotent() {
    assert_eq!(magnetic_action(true, false), MagneticAction::Start);
    assert_eq!(magnetic_action(true, true), MagneticAction::Noop);
    assert_eq!(magnetic_action(false, true), MagneticAction::Stop);
    assert_eq!(magnetic_action(false, false), MagneticAction::Noop);
}

#[test]
fn stale_translation_cannot_take_latest_original() {
    let mut pending = Some(PendingTranslation::new(
        2,
        "latest".to_string(),
        test_cache_key(),
    ));
    let stale = correlate_translation(&mut pending, 1, Ok::<_, ()>("old result"));

    assert!(stale.is_none());
    assert_eq!(
        pending.as_ref().map(|p| p.original.as_ref()),
        Some("latest")
    );
}

#[test]
fn success_and_failure_keep_the_matching_original() {
    let mut success = Some(PendingTranslation::new(
        7,
        "first".to_string(),
        test_cache_key(),
    ));
    let completed = correlate_translation(&mut success, 7, Ok::<_, ()>("translated"))
        .expect("current response");
    assert_eq!(completed.original.as_ref(), "first");
    assert_eq!(completed.result, Ok("translated"));
    assert!(success.is_none());

    let mut failure = Some(PendingTranslation::new(
        8,
        "second".to_string(),
        test_cache_key(),
    ));
    let completed = correlate_translation(&mut failure, 8, Err::<String, _>("failed"))
        .expect("current response");
    assert_eq!(completed.original.as_ref(), "second");
    assert_eq!(completed.result, Err("failed"));
    assert!(failure.is_none());
}

#[test]
fn drawable_size_rejects_zero_and_keeps_small_valid_clients() {
    assert_eq!(ClientSize::drawable(0, 100), None);
    assert_eq!(ClientSize::drawable(100, 0), None);
    assert_eq!(ClientSize::drawable(-1, 100), None);
    assert_eq!(ClientSize::drawable(1, 1), Some(ClientSize::new(1, 1)));
    assert_eq!(
        ClientSize::drawable(100, 100),
        Some(ClientSize::new(100, 100))
    );
    assert_eq!(
        ClientSize::drawable(640, 480),
        Some(ClientSize::new(640, 480))
    );
}

fn app_model() -> AppModel {
    let config = Config::default();
    AppModel {
        config,
        backlog: BacklogStore::new(),
        runtime: AppState {
            client_size: ClientSize::new(400, 200),
            resizing: false,
            pending_resize: None,
            original_text: String::new(),
            translated_text: String::new(),
            overlay_notice: None,
            pending_translation: None,
            clipboard_debounce: ClipboardDebounce::default(),
            hook_session: None,
            hook_merger: crate::hook::text_bridge::TextMerger::default(),
            hook_text_queue: std::collections::VecDeque::new(),
        },
    }
}

#[test]
fn command_reducer_mutates_model_and_returns_platform_effects() {
    let mut model = app_model();
    let previous = model.config.click_through;

    let effects = model.update(AppAction::Command(AppCommand::ClickThrough));

    assert_eq!(model.config.click_through, !previous);
    assert_eq!(effects, vec![Effect::SetClickThrough(!previous)]);
}

#[test]
fn settings_draft_distinguishes_preview_from_commit_effects() {
    let mut model = app_model();
    let mut changed = model.config.clone();
    changed.window_topmost = !changed.window_topmost;

    let effects = model.update(AppAction::PreviewSettings(SettingsDraft::new(
        changed.clone(),
    )));
    assert_eq!(model.config.window_topmost, changed.window_topmost);
    assert!(effects.contains(&Effect::SyncWindowState));
    assert!(effects.contains(&Effect::Repaint));
    assert!(!effects.contains(&Effect::SaveConfig));

    let effects = model.update(AppAction::CommitSettings(SettingsDraft::new(changed)));
    assert!(effects.contains(&Effect::SaveConfig));
}

#[test]
fn committing_settings_keeps_the_update_check_timestamp() {
    let mut model = app_model();
    // 설정 창을 열 때 draft를 스냅샷으로 뜬 상황을 재현한다.
    let draft = SettingsDraft::new(model.config.clone());

    // 설정 창이 열려 있는 동안 업데이트 워커가 last_update_check를 갱신한다.
    model.config.last_update_check = 1_700_000_000;

    // 사용자가 옛 draft로 적용을 눌러도 워커가 갱신한 값이 살아남아야 한다.
    model.update(AppAction::CommitSettings(draft));
    assert_eq!(model.config.last_update_check, 1_700_000_000);
}

#[test]
fn previewing_settings_keeps_the_update_check_timestamp() {
    let mut model = app_model();
    let draft = SettingsDraft::new(model.config.clone());

    model.config.last_update_check = 1_700_000_000;

    model.update(AppAction::PreviewSettings(draft));
    assert_eq!(model.config.last_update_check, 1_700_000_000);
}

#[test]
fn applying_settings_keeps_the_overlay_placement_changed_while_the_window_was_open() {
    for action in [
        AppAction::PreviewSettings as fn(SettingsDraft) -> AppAction,
        AppAction::CommitSettings,
    ] {
        let mut model = app_model();
        let draft = SettingsDraft::new(model.config.clone());

        // 설정 창이 열려 있는 동안 사용자가 오버레이를 끌어 옮기고 크기도 바꿨다.
        model.config.window_x = Some(640);
        model.config.window_y = Some(480);
        model.config.window_width = Some(720);
        model.config.window_height = Some(360);

        model.update(action(draft));
        assert_eq!(model.config.window_x, Some(640));
        assert_eq!(model.config.window_y, Some(480));
        assert_eq!(model.config.window_width, Some(720));
        assert_eq!(model.config.window_height, Some(360));
    }
}

#[test]
fn previewing_settings_does_not_consume_hotkey_change_before_commit() {
    let mut model = app_model();
    let mut changed = model.config.clone();
    changed.hotkeys.toggle_window.vk = b'B' as u32;
    let draft = SettingsDraft::new(changed.clone());

    // Live preview must leave the registered/configured hotkeys untouched until
    // the user presses Apply, even though visual settings are previewed at once.
    model.update(AppAction::PreviewSettings(draft.clone()));
    assert_ne!(model.config.hotkeys, changed.hotkeys);

    let effects = model.update(AppAction::CommitSettings(draft));
    assert!(effects.contains(&Effect::ReregisterHotkeys));
    assert_eq!(model.config.hotkeys, changed.hotkeys);
}

#[test]
fn clear_backlog_action_mutates_the_app_owned_store() {
    let mut model = app_model();
    model.backlog.push(LogEntry::new("line".into()));

    assert!(model.update(AppAction::ClearBacklog).is_empty());
    assert!(model.backlog.render(BacklogFilter::All, true).is_empty());
}

#[test]
fn hook_session_ignores_the_clipboard_watch_toggle_instead_of_flipping_it_silently() {
    let mut model = app_model();
    let configured = model.config.clipboard_watch;
    model.runtime.hook_session = Some(HookSessionInfo {
        pid: 42,
        process_name: "game.exe".into(),
        exe_sha256: None,
        arch_label: "x64".into(),
        detected_engines: Vec::new(),
    });

    // 단축키와 메뉴가 같은 명령으로 수렴하므로, 회색 처리한 메뉴 항목과 동작이
    // 어긋나지 않도록 명령 단계에서 막는다.
    assert!(
        model
            .update(AppAction::Command(AppCommand::ClipboardWatch))
            .is_empty()
    );
    assert_eq!(model.config.clipboard_watch, configured);

    // 세션이 끝나면 다시 평소대로 토글된다.
    model.runtime.hook_session = None;
    assert_eq!(
        model.update(AppAction::Command(AppCommand::ClipboardWatch)),
        vec![Effect::SetClipboardWatch(!configured)]
    );
    assert_eq!(model.config.clipboard_watch, !configured);
}

#[test]
fn selecting_a_hook_stream_saves_a_game_profile() {
    let mut model = app_model();
    model.runtime.hook_session = Some(HookSessionInfo {
        pid: 42,
        process_name: "game.exe".into(),
        exe_sha256: Some("digest".into()),
        arch_label: "x64".into(),
        detected_engines: Vec::new(),
    });

    let effects = model.update(AppAction::SaveHookProfile {
        hook_name: "UserUI".into(),
        hook_code: Some("HQ0@1000:game.exe".into()),
    });

    assert_eq!(effects, vec![Effect::SaveConfig]);
    let saved = model
        .config
        .hook
        .saved_profile("game.exe", Some("digest"))
        .unwrap();
    assert_eq!(saved.hook_name, "UserUI");
    assert_eq!(saved.hook_code.as_deref(), Some("HQ0@1000:game.exe"));
}

/// 후킹 관리 창에서 바꾼 병합 창은 config와 실행 중인 merger에 함께 간다.
/// 원본 LunaHost도 설정 창에서 `flushDelay`를 바로 바꾼다.
#[test]
fn changing_the_merge_window_updates_the_config_and_the_live_merger() {
    let mut model = app_model();
    let effects = model.update(AppAction::SetHookMergeWindow(400));

    assert_eq!(effects, vec![Effect::SaveConfig]);
    assert_eq!(model.config.hook.merge_window_ms, 400);
    assert_eq!(model.runtime.hook_merger.window_ms(), 400);
}

/// 범위 밖의 값은 잘라서 받는다. 같은 값이면 저장하지 않는다.
#[test]
fn the_merge_window_is_clamped_and_a_repeat_is_ignored() {
    let mut model = app_model();

    assert_eq!(
        model.update(AppAction::SetHookMergeWindow(1)),
        Vec::<Effect>::new(),
        "하한이 기본값과 같아 바뀌는 것이 없다"
    );
    assert_eq!(model.config.hook.merge_window_ms, 100);

    model.update(AppAction::SetHookMergeWindow(u32::MAX));
    assert_eq!(model.config.hook.merge_window_ms, 5_000);

    assert_eq!(
        model.update(AppAction::SetHookMergeWindow(5_000)),
        Vec::<Effect>::new()
    );
}

/// 진행 중인 요청이 있으면 후킹 문장은 기다린다. 그대로 보내면 워커가 앞
/// 요청을 취소해 앞 문장이 번역되지 않는다.
#[test]
fn a_hook_sentence_waits_for_the_request_in_flight() {
    assert!(!waits_for_pending_translation(None));
    assert!(waits_for_pending_translation(Some(
        std::time::Duration::from_secs(1)
    )));
}

/// 응답이 유실돼도 후킹 번역이 영영 멈추지는 않는다.
#[test]
fn a_lost_response_stops_blocking_the_queue() {
    assert!(!waits_for_pending_translation(Some(
        std::time::Duration::from_secs(60)
    )));
}
