use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    /// 요청을 보낸 시각. 응답이 유실됐을 때를 판정한다.
    requested_at: Instant,
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
            requested_at: Instant::now(),
        }
    }

    pub(super) fn age(&self) -> Duration {
        self.requested_at.elapsed()
    }
}

/// 번역을 기다리는 후킹 문장 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueuedHookText {
    pub text: String,
    /// 문장을 통째로 내는 훅에서 왔는지. 글자 단위 훅에서 온 것은 병합 창이
    /// 문장 한가운데서 끊은 조각일 수 있어 이어 붙일 때 줄을 나누지 않는다.
    pub full_string: bool,
}

/// 후킹 문장 대기열의 최대 길이. 쉬지 않고 뱉는 게임에서 무한히 쌓이지 않게 한다.
/// 넘치면 가장 오래된 문장을 버린다.
pub(super) const MAX_QUEUED_HOOK_TEXTS: usize = 8;

/// 앞 요청이 이 시간을 넘기도록 응답이 없으면 유실로 보고 다음 문장을 그냥
/// 보낸다. 응답 하나가 사라져도 후킹 번역이 영영 멈추지 않게 한다.
const HOOK_PENDING_TIMEOUT: Duration = Duration::from_secs(30);

/// 진행 중인 요청이 끝나기를 기다려야 하는지.
///
/// 워커는 같은 창의 새 요청이 오면 앞 요청을 취소한다. 클립보드는 최신 것만
/// 보면 되지만 후킹 문장은 연속으로 나오므로, 그대로 보내면 앞 문장이 번역되지
/// 않고 사라진다. 그래서 후킹 경로는 앞 요청이 끝난 뒤에 보낸다.
pub(super) fn waits_for_pending_translation(pending_age: Option<Duration>) -> bool {
    matches!(pending_age, Some(age) if age < HOOK_PENDING_TIMEOUT)
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

    // 위 검사로 `req_id`가 일치하는 pending이 있음을 확인했다. 그 사이 다른
    // 코드가 끼어들 수 없는 단일 스레드 상태 갱신이므로 실질적으로 항상
    // `Some`이지만, 패닉 대신 조용히 스킵해 호출부가 안전하게 이어지게 한다.
    let request = pending.take()?;
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

/// 클립보드 캡처를 일시정지해야 하는 상태인지.
///
/// `hook_session`은 다른 이유들과 성격이 다르다 — 후킹 중에는 게임 텍스트가
/// 이미 번역 파이프라인을 채우므로, 사용자가 복사한 텍스트까지 같은 경로로
/// 들어오면 후킹 문장을 덮어쓰고(진행 중이던 요청은 supersede된다) 유료 엔진
/// 호출도 낭비된다.
pub(super) const fn clipboard_capture_is_paused(
    translation_dialog: bool,
    file_translation_dialog: bool,
    settings_dialog: bool,
    context_menu: bool,
    overlay_notice: bool,
    hook_session: bool,
) -> bool {
    translation_dialog
        || file_translation_dialog
        || settings_dialog
        || context_menu
        || overlay_notice
        || hook_session
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
    /// 게임 후킹: 대상 선택과 후크 관리를 통합한 창.
    HookFind,
    /// 현재 세션을 끊는다.
    HookStop,
    Exit,
}

pub(super) struct AppState {
    pub client_size: ClientSize,
    /// WM_ENTERSIZEMOVE ~ WM_EXITSIZEMOVE 동안 true. WM_SIZE는 드래그 내내
    /// 연속으로 오므로 그 사이 ResizeBuffers/layout/bitmap 재빌드를 미룬다.
    pub resizing: bool,
    /// interactive resize 중 최종 목표 크기. 종료 시 1회만 적용한다.
    pub pending_resize: Option<ClientSize>,
    pub original_text: String,
    pub translated_text: String,
    pub overlay_notice: Option<OverlayNotice>,
    pub pending_translation: Option<PendingTranslation>,
    pub clipboard_debounce: ClipboardDebounce,
    /// 현재 후킹 중인 게임. `None`이면 세션 없음.
    pub hook_session: Option<HookSessionInfo>,
    /// 도착한 후킹 텍스트의 병합 창.
    pub hook_merger: crate::hook::text_bridge::TextMerger,
    /// 앞 문장의 번역이 끝나기를 기다리는 후킹 문장들.
    pub hook_text_queue: VecDeque<QueuedHookText>,
}

/// 활성 후킹 세션의 표시 정보.
#[derive(Debug, Clone)]
pub(super) struct HookSessionInfo {
    pub pid: u32,
    pub process_name: String,
    pub exe_sha256: Option<String>,
    pub arch_label: String,
    pub detected_engines: Vec<String>,
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
    SaveHookProfile {
        hook_name: String,
        hook_code: Option<String>,
    },
    /// 후킹 관리 창에서 문장 병합 창(ms)을 바꿨다. 원본 LunaHost의 설정 창에서
    /// `TextThread::flushDelay`를 바꾸는 것과 같다.
    SetHookMergeWindow(u32),
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
    /// 현재 후킹 세션을 끊는다.
    HookStop,
}

/// 설정 UI가 편집하지 않는 오버레이 창 배치. draft로 config를 덮어쓸 때
/// 살려 두기 위한 임시 보관용이다.
#[derive(Debug, Clone, Copy)]
struct WindowPlacement {
    x: Option<i32>,
    y: Option<i32>,
    width: Option<i32>,
    height: Option<i32>,
}

impl WindowPlacement {
    fn capture(config: &Config) -> Self {
        Self {
            x: config.window_x,
            y: config.window_y,
            width: config.window_width,
            height: config.window_height,
        }
    }

    fn restore(self, config: &mut Config) {
        config.window_x = self.x;
        config.window_y = self.y;
        config.window_width = self.width;
        config.window_height = self.height;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DialogKind {
    Settings,
    Translate,
    Backlog,
    FileTranslation,
    HookFind,
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
                // 단축키는 preview 단계에서 OS 등록을 바꾸지 않는다. 미리보기
                // 때 draft의 hotkeys까지 config에 복사하면 CommitSettings가
                // "이미 같은 값"으로 판단해 ReregisterHotkeys를 빠뜨린다.
                // 실제 적용 시점까지 현재 등록 상태를 유지해 변경 비교를
                // 순서와 무관하게 만든다.
                let hotkeys = self.config.hotkeys.clone();
                let hook = self.config.hook.clone();
                let placement = WindowPlacement::capture(&self.config);
                self.config = draft.into_config();
                self.config.last_update_check = last_update_check;
                self.config.hotkeys = hotkeys;
                self.config.hook = hook;
                placement.restore(&mut self.config);
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
                let hook = self.config.hook.clone();
                // 창 위치와 크기도 설정 UI가 편집하지 않는 값이다. 설정 창이 열려
                // 있는 동안 오버레이를 끌거나 크기를 바꿨다면 draft의 낡은 값이
                // 그것을 지운다.
                let placement = WindowPlacement::capture(&self.config);
                self.config = draft.into_config();
                self.config.last_update_check = last_update_check;
                self.config.hook = hook;
                placement.restore(&mut self.config);
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
            AppAction::SaveHookProfile {
                hook_name,
                hook_code,
            } => {
                let Some(session) = self.runtime.hook_session.as_ref() else {
                    return Vec::new();
                };
                self.config
                    .hook
                    .save_profile(crate::config::SavedHookProfile {
                        process_name: session.process_name.clone(),
                        exe_sha256: session.exe_sha256.clone(),
                        hook_name,
                        hook_code,
                    });
                vec![Effect::SaveConfig]
            }
            AppAction::SetHookMergeWindow(window_ms) => {
                let window_ms = window_ms.clamp(
                    crate::config::MIN_MERGE_WINDOW_MS,
                    crate::config::MAX_MERGE_WINDOW_MS,
                );
                if self.config.hook.merge_window_ms == window_ms {
                    return Vec::new();
                }
                self.config.hook.merge_window_ms = window_ms;
                self.runtime.hook_merger.set_window_ms(u64::from(window_ms));
                vec![Effect::SaveConfig]
            }
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
            // 후킹 중에는 감시가 이미 멈춰 있어(`clipboard_capture_is_paused`) 지금
            // 토글해도 달라지는 게 없다. 설정 값만 조용히 뒤집혀 detach 뒤에 사용자가
            // 기억하지 못하는 상태로 되살아나지 않도록, 회색 처리한 메뉴 항목과 같이
            // 명령 자체를 무시한다.
            AppCommand::ClipboardWatch if self.runtime.hook_session.is_some() => Vec::new(),
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
            AppCommand::HookFind => vec![Effect::OpenDialog(DialogKind::HookFind)],
            AppCommand::HookStop => vec![Effect::HookStop],
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
