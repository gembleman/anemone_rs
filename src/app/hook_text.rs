//! 후킹 워커 이벤트 소비 → 텍스트 병합 → 기존 번역 파이프라인 연결.
//!
//! 워커가 보낸 문자/조각을 원본 LunaHost의 TextThread처럼 출처별로 모았다가,
//! 마지막 조각 이후 병합 창이 지나면 관리 창과 `request_translation_async`로
//! 넘긴다. 번역 완료/표시/backlog는 기존 경로를 그대로 재사용한다.

use std::collections::VecDeque;

use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use super::{App, HOOK_MERGE_TIMER, state};
use crate::hook::{HookEvent, text_bridge::HookText};

/// 병합 타이머 주기(ms). 원본 `TextThread::Start`의 `CreateTimerQueueTimer`가
/// 10ms다. `flushDelay`와 무관하게 고정이다 — 창 길이는 방출 조건이 쓰고,
/// 타이머는 그 조건을 얼마나 자주 확인할지만 정한다.
const MERGE_TIMER_PERIOD_MS: u32 = 10;

impl App {
    /// WM_APP_HOOK_STATE — 후킹 워커의 이벤트를 모두 소비한다.
    pub(super) fn handle_hook_state(&mut self) {
        for event in self.services.hook.drain_events() {
            match event {
                HookEvent::Attached {
                    pid,
                    arch,
                    process_name,
                    exe_sha256,
                } => {
                    let target_label = format!("{} (PID {}, {})", process_name, pid, arch.label());
                    self.handle_hook_attached(pid, arch, process_name, exe_sha256);
                    crate::dialogs::hook_find::notify_attached(target_label);
                }
                HookEvent::AttachFailed { pid, error } => {
                    tracing::error!(pid, "후킹 연결 실패: {error}");
                    // 새 연결 요청을 처리하기 전에 워커가 기존 세션을 정리한다.
                    // 실패 이벤트만 도착한 경우에도 앱 스냅샷을 반드시 비운다.
                    self.handle_hook_detached(true);
                    crate::dialogs::hook_find::notify_attach_failed();
                    crate::dialogs::helpers::show_error_message(
                        self.hwnd,
                        "후킹 연결 실패",
                        &error,
                    );
                }
                HookEvent::Detached { pid, by_user } => {
                    tracing::debug!(pid, by_user, "후킹 세션 종료 처리");
                    self.handle_hook_detached(by_user);
                    crate::dialogs::hook_find::notify_detached();
                }
                HookEvent::Text { text, received_at } => {
                    self.submit_hook_text(text, received_at);
                }
                HookEvent::EngineDetected(engine_name) => {
                    self.handle_engine_detected(engine_name);
                }
                HookEvent::FoundHook(found) => {
                    // 후보 목록이 열려 있으면 후킹 관리 창에 전달한다.
                    crate::dialogs::hook_find::add_candidate(*found);
                }
                HookEvent::HookInserted { address, hook_code } => {
                    crate::dialogs::hook_find::note_hook_inserted(address, hook_code);
                }
                HookEvent::Info(message) => {
                    tracing::info!(target: "hook_dll", "{message}");
                }
            }
        }
    }

    fn handle_hook_attached(
        &mut self,
        pid: u32,
        arch: crate::hook::Arch,
        process_name: String,
        exe_sha256: Option<String>,
    ) {
        crate::dialogs::hook_find::reset_session();
        tracing::info!(pid, arch = arch.label(), %process_name, "게임 후킹 시작");
        let saved_profile = self
            .model
            .config
            .hook
            .saved_profile(&process_name, exe_sha256.as_deref())
            .cloned();
        self.model.runtime.hook_session = Some(super::state::HookSessionInfo {
            pid,
            process_name: process_name.clone(),
            exe_sha256: exe_sha256.clone(),
            arch_label: arch.label().to_string(),
            detected_engines: Vec::new(),
        });
        crate::hook::set_session_active(true);
        self.start_hook_merge_timer();
        // 번역 요청 출처 입증용 신원. 해시는 워커가 attach 중 계산해 온다.
        crate::hook::set_session_identity(Some(crate::hook::ProcessIdentity {
            pid,
            name: process_name.clone(),
            sha256: exe_sha256,
        }));
        // 후킹된 동안에는 게임 텍스트만 번역 파이프라인에 흘린다. 사용자가 다른
        // 창에서 복사한 텍스트가 후킹 문장을 덮어쓰거나, 그 게임의 신원으로
        // 태깅되어 유료 엔진에 나가는 일을 막는다. detach 때 다시 켠다.
        self.pause_clipboard_capture("게임 후킹");
        // 후킹 연결 자체가 후킹 번역 사용 의사다. 스레드를 선택하면 텍스트가
        // 흐르도록 기능을 켜고, 다음 실행에서 쓸 대상 정보도 함께 보존한다.
        self.model.config.hook.enabled = true;
        self.model.config.hook.last_target_pid = pid;
        self.model.config.hook.last_target_name = process_name;
        if !self.services.config_save.request(self.model.config.clone()) {
            tracing::warn!("마지막 후킹 대상 저장 요청 실패");
        }

        if let Some(profile) = saved_profile {
            let auto_hook_name = if profile.hook_code.is_some() {
                "UserUI".to_string()
            } else {
                profile.hook_name.clone()
            };
            crate::dialogs::hook_find::prepare_auto_select(auto_hook_name);
            if let Some(code) = profile.hook_code {
                let wide: Vec<u16> = code.encode_utf16().collect();
                if let Some(mut hook_param) = lunahook_rs::hookcode::parse(&wide) {
                    hook_param.name[..6].copy_from_slice(b"UserUI");
                    if self
                        .services
                        .hook
                        .request(crate::hook::HookRequest::NewHook(Box::new(hook_param)))
                        .is_err()
                    {
                        tracing::warn!("저장된 후크 자동 설치 실패: 후킹 워커 종료됨");
                    }
                } else {
                    tracing::warn!("저장된 후크 코드를 해석할 수 없어 자동 설치를 건너뜁니다");
                }
            }
        }
    }

