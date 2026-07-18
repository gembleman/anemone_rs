use crate::menu;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ClientSize {
    pub width: i32,
    pub height: i32,
}

impl ClientSize {
    pub(super) const fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }

    /// DXGI cannot resize to an empty surface. Minimized and malformed sizes are ignored.
    pub(super) fn drawable(width: i32, height: i32) -> Option<Self> {
        (width > 0 && height > 0).then_some(Self { width, height })
    }
}

#[derive(Debug)]
pub(super) struct PendingTranslation {
    pub req_id: u64,
    pub original: String,
}

impl PendingTranslation {
    pub(super) fn new(req_id: u64, original: String) -> Self {
        Self { req_id, original }
    }
}

#[derive(Debug)]
pub(super) struct TranslationCompletion<T, E> {
    pub original: String,
    pub result: Result<T, E>,
}

/// Consume a response only when it belongs to the currently displayed request.
/// Stale responses are deliberately consumed by the worker response store but do not
/// mutate application state.
pub(super) fn correlate_translation<T, E>(
    pending: &mut Option<PendingTranslation>,
    req_id: u64,
    result: Result<T, E>,
) -> Option<TranslationCompletion<T, E>> {
    if pending.as_ref().map(|request| request.req_id) != Some(req_id) {
        return None;
    }

    let request = pending.take().expect("matching pending translation exists");
    Some(TranslationCompletion {
        original: request.original,
        result,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MagneticAction {
    Noop,
    Start,
    Stop,
}

pub(super) const fn magnetic_action(enabled: bool, active: bool) -> MagneticAction {
    match (enabled, active) {
        (true, false) => MagneticAction::Start,
        (false, true) => MagneticAction::Stop,
        _ => MagneticAction::Noop,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AppCommand {
    WindowShow,
    ClickThrough,
    ClipboardWatch,
    BackgroundToggle,
    BorderToggle,
    MagneticMode,
    Settings,
    Translate,
    Backlog,
    FileTrans,
    HookSettings,
    TextSizeUp,
    TextSizeDown,
    Exit,
}

impl AppCommand {
    pub(super) const fn from_menu_id(id: u16) -> Option<Self> {
        match id {
            menu::id::WINDOW_SHOW => Some(Self::WindowShow),
            menu::id::CLICK_THROUGH => Some(Self::ClickThrough),
            menu::id::CLIPBOARD_WATCH => Some(Self::ClipboardWatch),
            menu::id::BACKGROUND_TOGGLE => Some(Self::BackgroundToggle),
            menu::id::BORDER_TOGGLE => Some(Self::BorderToggle),
            menu::id::MAGNETIC_MODE => Some(Self::MagneticMode),
            menu::id::SETTINGS => Some(Self::Settings),
            menu::id::TRANSLATE => Some(Self::Translate),
            menu::id::BACKLOG => Some(Self::Backlog),
            menu::id::FILE_TRANS => Some(Self::FileTrans),
            menu::id::HOOK_SETTINGS => Some(Self::HookSettings),
            menu::id::TEXT_SIZE_UP => Some(Self::TextSizeUp),
            menu::id::TEXT_SIZE_DOWN => Some(Self::TextSizeDown),
            menu::id::EXIT => Some(Self::Exit),
            _ => None,
        }
    }
}

pub(super) struct AppState {
    pub client_size: ClientSize,
    pub current_text: String,
    pub pending_translation: Option<PendingTranslation>,
}

#[cfg(test)]
mod tests {
    use super::{
        AppCommand, ClientSize, MagneticAction, PendingTranslation, correlate_translation,
        magnetic_action,
    };
    use crate::menu;

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
}
