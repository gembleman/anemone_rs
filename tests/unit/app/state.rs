use super::{
    AppAction, AppCommand, AppModel, AppState, ClientSize, ClipboardDebounce, Effect,
    MagneticAction, OverlayNotice, PendingTranslation, clipboard_capture_is_paused,
    correlate_translation, magnetic_action, should_watch_clipboard,
};
use crate::backlog::{BacklogFilter, BacklogStore, LogEntry};
use crate::cache::CacheKey;
use crate::config::Config;
use crate::dialogs::models::SettingsDraft;
use crate::menu;

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
        false, false, false, false, false
    ));
    for active_reason in 0..5 {
        let mut reasons = [false; 5];
        reasons[active_reason] = true;
        assert!(clipboard_capture_is_paused(
            reasons[0], reasons[1], reasons[2], reasons[3], reasons[4]
        ));
    }
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
        .map(|id| AppCommand::from_menu_id(id).expect("known menu id"))
        .collect::<Vec<_>>();
    commands.sort_by_key(|command| *command as u8);
    commands.dedup();
    assert_eq!(commands.len(), ids.len());
    assert_eq!(AppCommand::from_menu_id(u16::MAX), None);
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
    let mut pending = Some(PendingTranslation::new(2, "latest".to_string(), test_cache_key()));
    let stale = correlate_translation(&mut pending, 1, Ok::<_, ()>("old result"));

    assert!(stale.is_none());
    assert_eq!(
        pending.as_ref().map(|p| p.original.as_ref()),
        Some("latest")
    );
}

#[test]
fn success_and_failure_keep_the_matching_original() {
    let mut success = Some(PendingTranslation::new(7, "first".to_string(), test_cache_key()));
    let completed = correlate_translation(&mut success, 7, Ok::<_, ()>("translated"))
        .expect("current response");
    assert_eq!(completed.original.as_ref(), "first");
    assert_eq!(completed.result, Ok("translated"));
    assert!(success.is_none());

    let mut failure = Some(PendingTranslation::new(8, "second".to_string(), test_cache_key()));
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
    AppModel {
        config: Config::default(),
        backlog: BacklogStore::new(),
        runtime: AppState {
            client_size: ClientSize::new(400, 200),
            original_text: String::new(),
            translated_text: String::new(),
            overlay_notice: None,
            pending_translation: None,
            clipboard_debounce: ClipboardDebounce::default(),
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
fn clear_backlog_action_mutates_the_app_owned_store() {
    let mut model = app_model();
    model.backlog.push(LogEntry::new("line".into()));

    assert!(model.update(AppAction::ClearBacklog).is_empty());
    assert!(model.backlog.render(BacklogFilter::All, true).is_empty());
}
