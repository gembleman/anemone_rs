//! 번역 탭에서 선택된 엔진의 전용 패널만 보이도록 하는 컨트롤 그룹과,
//! 사용량 라벨 서식.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows_sys::Win32::{
    Foundation::HWND,
    UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled},
    UI::WindowsAndMessaging::*,
};

use super::{SettingsDialog, TAB_TRANSLATION, ctrl_id};
use crate::translation::{TranslationEngine, lang_utils};
use crate::win32::to_wide;

pub(super) const WM_MYS_USAGE_RESULT: u32 = WM_APP + 0x33;

type UsageRequest = (String, String);
type UsageResult = Option<crate::translation::mys_usage::UsageSnapshot>;
type UsageResultSlot = Arc<Mutex<Vec<UsageResult>>>;

/// MyS 사용량을 UI 스레드 밖에서 조회한다.
pub(super) struct MysUsageWorker {
    sender: Mutex<Option<Sender<UsageRequest>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    results: UsageResultSlot,
    active: Arc<AtomicBool>,
}

impl MysUsageWorker {
    pub(super) fn spawn(hwnd: HWND) -> Option<Self> {
        let (tx, rx) = mpsc::channel::<UsageRequest>();
        let results: UsageResultSlot = Arc::new(Mutex::new(Vec::new()));
        let worker_results = Arc::clone(&results);
        let active = Arc::new(AtomicBool::new(true));
        let worker_active = Arc::clone(&active);
        let hwnd_raw = hwnd as usize;
        let handle = std::thread::Builder::new()
            .name("anemone-mys-usage".to_string())
            .spawn(move || Self::worker_thread(rx, worker_results, worker_active, hwnd_raw))
            .map_err(|error| {
                tracing::error!("MyS 사용량 워커를 시작하지 못했습니다: {error}");
            })
            .ok()?;
        Some(Self {
            sender: Mutex::new(Some(tx)),
            handle: Mutex::new(Some(handle)),
            results,
            active,
        })
    }

    pub(super) fn request(&self, url: String, token: String) -> bool {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        sender
            .as_ref()
            .is_some_and(|tx| tx.send((url, token)).is_ok())
    }

    pub(super) fn drain_results(&self) -> Vec<UsageResult> {
        let mut results = self
            .results
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *results)
    }

    /// 채널을 닫는다. 진행 중인 DB 조회는 기다리지 않는다.
    pub(super) fn shutdown(&self) {
        self.active.store(false, Ordering::Release);
        *self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        let handle = self
            .handle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(handle) = handle
            && handle.is_finished()
        {
            let _ = handle.join();
        }
    }

    fn worker_thread(
        rx: Receiver<UsageRequest>,
        results: UsageResultSlot,
        active: Arc<AtomicBool>,
        hwnd_raw: usize,
    ) {
        while let Ok((url, token)) = rx.recv() {
            let result = crate::translation::mys_usage::snapshot(&url, &token);
            if !active.load(Ordering::Acquire) {
                break;
            }
            results
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(result);
            // SAFETY: 창이 닫혔으면 PostMessageW가 실패하고 반환한다.
            if unsafe { PostMessageW(hwnd_raw as HWND, WM_MYS_USAGE_RESULT, 0, 0) } == 0 {
                tracing::warn!("MyS 사용량 결과 알림을 게시하지 못했습니다");
            }
        }
    }
}

/// 선택된 엔진 패널만 표시하기 위한 컨트롤 그룹.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EngineGroup {
    EzTrans = 0,
    DeepL = 1,
    Papago = 2,
    Llm = 3,
    Custom = 4,
    MysTranslater = 5,
}

fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(character);
    }
    formatted
}

pub(super) fn format_mys_usage(
    snapshot: Option<crate::translation::mys_usage::UsageSnapshot>,
) -> String {
    let Some(usage) = snapshot else {
        return "이번 달 사용량: 아직 없음".to_string();
    };
    format!(
        "이번 달: 신규 {}토큰 (입력 {} / 출력 {})\r\n캐시 {}자 · 처리 {}건",
        format_count(usage.fresh_total_tokens()),
        format_count(usage.fresh_prompt_tokens),
        format_count(usage.fresh_output_tokens),
        format_count(usage.cached_characters),
        format_count(usage.total_count()),
    )
}

impl SettingsDialog {
    pub(super) fn engine_group(engine: TranslationEngine) -> Option<EngineGroup> {
        match engine {
            TranslationEngine::EzTrans => Some(EngineGroup::EzTrans),
            TranslationEngine::DeepL => Some(EngineGroup::DeepL),
            TranslationEngine::Papago => Some(EngineGroup::Papago),
            TranslationEngine::Llm => Some(EngineGroup::Llm),
            TranslationEngine::Custom => Some(EngineGroup::Custom),
            TranslationEngine::MysTranslater => Some(EngineGroup::MysTranslater),
            TranslationEngine::Google => None,
        }
    }

