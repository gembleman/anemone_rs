//! 설정/번역/백로그/파일 번역/후킹 대상 선택 대화상자의 열기·닫기 처리.

use windows_sys::Win32::Foundation::HWND;

use crate::app::App;
use crate::dialogs::{BacklogDialog, FileTransDialog, SettingsDialog, TranslateDialog};

impl App {
    /// 후킹 대상 선택과 후크 관리가 통합된 창.
    pub(in crate::app) fn open_hook_find_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let hook = self.services.hook.clone();
        let actions = self.action_sender();
        let target_label = self.model.runtime.hook_session.as_ref().map(|session| {
            format!(
                "{} (PID {}, {})",
                session.process_name, session.pid, session.arch_label
            )
        });
        let merge_window_ms = self.model.config.hook.merge_window_ms;
        Self::open_dialog_generic("hook_find", || {
            crate::dialogs::hook_find::HookFindDialog::show(
                main_hwnd,
                hook,
                target_label,
                actions,
                merge_window_ms,
            )
        });
    }

    /// 현재 후킹 세션을 끊고 메뉴 상태를 갱신한다.
    pub(in crate::app) fn stop_hook_session(&mut self) {
        let Some(session) = self.model.runtime.hook_session.take() else {
            return;
        };
        tracing::info!(
            pid = session.pid,
            name = %session.process_name,
            arch = %session.arch_label,
            "후킹 중지 요청"
        );
        // Detached 이벤트를 기다리지 않고 즉시 스냅샷을 내린다.
        crate::hook::set_session_active(false);
        crate::hook::set_session_identity(None);
        crate::dialogs::hook_find::notify_detaching();
        if self
            .services
            .hook
            .request(crate::hook::HookRequest::Detach)
            .is_err()
        {
            crate::dialogs::hook_find::notify_detached();
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "후킹 중지 실패",
                "후킹 워커가 이미 종료되었습니다.",
            );
        }
        // Detach 완료는 워커의 Detached 이벤트로 확인되지만, UI 반응성을 위해
        // 즉시 병합 대기열도 비운다.
        self.clear_hook_pending();
        // 워커가 이미 죽었으면 Detached 이벤트가 영영 오지 않는다. 세션 스냅샷을
        // 내린 이 자리에서 클립보드 감시도 함께 되돌린다(재개는 멱등이다).
        self.resume_clipboard_capture();
    }

    /// 각 dialog의 instance registry가 기존 창 focus와 새 창 생성을 책임진다.
    fn open_dialog_generic<F, E>(dialog_name: &str, create_fn: F)
    where
        F: FnOnce() -> std::result::Result<HWND, E>,
        E: std::fmt::Display,
    {
        match create_fn() {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Failed to open {} dialog: {}", dialog_name, e);
            }
        }
    }

    /// 설정 대화상자 열기
    pub(in crate::app) fn open_settings_dialog(&mut self) {
        self.settings_dialog_active = true;
        if !self.pause_clipboard_capture("설정 창") {
            self.settings_dialog_active = false;
            return;
        }
        let main_hwnd = self.hwnd;
        let config = self.model.config.clone();
        let actions = self.action_sender();
        if let Err(error) = SettingsDialog::show(main_hwnd, config, Some(actions)) {
            self.settings_dialog_active = false;
            tracing::error!("Failed to open settings dialog: {error}");
            self.resume_clipboard_capture();
        }
    }

    pub(in crate::app) fn handle_settings_dialog_closed(&mut self) {
        self.settings_dialog_active = false;
        self.resume_clipboard_capture();
    }

    /// 번역 대화상자 열기
    pub(in crate::app) fn open_translate_dialog(&mut self) {
        if let Some(session) = TranslateDialog::current_session() {
            self.translate_dialog_session = Some(session);
            if !self.pause_clipboard_capture("번역 창") {
                return;
            }
            let main_hwnd = self.hwnd;
            let config = self.model.config.clone();
            let translation = self.services.translation_ui.clone();
            let actions = self.action_sender();
            Self::open_dialog_generic("translate", || {
                TranslateDialog::show(main_hwnd, config, translation, actions, session)
            });
            return;
        }
        // 파괴 알림 action보다 재열기 명령이 먼저 도착한 경우 이전 session을 폐기한다.
        self.translate_dialog_session = None;

        // 창 생성보다 먼저 listener와 이미 예약된 자동 번역을 멈춰, 초기화 중 발생한
        // clipboard 변경도 수동 번역 경로로만 소비되게 한다.
        if !self.pause_clipboard_capture("번역 창") {
            return;
        }

        let session = self.next_clipboard_pause_session();
        self.translate_dialog_session = Some(session);

        let main_hwnd = self.hwnd;
        let config = self.model.config.clone();
        let translation = self.services.translation_ui.clone();
        let actions = self.action_sender();
        if let Err(error) = TranslateDialog::show(main_hwnd, config, translation, actions, session)
        {
            self.translate_dialog_session = None;
            tracing::error!("Failed to open translate dialog: {error}");
            self.resume_clipboard_capture();
        }
    }

    pub(in crate::app) fn handle_translate_dialog_closed(&mut self, session: u64) {
        if self.translate_dialog_session != Some(session) {
            return;
        }
        self.translate_dialog_session = None;
        self.resume_clipboard_capture();
    }

    /// 백로그 대화상자 열기
    pub(in crate::app) fn open_backlog_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let store = self.model.backlog.clone();
        let actions = self.action_sender();
        Self::open_dialog_generic("backlog", || BacklogDialog::show(main_hwnd, store, actions));
    }

    /// 파일 번역 대화상자 열기
    pub(in crate::app) fn open_file_trans_dialog(&mut self) {
        if let Some(session) = FileTransDialog::current_session() {
            self.file_trans_dialog_session = Some(session);
            if !self.pause_clipboard_capture("파일 번역 창") {
                return;
            }
            let main_hwnd = self.hwnd;
            let config = self.model.config.clone();
            let supervisor = self.services.file_translation.clone();
            let actions = self.action_sender();
            Self::open_dialog_generic("file_trans", || {
                FileTransDialog::show(main_hwnd, config, supervisor, actions, session)
            });
            return;
        }
        self.file_trans_dialog_session = None;

        if !self.pause_clipboard_capture("파일 번역 창") {
            return;
        }
        let session = self.next_clipboard_pause_session();
        self.file_trans_dialog_session = Some(session);

        let main_hwnd = self.hwnd;
        let config = self.model.config.clone();
        let supervisor = self.services.file_translation.clone();
        let actions = self.action_sender();
        if let Err(error) = FileTransDialog::show(main_hwnd, config, supervisor, actions, session) {
            self.file_trans_dialog_session = None;
            tracing::error!("Failed to open file_trans dialog: {error}");
            self.resume_clipboard_capture();
        }
    }

    pub(in crate::app) fn handle_file_trans_dialog_closed(&mut self, session: u64) {
        if self.file_trans_dialog_session != Some(session) {
            return;
        }
        self.file_trans_dialog_session = None;
        self.resume_clipboard_capture();
    }
}
