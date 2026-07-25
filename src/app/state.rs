use std::sync::Arc;

use crate::config::Config;
use crate::dialogs::models::SettingsDraft;

use super::backlog::BacklogStore;

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
    pub cache_key: crate::translation::CacheKey,
}

impl PendingTranslation {
    pub(super) fn new(
        req_id: u64,
        original: impl Into<Arc<str>>,
        cache_key: crate::translation::CacheKey,
    ) -> Self {
        Self {
            req_id,
            original: original.into(),
            cache_key,
        }
    }
}

#[derive(Debug)]
pub(super) struct TranslationCompletion<T, E> {
    pub original: Arc<str>,
    pub cache_key: crate::translation::CacheKey,
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
        cache_key: request.cache_key,
        result,
    })
}

/// 사용자가 감시를 켰더라도 수동 번역 창이 열려 있는 동안에는 clipboard를 캡처하지 않는다.
pub(super) const fn should_watch_clipboard(
    configured: bool,
    translate_dialog_active: bool,
) -> bool {
    configured && !translate_dialog_active
}

pub(super) const fn clipboard_capture_is_paused(
    translation_dialog: bool,
    file_translation_dialog: bool,
    settings_dialog: bool,
    context_menu: bool,
    overlay_notice: bool,
) -> bool {
    translation_dialog
        || file_translation_dialog
        || settings_dialog
        || context_menu
        || overlay_notice
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
    TextSizeUp,
    TextSizeDown,
    Exit,
}

pub(super) struct AppState {
    pub client_size: ClientSize,
    pub original_text: String,
    pub translated_text: String,
    pub overlay_notice: Option<OverlayNotice>,
    pub pending_translation: Option<PendingTranslation>,
    pub clipboard_debounce: ClipboardDebounce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OverlayNotice {
    SelectMagneticTarget,
    MagneticTargetAttached,
}

impl OverlayNotice {
    pub(super) const fn text(self) -> &'static str {
        match self {
            Self::SelectMagneticTarget => "따라다닐 창을 선택해주세요.",
            Self::MagneticTargetAttached => "해당 창을 따라다닐게요! >.<",
        }
    }
}

/// Win32 handle과 service를 제외한 애플리케이션의 단일 상태 소유자.
pub(super) struct AppModel {
    pub config: Config,
    pub backlog: BacklogStore,
    pub runtime: AppState,
}