    fn handle_engine_detected(&mut self, engine_name: String) {
        let Some(session) = self.model.runtime.hook_session.as_mut() else {
            return;
        };
        if !session
            .detected_engines
            .iter()
            .any(|known| known.eq_ignore_ascii_case(&engine_name))
        {
            session.detected_engines.push(engine_name);
        }
    }

    fn handle_hook_detached(&mut self, by_user: bool) {
        if !by_user {
            tracing::info!("게임과의 후킹 세션이 끊겼습니다 (프로세스 종료 추정)");
        }
        self.clear_hook_pending();
        crate::dialogs::hook_find::reset_session();
        // `[overlay-layout]`은 직전 줄과 같으면 남기지 않는다. 세션을 넘겨
        // 비교하면 다시 붙었을 때 레이아웃이 같다는 이유로 새 세션의 첫 줄이
        // 통째로 눌린다 — 진단이 가장 필요한 지점이다.
        self.last_render_diagnostic = None;
        self.model.runtime.hook_session = None;
        crate::hook::set_session_active(false);
        crate::hook::set_session_identity(None);
        // 세션 스냅샷을 비운 뒤에 재개해야 일시정지 판정이 해제된 상태로 읽힌다.
        self.resume_clipboard_capture();
    }

    /// 워커로부터 받은 문장을 병합 창에 적립한다.
    ///
    /// 방출은 훅 종류를 가리지 않고 타이머가 한다 — 원본 `TextThread::Flush`와
    /// 같다. 문장을 통째로 내는 훅을 이 자리에서 바로 내보내면, 같은 대사의
    /// 다음 줄이 창 안에 들어와도 합쳐질 기회가 없다.
    fn submit_hook_text(&mut self, hook_text: HookText, received_at: std::time::Instant) {
        let ready = self
            .model
            .runtime
            .hook_merger
            .submit_at(hook_text, received_at);
        self.dispatch_hook_texts(ready);
        if self.hook_merge_timer_active {
            return;
        }
        // 타이머를 만들지 못한 세션. 방출할 것이 없으니 도착하는 대로 낸다.
        let ready = self.model.runtime.hook_merger.drain_all();
        self.dispatch_hook_texts(ready);
    }

    /// 세션이 붙는 순간 병합 타이머를 시작한다 — 원본 `TextThread::Start`.
    ///
    /// 원본은 TextThread를 만들 때 10ms 주기 타이머를 걸고 스레드가 사라질 때
    /// 끝낸다. 버퍼가 비었는지로 켜고 끄지 않는다. 도착 이벤트마다 `SetTimer`를
    /// 다시 부르면 같은 ID의 countdown이 매번 리셋되어, 글자 단위 훅처럼 쉬지
    /// 않고 오는 출처가 있으면 타이머가 영영 안 터진다.
    fn start_hook_merge_timer(&mut self) {
        if self.hook_merge_timer_active {
            return;
        }
        let timer = unsafe { SetTimer(self.hwnd, HOOK_MERGE_TIMER, MERGE_TIMER_PERIOD_MS, None) };
        if timer == 0 {
            tracing::warn!("후킹 병합 타이머를 만들지 못했습니다; 문장을 즉시 번역합니다");
            return;
        }
        self.hook_merge_timer_active = true;
    }

    /// 원본 `TextThread::Stop`. 세션이 끝날 때만 부른다.
    fn stop_hook_merge_timer(&mut self) {
        if !self.hook_merge_timer_active {
            return;
        }
        unsafe {
            let _ = KillTimer(self.hwnd, HOOK_MERGE_TIMER);
        }
        self.hook_merge_timer_active = false;
    }

    /// 주기 tick마다 마지막 조각 이후 병합 창이 지난 문장만 방출한다.
    pub(super) fn handle_hook_merge_timer(&mut self) {
        let ready = self.model.runtime.hook_merger.flush_expired();
        self.dispatch_hook_texts(ready);
    }