    /// 번역 탭에서는 선택된 엔진의 전용 컨트롤만 표시한다.
    pub(super) fn update_engine_controls(&self, engine: TranslationEngine) {
        let active = Self::engine_group(engine).map(|group| group as usize);
        let translation_tab_visible = self.current_tab == TAB_TRANSLATION;
        let free_token_button = self.control(ctrl_id::MYS_TRANSLATER_FREE_TOKEN_BTN).ok();
        let usage_refresh_button = self.control(ctrl_id::MYS_TRANSLATER_USAGE_REFRESH_BTN).ok();
        let signup_in_progress = self.mys_signup_in_progress.get();
        let usage_refresh_in_progress = self.mys_usage_refresh_in_progress.get();
        // SAFETY: HWNDs in engine_controls are valid child controls.
        unsafe {
            for (idx, group) in self.engine_controls.iter().enumerate() {
                let visible = translation_tab_visible && active == Some(idx);
                for &h in group {
                    let blocked_by_signup = signup_in_progress && Some(h) == free_token_button;
                    let blocked_by_usage =
                        usage_refresh_in_progress && Some(h) == usage_refresh_button;
                    let enabled = visible && !blocked_by_signup && !blocked_by_usage;
                    if (IsWindowEnabled(h) != 0) != enabled {
                        let _ = EnableWindow(h, if enabled { 1 } else { 0 });
                    }
                    if (IsWindowVisible(h) != 0) != visible {
                        let _ = ShowWindow(h, if visible { SW_SHOW } else { SW_HIDE });
                    }
                }
            }
        }
        if translation_tab_visible && engine == TranslationEngine::MysTranslater {
            self.refresh_mys_token_status();
        }
        if translation_tab_visible && engine == TranslationEngine::EzTrans {
            self.request_eztrans_path_validation();
        }
    }

    pub(super) fn refresh_mys_token_status(&self) {
        let configured = !self
            .draft
            .borrow()
            .translation
            .mys_translater_api_key
            .trim()
            .is_empty();
        self.set_control_text(
            ctrl_id::MYS_TRANSLATER_TOKEN_STATUS_LABEL,
            if configured {
                "발급됨 (앱이 보관합니다)"
            } else {
                "없음 — [무료 API key 받기]를 눌러 주세요"
            },
        );
    }

    pub(super) fn handle_mys_usage_refresh_button(&mut self) {
        if self.mys_usage_refresh_in_progress.get() {
            return;
        }
        let (url, token) = {
            let draft = self.draft.borrow();
            (
                draft.translation.mys_translater_url.clone(),
                draft.translation.mys_translater_api_key.clone(),
            )
        };
        let accepted = {
            let mut slot = self.mys_usage_worker.borrow_mut();
            if slot.is_none() {
                *slot = MysUsageWorker::spawn(self.hwnd);
            }
            slot.as_ref()
                .is_some_and(|worker| worker.request(url, token))
        };
        if !accepted {
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "사용량 새로고침",
                "사용량 조회를 시작할 수 없습니다.",
            );
            return;
        }
        self.set_control_text(
            ctrl_id::MYS_TRANSLATER_USAGE_LABEL,
            "이번 달 사용량: 조회 중...",
        );
        self.set_mys_usage_refresh_in_progress(true);
    }

    pub(super) fn handle_mys_usage_result(&mut self) {
        let results = {
            let slot = self.mys_usage_worker.borrow();
            match slot.as_ref() {
                Some(worker) => worker.drain_results(),
                None => return,
            }
        };
        let Some(result) = results.last().copied() else {
            return;
        };
        self.set_mys_usage_refresh_in_progress(false);
        self.set_control_text(
            ctrl_id::MYS_TRANSLATER_USAGE_LABEL,
            &format_mys_usage(result),
        );
    }

    fn set_mys_usage_refresh_in_progress(&self, in_progress: bool) {
        self.mys_usage_refresh_in_progress.set(in_progress);
        if let Ok(engine) = self.draft.borrow().translation.get_engine() {
            self.update_engine_controls(engine);
        }
    }

    /// 현재 엔진에 맞춰 소스/타겟 언어 콤보 항목을 갱신
    pub(super) fn refresh_language_combos(&self, engine: TranslationEngine) {
        // SAFETY: self.hwnd is valid; GetDlgItem returns valid combobox handles.
        unsafe {
            let src = GetDlgItem(self.hwnd, ctrl_id::TRANS_SOURCE_LANG as i32);
            let tgt = GetDlgItem(self.hwnd, ctrl_id::TRANS_TARGET_LANG as i32);
            if src.is_null() || tgt.is_null() {
                return;
            }

            let _ = SendMessageW(src, CB_RESETCONTENT, 0, 0);
            for &lang in engine.supported_source_languages() {
                let w = to_wide(lang_utils::to_korean_name(lang));
                let _ = SendMessageW(src, CB_ADDSTRING, 0, w.as_ptr() as isize);
            }
            let src_sel = match self.draft.borrow().translation.source_lang_index(engine) {
                Ok(index) => index,
                Err(error) => {
                    tracing::error!("번역 언어 설정 오류: {error}");
                    return;
                }
            };
            let _ = SendMessageW(src, CB_SETCURSEL, src_sel, 0);

            let source = engine
                .supported_source_languages()
                .get(src_sel)
                .copied()
                .or_else(|| engine.supported_source_languages().first().copied());
            let Some(source) = source else {
                return;
            };
            let targets = engine.supported_targets_for(source);

            let _ = SendMessageW(tgt, CB_RESETCONTENT, 0, 0);
            for &lang in &targets {
                let w = to_wide(lang_utils::to_korean_name(lang));
                let _ = SendMessageW(tgt, CB_ADDSTRING, 0, w.as_ptr() as isize);
            }
            let configured_target = self.draft.borrow().translation.get_target_language().ok();
            let tgt_sel = configured_target
                .and_then(|target| targets.iter().position(|&language| language == target))
                .unwrap_or(0);
            let _ = SendMessageW(tgt, CB_SETCURSEL, tgt_sel, 0);
        }
    }

    /// 엔진별 패널, 언어 콤보, 번역 탭 높이를 함께 갱신한다.
    pub(super) fn apply_engine_state(&mut self, engine: TranslationEngine) {
        self.resize_translation_group(engine);
        self.update_engine_controls(engine);
        self.refresh_language_combos(engine);
        if self.current_tab == TAB_TRANSLATION {
            self.adjust_dialog_size_for_tab(TAB_TRANSLATION);
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/settings/engine_panel.rs"]
mod tests;
