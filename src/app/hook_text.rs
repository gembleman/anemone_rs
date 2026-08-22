//! 후킹 워커 이벤트 소비 → 텍스트 병합 → 기존 번역 파이프라인 연결.
//!
//! 텍스트 흐름은 클립보드 경로(`translation.rs::handle_clipboard_change`)와
//! 같은 모양이다: 워커가 보낸 문장을 짧은 디바운스 창에 모았다가(갱신형
//! 엔진의 홍수 방지), 타이머가 만료되면 마지막 값만 `request_translation_async`
//! 으로 넘긴다. 번역 완료/표시/backlog는 기존 경로를 그대로 재사용한다.

use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use super::{App, HOOK_MERGE_TIMER};
use crate::hook::{HookEvent, text_bridge::HookText};

impl App {
    /// WM_APP_HOOK_STATE — 후킹 워커의 이벤트를 모두 소비한다.
    pub(super) fn handle_hook_state(&mut self) {
        for event in self.services.hook.drain_events() {
            match event {
                HookEvent::Attached {
                    pid,
                    arch,
                    process_name,
                } => self.handle_hook_attached(pid, arch, process_name),
                HookEvent::AttachFailed { pid, error } => {
                    tracing::error!(pid, "후킹 연결 실패: {error}");
                    crate::dialogs::helpers::show_error_message(
                        self.hwnd,
                        "후킹 연결 실패",
                        &error,
                    );
                }
                HookEvent::Detached { pid, by_user } => {
                    tracing::debug!(pid, by_user, "후킹 세션 종료 처리");
                    self.handle_hook_detached(by_user);
                }
                HookEvent::Text(hook_text) => self.submit_hook_text(hook_text),
                HookEvent::FoundHook(found) => {
                    // 후보 목록 UI(후크 찾기 다이얼로그)가 열려 있으면 전달한다.
                    crate::dialogs::hook_find::add_candidate(found);
                }
                HookEvent::Info(message) => {
                    tracing::info!(target: "hook_dll", "{message}");
                }
            }
        }
    }

    fn handle_hook_attached(&mut self, pid: u32, arch: crate::hook::Arch, process_name: String) {
        tracing::info!(pid, arch = arch.label(), %process_name, "게임 후킹 시작");
        self.model.runtime.hook_session = Some(super::state::HookSessionInfo {
            pid,
            process_name: process_name.clone(),
            arch_label: arch.label().to_string(),
        });
        if let Err(error) = self.model.config.save() {
            tracing::warn!("마지막 후킹 대상 저장 실패: {error}");
        }
    }

    fn handle_hook_detached(&mut self, by_user: bool) {
        if !by_user {
            tracing::info!("게임과의 후킹 세션이 끊겼습니다 (프로세스 종료 추정)");
        }
        self.clear_hook_pending();
        self.model.runtime.hook_session = None;
    }

    /// 워커로부터 받은 문장을 병합 창에 적립하고 타이머를 건다.
    fn submit_hook_text(&mut self, hook_text: HookText) {
        let hook_config = &self.model.config.hook;
        if !hook_config.enabled {
            return;
        }
        if hook_config.max_text_length > 0
            && hook_text.text.chars().count() > hook_config.max_text_length as usize
        {
            tracing::debug!(
                len = hook_text.text.chars().count(),
                "hook text skipped: exceeds max_text_length"
            );
            return;
        }

        self.model.runtime.hook_merger.submit(hook_text);
        unsafe {
            let _ = KillTimer(Some(self.hwnd), HOOK_MERGE_TIMER);
        }
        let delay = self.model.config.hook.merge_window_ms.max(1);
        let timer = unsafe { SetTimer(Some(self.hwnd), HOOK_MERGE_TIMER, delay, None) };
        if timer == 0 {
            tracing::warn!("후킹 병합 타이머를 만들지 못했습니다; 즉시 번역합니다");
            self.flush_hook_texts();
        }
    }

    /// 병합 창이 닫혔을 때 마지막 값들을 번역 요청으로 넘긴다.
    pub(super) fn handle_hook_merge_timer(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), HOOK_MERGE_TIMER);
        }
        self.flush_hook_texts();
    }

    fn flush_hook_texts(&mut self) {
        let ready = self.model.runtime.hook_merger.drain_all();
        // 단일 게임 정책이라 활성 출처는 사실상 하나다. 여러 개여도 화면에는
        // 마지막 것이 남는다(요청 supersede).
        if let Some(latest) = ready.last() {
            self.request_translation_async(&latest.text);
        }
    }

    /// 세션 종료 시 대기 중인 미번역 텍스트를 버린다.
    pub(super) fn clear_hook_pending(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::KillTimer as kill;
        unsafe {
            let _ = kill(Some(self.hwnd), HOOK_MERGE_TIMER);
        }
        self.model.runtime.hook_merger.drain_all();
    }
}
