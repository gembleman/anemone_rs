//! 수동 후크 설치, 후보 검색과 후보 목록에서의 설치.

use windows_sys::Win32::UI::WindowsAndMessaging::{LB_RESETCONTENT, SendMessageW};

use crate::hook::HookRequest;

use super::install::{HookInstallError, queue_hook_replacement};
use super::view::{add_list_string, candidate_label, list_selection, read_control_text};
use super::{HookFindDialog, PENDING_CANDIDATES, ctrl_id, set_enabled};
use crate::dialogs::helpers::show_error_message;

pub(super) const MAX_PENDING_CANDIDATES: usize = 512;
const MAX_CANDIDATES: usize = 512;
// lunahook의 SearchParam.search_time_ms(30초)가 끝난 뒤 pipe에 남은 결과가
// 정리될 시간을 둔다. 이 기간에는 새 검색을 보내지 않아 이전 결과가 새
// 세대로 잘못 붙는 경합을 막는다.
const SEARCH_IN_FLIGHT_GRACE_MS: u64 = 1_000;

pub(super) fn candidate_generation_matches(active: u64, incoming: u64) -> bool {
    active == incoming
}

impl HookFindDialog {
    pub(super) fn install_manual_hook(&mut self) {
        let code = read_control_text(self.hwnd, ctrl_id::MANUAL_CODE);
        let code = code.trim().to_string();
        let wide: Vec<u16> = code.encode_utf16().collect();
        let Some(mut hook_param) = lunahook_rs::hookcode::parse(&wide) else {
            show_error_message(
                self.hwnd,
                "후크 코드 오류",
                "LunaHook 형식의 올바른 후크 코드를 입력해 주세요.",
            );
            return;
        };
        hook_param.name[..6].copy_from_slice(b"UserUI");
        let previous = self.installed_manual;
        let address = hook_param.address;
        match queue_hook_replacement(Box::new(hook_param), previous, |request| {
            self.hook.request(request).map_err(|_| ())
        }) {
            Ok(address) => {
                self.installed_manual = Some(address);
            }
            Err(HookInstallError::InstallRequest) => show_error_message(
                self.hwnd,
                "후크 설치 실패",
                "후킹 워커가 이미 종료되었습니다.",
            ),
            Err(HookInstallError::RemoveRequest) => {
                self.installed_manual = Some(address);
                show_error_message(
                    self.hwnd,
                    "기존 후크 정리 실패",
                    "새 수동 후크 설치 요청은 전달됐지만 기존 후크 제거 요청을 보내지 못했습니다.",
                );
            }
        }
    }

    pub(super) fn drain_candidates(&mut self) {
        let incoming = PENDING_CANDIDATES.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
        for (generation, found) in incoming {
            if !candidate_generation_matches(self.search_generation, generation) {
                continue;
            }
            if self.candidates.len() >= MAX_CANDIDATES {
                break;
            }
            if self
                .candidates
                .iter()
                .any(|old| old.hook_address == found.hook_address)
            {
                continue;
            }
            add_list_string(self.candidates_list, &candidate_label(&found));
            self.candidates.push(found);
        }
        set_enabled(
            self.candidate_install_btn,
            self.target_label.is_some() && !self.candidates.is_empty(),
        );
    }

    pub(super) fn clear_candidates(&mut self) {
        PENDING_CANDIDATES.with(|slot| slot.borrow_mut().clear());
        self.candidates.clear();
        unsafe {
            let _ = SendMessageW(self.candidates_list, LB_RESETCONTENT, 0, 0);
            set_enabled(self.candidate_install_btn, false);
        }
    }

    pub(super) fn start_text_search(&mut self) {
        let text = read_control_text(self.hwnd, ctrl_id::SEARCH_TEXT);
        if text.trim().is_empty() {
            self.start_auto_search();
            return;
        }
        let Some(generation) = self.next_search_generation() else {
            return;
        };
        self.clear_candidates();
        self.send_search(
            crate::hook::pipe_client::build_text_search_param(&text),
            generation,
        );
    }

    pub(super) fn start_auto_search(&mut self) {
        let Some(generation) = self.next_search_generation() else {
            return;
        };
        self.clear_candidates();
        self.send_search(
            crate::hook::pipe_client::build_general_search_param(),
            generation,
        );
    }

    fn next_search_generation(&mut self) -> Option<u64> {
        if self
            .search_in_flight_until
            .is_some_and(|until| std::time::Instant::now() < until)
        {
            tracing::debug!("후크 검색이 진행 중이어서 중복 검색을 무시합니다");
            return None;
        }
        self.search_generation = self.search_generation.wrapping_add(1);
        // 새 검색이 시작되면 이전 세대의 대기 결과도 즉시 비운다.
        PENDING_CANDIDATES.with(|slot| slot.borrow_mut().clear());
        self.search_in_flight_until = Some(
            std::time::Instant::now()
                + std::time::Duration::from_millis(30_000 + SEARCH_IN_FLIGHT_GRACE_MS),
        );
        Some(self.search_generation)
    }

    fn send_search(&self, search: lunahook_rs::protocol::SearchParam, generation: u64) {
        if self
            .hook
            .request(HookRequest::FindHook {
                search: Box::new(search),
                generation,
            })
            .is_err()
        {
            show_error_message(
                self.hwnd,
                "후크 검색 실패",
                "후킹 워커가 이미 종료되었습니다.",
            );
        }
    }

    pub(super) fn install_candidate(&mut self) {
        let Some(found) = list_selection(self.candidates_list)
            .and_then(|index| self.candidates.get(index))
            .cloned()
        else {
            return;
        };

        let mut hook_param = found.hook_param;
        hook_param.name[..6].copy_from_slice(b"UserUI");

        let previous = self.installed_candidate;
        match queue_hook_replacement(Box::new(hook_param), previous, |request| {
            self.hook.request(request).map_err(|_| ())
        }) {
            Ok(address) => {
                self.installed_candidate = Some(address);
            }
            Err(HookInstallError::InstallRequest) => show_error_message(
                self.hwnd,
                "후크 설치 실패",
                "후킹 워커가 이미 종료되었습니다. 기존 후크는 유지됩니다.",
            ),
            Err(HookInstallError::RemoveRequest) => {
                self.installed_candidate = Some(found.hook_address);
                show_error_message(
                    self.hwnd,
                    "기존 후크 정리 실패",
                    "새 후보 후크 설치 요청은 전달됐지만 기존 후크 제거 요청을 보내지 못했습니다.",
                );
            }
        }
    }
}