#[derive(Clone)]
pub(super) enum AppAction {
    Command(AppCommand),
    PreviewSettings(SettingsDraft),
    CommitSettings(SettingsDraft),
    ClearBacklog,
    ClearTranslationCache,
    SettingsDialogClosed,
    TranslateDialogClosed(u64),
    FileTransDialogClosed(u64),
    /// 업데이트 확인이 끝났다. `last_update_check`를 갱신할지는 오류 종류에 달려
    /// 있으므로 갱신할 시각(성공/스킵 불가 오류)만 담아 보낸다. `None`이면 이번
    /// 결과로는 시각을 갱신하지 않는다(네트워크 실패 등).
    UpdateCheckSettled(Option<i64>),
    /// 설정 창의 "업데이트 확인" 버튼이 눌렸다.
    RequestUpdateCheck,
    /// 설정 창에서 발견한 업데이트의 다운로드·적용을 요청했다.
    RequestUpdateApply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Effect {
    Repaint,
    SyncWindowState,
    SaveConfig,
    SetWindowVisible(bool),
    SetClickThrough(bool),
    SetClipboardWatch(bool),
    SetMagnetic(bool),
    /// 단축키 설정이 바뀌었으므로 App이 소유한 HotkeyManager를 config 기준으로 재등록해야 한다.
    ReregisterHotkeys,
    ClearTranslationCache,
    OpenDialog(DialogKind),
    SettingsDialogClosed,
    TranslateDialogClosed(u64),
    FileTransDialogClosed(u64),
    Close,
    RequestUpdateCheck,
    RequestUpdateApply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DialogKind {
    Settings,
    Translate,
    Backlog,
    FileTranslation,
}

impl AppModel {
    /// 상태만 변경하고 Win32/I/O 작업은 effect로 반환한다.
    pub(super) fn update(&mut self, action: AppAction) -> Vec<Effect> {
        match action {
            AppAction::Command(command) => self.update_command(command),
            AppAction::PreviewSettings(draft) => {
                // draft는 설정 창을 열 때 만든 스냅샷이라 last_update_check가 없다(0).
                // 미리보기에서 그대로 반영하면 이후 CommitSettings까지 갱신된 값이
                // 사라진 채로 이어지므로, 여기서도 기존 값을 보존해 둔다.
                let last_update_check = self.config.last_update_check;
                self.config = draft.into_config();
                self.config.last_update_check = last_update_check;
                vec![Effect::SyncWindowState, Effect::Repaint]
            }
            AppAction::CommitSettings(draft) => {
                let hotkeys_changed = self.config.hotkeys != draft.hotkeys;
                // last_update_check는 설정 UI가 편집하는 필드가 아니라 업데이트
                // 워커가 백그라운드에서 갱신하는 값이다. draft는 설정 창을 열 때의
                // config 스냅샷이므로, 창이 열려 있는 동안 워커가 값을 갱신했다면
                // draft로 그대로 덮어쓸 경우 그 갱신이 사라진다. 그래서 이 필드만은
                // draft가 아니라 현재 self.config 값을 유지한다.
                let last_update_check = self.config.last_update_check;
                self.config = draft.into_config();
                self.config.last_update_check = last_update_check;
                let mut effects =
                    vec![Effect::SyncWindowState, Effect::Repaint, Effect::SaveConfig];
                if hotkeys_changed {
                    effects.push(Effect::ReregisterHotkeys);
                }
                effects
            }
            AppAction::ClearBacklog => {
                self.backlog.clear();
                Vec::new()
            }
            AppAction::ClearTranslationCache => vec![Effect::ClearTranslationCache],
            AppAction::SettingsDialogClosed => vec![Effect::SettingsDialogClosed],
            AppAction::TranslateDialogClosed(session) => {
                vec![Effect::TranslateDialogClosed(session)]
            }
            AppAction::FileTransDialogClosed(session) => {
                vec![Effect::FileTransDialogClosed(session)]
            }
            AppAction::UpdateCheckSettled(new_last_check) => {
                let Some(timestamp) = new_last_check else {
                    // 네트워크 실패·타임아웃: 지금 갱신하면 오프라인이었던 하루 때문에
                    // 다음 24시간을 더 놓칠 수 있으므로 값을 그대로 둔다.
                    return Vec::new();
                };
                self.config.last_update_check = timestamp;
                vec![Effect::SaveConfig]
            }
            AppAction::RequestUpdateCheck => vec![Effect::RequestUpdateCheck],
            AppAction::RequestUpdateApply => vec![Effect::RequestUpdateApply],
        }
    }

    fn update_command(&mut self, command: AppCommand) -> Vec<Effect> {
        match command {
            AppCommand::WindowShow => {
                self.config.toggle_window_visible();
                vec![Effect::SetWindowVisible(self.config.window_visible)]
            }
            AppCommand::ClickThrough => {
                self.config.toggle_click_through();
                vec![Effect::SetClickThrough(self.config.click_through)]
            }
            AppCommand::ClipboardWatch => {
                self.config.clipboard_watch = !self.config.clipboard_watch;
                vec![Effect::SetClipboardWatch(self.config.clipboard_watch)]
            }
            AppCommand::BackgroundToggle => {
                self.config.toggle_background_visible();
                vec![Effect::Repaint]
            }
            AppCommand::BorderToggle => {
                self.config.toggle_border_visible();
                vec![Effect::Repaint]
            }
            AppCommand::MagneticMode => {
                self.config.magnetic_mode = !self.config.magnetic_mode;
                vec![Effect::SetMagnetic(self.config.magnetic_mode)]
            }
            AppCommand::Settings => vec![Effect::OpenDialog(DialogKind::Settings)],
            AppCommand::Translate => vec![Effect::OpenDialog(DialogKind::Translate)],
            AppCommand::Backlog => vec![Effect::OpenDialog(DialogKind::Backlog)],
            AppCommand::FileTrans => vec![Effect::OpenDialog(DialogKind::FileTranslation)],
            AppCommand::TextSizeUp => {
                let new_size = (self.config.translation_style.size + 1).min(100);
                self.set_all_text_sizes(new_size);
                vec![Effect::Repaint]
            }
            AppCommand::TextSizeDown => {
                let new_size = (self.config.translation_style.size - 1).max(6);
                self.set_all_text_sizes(new_size);
                vec![Effect::Repaint]
            }
            AppCommand::Exit => vec![Effect::Close],
        }
    }

    fn set_all_text_sizes(&mut self, size: i32) {
        self.config.translation_style.size = size;
        self.config.name_style.size = size;
        self.config.original_style.size = size;
    }
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