    fn dispatch_hook_texts(&mut self, mut ready: Vec<HookText>) {
        let kirikiri_engine = self
            .model
            .runtime
            .hook_session
            .as_ref()
            .and_then(|session| {
                session
                    .detected_engines
                    .iter()
                    .find(|engine| crate::hook::text_bridge::is_kirikiri_engine(engine))
                    .map(String::as_str)
            });
        if let Some(engine_name) = kirikiri_engine {
            for text in &mut ready {
                crate::hook::text_bridge::postprocess_hook_text(engine_name, &mut text.text);
            }
        }

        // 관리 창에는 필터링 전의 모든 스레드에서 완성된 문장을 보여 준다.
        for text in &ready {
            crate::dialogs::hook_find::observe_text(text);
        }

        if !self.model.config.hook.enabled {
            return;
        }

        // 단일 게임 정책이라 활성 출처는 사실상 하나다. 선택 변경과 timer tick
        // 사이의 경합을 막기 위해 방출 직전에 다시 거른다.
        let max_len = self.model.config.hook.max_text_length;
        for text in ready
            .into_iter()
            .filter(|text| crate::dialogs::hook_find::accepts(text.source))
        {
            tracing::info!(
                target: crate::logging::LUNAHOOK_TARGET,
                "[merged @{:x} ctx={:x} ctx2={:x} {}]{}",
                text.source.address,
                text.source.context,
                text.source.subcontext,
                text.hook_name,
                text.text
            );
            if max_len > 0 && text.text.chars().count() > max_len as usize {
                tracing::debug!(
                    len = text.text.chars().count(),
                    "hook text skipped: exceeds max_text_length"
                );
                continue;
            }
            let queued = state::QueuedHookText {
                text: text.text,
                full_string: text.full_string,
            };
            if let Some(dropped) = queue_hook_text(&mut self.model.runtime.hook_text_queue, queued)
            {
                tracing::debug!(
                    dropped = dropped.text,
                    "후킹 문장 대기열이 가득 차 앞 문장을 버립니다"
                );
            }
        }
        self.pump_hook_translation();
    }

    /// 대기 중인 후킹 문장을 번역에 보낸다.
    ///
    /// 워커는 같은 창의 새 요청이 오면 앞 요청을 취소한다. 문장이 연속으로
    /// 나오는 동안 그대로 보내면 앞 문장이 번역되지 않고 사라지므로, 앞 요청이
    /// 끝날 때까지 모았다가 한 번에 보낸다. 모인 문장은 병합 창에서와 같이
    /// 줄바꿈으로 잇는다.
    pub(super) fn pump_hook_translation(&mut self) {
        if self.model.runtime.hook_text_queue.is_empty() {
            return;
        }
        let pending_age = self
            .model
            .runtime
            .pending_translation
            .as_ref()
            .map(state::PendingTranslation::age);
        if state::waits_for_pending_translation(pending_age) {
            return;
        }

        if let Some(joined) = take_queued_hook_text(&mut self.model.runtime.hook_text_queue) {
            tracing::info!(
                target: crate::logging::LUNAHOOK_TARGET,
                "[translate-request]{}",
                joined
            );
            self.request_translation_async(&joined);
        }
    }

    /// 세션 종료 시 대기 중인 미번역 텍스트를 버린다.
    pub(super) fn clear_hook_pending(&mut self) {
        self.stop_hook_merge_timer();
        self.model.runtime.hook_merger.drain_all();
        self.model.runtime.hook_text_queue.clear();
    }
}

/// 문장을 대기열에 넣는다. 가득 찼으면 버린 가장 오래된 문장을 돌려준다.
fn queue_hook_text(
    queue: &mut VecDeque<state::QueuedHookText>,
    text: state::QueuedHookText,
) -> Option<state::QueuedHookText> {
    let dropped = (queue.len() >= state::MAX_QUEUED_HOOK_TEXTS)
        .then(|| queue.pop_front())
        .flatten();
    queue.push_back(text);
    dropped
}

/// 대기열을 비우고 한 요청으로 합친다.
///
/// 문장을 통째로 내는 훅에서 온 것끼리는 병합 창에서와 같이 줄을 나눈다. 글자
/// 단위 훅에서 온 조각은 한 문장이 창을 넘겨 끊긴 것이므로 그대로 잇는다 —
/// `ふふふ`처럼 글자 간격이 벌어지는 구간에서 문장이 쪼개진다.
fn take_queued_hook_text(queue: &mut VecDeque<state::QueuedHookText>) -> Option<String> {
    let mut joined = String::new();
    for piece in queue.drain(..) {
        if piece.full_string && !joined.is_empty() {
            joined.push('\n');
        }
        joined.push_str(&piece.text);
    }
    (!joined.is_empty()).then_some(joined)
}

#[cfg(test)]
#[path = "../../tests/unit/app/hook_text.rs"]
mod tests;
