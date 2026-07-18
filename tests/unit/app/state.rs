use super::{
    AppCommand, ClientSize, ClipboardDebounce, MagneticAction, PendingTranslation,
    correlate_translation, magnetic_action,
};
use crate::menu;

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
        menu::id::HOOK_SETTINGS,
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
    let mut pending = Some(PendingTranslation::new(2, "latest".to_string()));
    let stale = correlate_translation(&mut pending, 1, Ok::<_, ()>("old result"));

    assert!(stale.is_none());
    assert_eq!(
        pending.as_ref().map(|p| p.original.as_str()),
        Some("latest")
    );
}

#[test]
fn success_and_failure_keep_the_matching_original() {
    let mut success = Some(PendingTranslation::new(7, "first".to_string()));
    let completed = correlate_translation(&mut success, 7, Ok::<_, ()>("translated"))
        .expect("current response");
    assert_eq!(completed.original, "first");
    assert_eq!(completed.result, Ok("translated"));
    assert!(success.is_none());

    let mut failure = Some(PendingTranslation::new(8, "second".to_string()));
    let completed = correlate_translation(&mut failure, 8, Err::<String, _>("failed"))
        .expect("current response");
    assert_eq!(completed.original, "second");
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
