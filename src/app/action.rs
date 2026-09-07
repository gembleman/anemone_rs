use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE},
};

use super::messages::WM_APP_ACTION;
use super::{App, state::AppAction, state::DialogKind, state::Effect};
use crate::dialogs::models::SettingsDraft;
use crate::window;

const ACTION_BATCH_SIZE: usize = 32;

/// 다이얼로그가 App의 소유 상태를 직접 공유하지 않고 action을 전달하는 UI-thread 채널.
#[derive(Clone)]
pub(crate) struct AppActionSender {
    hwnd: HWND,
    queue: Rc<RefCell<VecDeque<AppAction>>>,
}

impl AppActionSender {
    pub(super) fn new(hwnd: HWND, queue: Rc<RefCell<VecDeque<AppAction>>>) -> Self {
        Self { hwnd, queue }
    }

    fn send(&self, action: AppAction) {
        self.queue.borrow_mut().push_back(action);
        // SAFETY: hwnd는 App이 소유하고 action 데이터는 별도 queue가 소유한다.
        if unsafe { PostMessageW(self.hwnd, WM_APP_ACTION, 0, 0) } == 0 {
            tracing::error!("AppAction 알림을 게시하지 못했습니다");
        }
    }

    pub(crate) fn commit_settings(&self, draft: SettingsDraft) {
        self.send(AppAction::CommitSettings(draft));
    }

    pub(crate) fn preview_settings(&self, draft: SettingsDraft) {
        let mut queue = self.queue.borrow_mut();
        let notify = !matches!(queue.back(), Some(AppAction::PreviewSettings(_)));
        if let Some(AppAction::PreviewSettings(pending)) = queue.back_mut() {
            *pending = draft;
        } else {
            queue.push_back(AppAction::PreviewSettings(draft));
        }
        drop(queue);
        if notify && unsafe { PostMessageW(self.hwnd, WM_APP_ACTION, 0, 0) } == 0 {
            tracing::error!("AppAction 알림을 게시하지 못했습니다");
        }
    }

    pub(crate) fn clear_backlog(&self) {
        self.send(AppAction::ClearBacklog);
    }

    pub(crate) fn clear_translation_cache(&self) {
        self.send(AppAction::ClearTranslationCache);
    }

    pub(crate) fn set_translation_route(
        &self,
        route: crate::config::TranslationRoute,
        config: crate::config::TranslationRouteConfig,
    ) {
        self.send(AppAction::SetTranslationRoute { route, config });
    }

    pub(crate) fn save_hook_profile(&self, hook_name: String, hook_code: Option<String>) {
        self.send(AppAction::SaveHookProfile {
            hook_name,
            hook_code,
        });
    }

    pub(crate) fn set_hook_merge_window(&self, window_ms: u32) {
        self.send(AppAction::SetHookMergeWindow(window_ms));
    }

    pub(crate) fn settings_dialog_closed(&self) {
        self.send(AppAction::SettingsDialogClosed);
    }

    pub(crate) fn translate_dialog_closed(&self, session: u64) {
        self.send(AppAction::TranslateDialogClosed(session));
    }

    pub(crate) fn file_trans_dialog_closed(&self, session: u64) {
        self.send(AppAction::FileTransDialogClosed(session));
    }

    pub(crate) fn request_update_check(&self) {
        self.send(AppAction::RequestUpdateCheck);
    }

    pub(crate) fn request_update_apply(&self) {
        self.send(AppAction::RequestUpdateApply);
    }
}

impl App {
    pub(super) fn action_sender(&self) -> AppActionSender {
        AppActionSender::new(self.hwnd, self.action_queue.clone())
    }

    pub(super) fn process_actions(&mut self) {
        for _ in 0..ACTION_BATCH_SIZE {
            let action = { self.action_queue.borrow_mut().pop_front() };
            let Some(action) = action else {
                break;
            };
            // 슬라이더를 끌 때 같은 message turn에 쌓인 preview는 마지막
            // 스냅숏만 적용한다. 중간 스냅숏마다 전체 paint를 하지 않는다.
            let action = if let AppAction::PreviewSettings(mut draft) = action {
                loop {
                    let next = { self.action_queue.borrow_mut().pop_front() };
                    match next {
                        Some(AppAction::PreviewSettings(next_draft)) => draft = next_draft,
                        Some(other) => {
                            self.action_queue.borrow_mut().push_front(other);
                            break;
                        }
                        None => break,
                    }
                }
                AppAction::PreviewSettings(draft)
            } else {
                action
            };
            let effects = self.model.update(action);
            self.run_effects(effects);
        }
        if !self.action_queue.borrow().is_empty()
            && unsafe { PostMessageW(self.hwnd, WM_APP_ACTION, 0, 0) } == 0
        {
            tracing::error!("남은 AppAction 알림을 게시하지 못했습니다");
        }
    }

    pub(super) fn run_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Repaint => {
                    if let Err(error) = self.paint() {
                        tracing::warn!("AppAction repaint failed: {error}");
                    }
                }
                Effect::SyncWindowState => self.sync_window_state(),
                Effect::SaveConfig => {
                    if !self.services.config_save.request(self.model.config.clone()) {
                        tracing::error!("설정 저장 워커가 요청을 받지 못했습니다");
                    }
                }
                Effect::SetWindowVisible(visible) => {
                    window::set_window_visible(self.hwnd, visible);
                    if let Some(magnetic) = self.magnetic.as_mut() {
                        magnetic.update_policy(
                            self.model.config.magnetic_minimize,
                            self.model.config.window_visible,
                        );
                    }
                }
                Effect::SetClickThrough(enabled) => {
                    window::set_click_through(self.hwnd, enabled);
                }
                Effect::SetClipboardWatch(enabled) => self.apply_clipboard_watch(enabled),
                Effect::SetMagnetic(enabled) => self.apply_magnetic_request(enabled),
                Effect::ReregisterHotkeys => self.reregister_hotkeys(),
                Effect::OpenDialog(kind) => match kind {
                    DialogKind::Settings => self.open_settings_dialog(),
                    DialogKind::Translate => self.open_translate_dialog(),
                    DialogKind::Backlog => self.open_backlog_dialog(),
                    DialogKind::FileTranslation => self.open_file_trans_dialog(),
                    DialogKind::HookFind => self.open_hook_find_dialog(),
                },
                Effect::ClearTranslationCache => self.services.translation_cache.clear(),
                Effect::SettingsDialogClosed => self.handle_settings_dialog_closed(),
                Effect::TranslateDialogClosed(session) => {
                    self.handle_translate_dialog_closed(session);
                }
                Effect::FileTransDialogClosed(session) => {
                    self.handle_file_trans_dialog_closed(session);
                }
                Effect::Close => {
                    // DestroyWindow의 동기 재진입을 피하고 현재 reducer turn 뒤에 닫는다.
                    if unsafe { PostMessageW(self.hwnd, WM_CLOSE, 0, 0) } == 0 {
                        tracing::error!("WM_CLOSE 게시 실패");
                    }
                }
                Effect::RequestUpdateCheck => self.start_manual_update_check(),
                Effect::RequestUpdateApply => self.start_update_apply(),
                Effect::HookStop => self.stop_hook_session(),
            }
        }
    }
}
