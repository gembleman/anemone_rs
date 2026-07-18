use crate::menu;
use std::sync::Arc;

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
    pub original: Arc<str>,
}

impl PendingTranslation {
    pub(super) fn new(req_id: u64, original: impl Into<Arc<str>>) -> Self {
        Self {
            req_id,
            original: original.into(),
        }
    }
}

#[derive(Debug)]
pub(super) struct TranslationCompletion<T, E> {
    pub original: Arc<str>,
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
    pub original_text: String,
    pub translated_text: String,
    pub pending_translation: Option<PendingTranslation>,
    pub clipboard_debounce: ClipboardDebounce,
}

#[derive(Debug, Default)]
pub(super) struct ClipboardDebounce {
    pending: Option<String>,
}

impl ClipboardDebounce {
    pub(super) fn submit(&mut self, text: String) {
        self.pending = Some(text);
    }

    pub(super) fn take(&mut self) -> Option<String> {
        self.pending.take()
    }

    pub(super) fn clear(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app/state.rs"]
mod tests;
