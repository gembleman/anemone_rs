use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE},
};

use super::{App, state::AppAction, state::DialogKind, state::Effect};
use crate::constants::WM_APP_ACTION;
use crate::dialogs::models::SettingsDraft;
use crate::window;

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
        if let Err(error) =
            unsafe { PostMessageW(Some(self.hwnd), WM_APP_ACTION, WPARAM(0), LPARAM(0)) }
        {
            tracing::error!("AppAction 알림을 게시하지 못했습니다: {error}");
        }
    }

    pub(crate) fn commit_settings(&self, draft: SettingsDraft) {
        self.send(AppAction::CommitSettings(draft));
    }

    pub(crate) fn preview_settings(&self, draft: SettingsDraft) {
        self.send(AppAction::PreviewSettings(draft));
    }

    pub(crate) fn clear_backlog(&self) {
        self.send(AppAction::ClearBacklog);
    }

    pub(crate) fn translate_dialog_closed(&self, session: u64) {
        self.send(AppAction::TranslateDialogClosed(session));
    }

    pub(crate) fn file_trans_dialog_closed(&self, session: u64) {
        self.send(AppAction::FileTransDialogClosed(session));
    }
}

impl App {
    pub(super) fn action_sender(&self) -> AppActionSender {
        AppActionSender::new(self.hwnd, self.action_queue.clone())
    }

    pub(super) fn process_actions(&mut self) {
        loop {
            let action = { self.action_queue.borrow_mut().pop_front() };
            let Some(action) = action else {
                break;
            };
            let effects = self.model.update(action);
            self.run_effects(effects);
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
                    if let Err(error) = self.model.config.save() {
                        tracing::error!("AppAction 설정 저장 실패: {error}");
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
                Effect::OpenDialog(kind) => match kind {
                    DialogKind::Settings => self.open_settings_dialog(),
                    DialogKind::Translate => self.open_translate_dialog(),
                    DialogKind::Backlog => self.open_backlog_dialog(),
                    DialogKind::FileTranslation => self.open_file_trans_dialog(),
                },
                Effect::TranslateDialogClosed(session) => {
                    self.handle_translate_dialog_closed(session);
                }
                Effect::FileTransDialogClosed(session) => {
                    self.handle_file_trans_dialog_closed(session);
                }
                Effect::Close => {
                    // DestroyWindow의 동기 재진입을 피하고 현재 reducer turn 뒤에 닫는다.
                    if let Err(error) =
                        unsafe { PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                    {
                        tracing::error!("WM_CLOSE 게시 실패: {error}");
                    }
                }
            }
        }
    }
}
