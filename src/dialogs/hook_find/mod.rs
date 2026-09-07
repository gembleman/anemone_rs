//! LunaHost식 후킹 관리 창.
//!
//! 대상 프로세스 선택부터 활성 텍스트 스레드 선택/미리보기, 수동 후크 코드
//! 설치, 자동·문장 검색까지 한 창에서 제공한다. 상태는 App과 같은 UI
//! 스레드에서만 접근한다.

mod connection;
mod install;
mod search;
mod streams;
mod view;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use windows_core::{Error, HRESULT};
use windows_sys::Win32::{
    Foundation::{E_FAIL, HWND, LPARAM, WPARAM},
    UI::Controls::EM_SETCUEBANNER,
    UI::Input::KeyboardAndMouse::EnableWindow,
    UI::WindowsAndMessaging::{
        GetDlgItem, LB_SETCURSEL, LBN_SELCHANGE, SendMessageW, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU,
    },
};

use super::host::{DialogHost, DialogResult, HostedDialog};
use crate::app::action::AppActionSender;
use crate::hook::HookWorker;
use crate::hook::pipe_client::FoundHook;
use crate::hook::process_list::ProcessEntry;
use crate::hook::text_bridge::{HookSource, HookText};
use view::{
    SavedView, StreamView, add_list_string, candidate_label, list_selection, read_control_text,
    set_control_text, set_log_text, stream_label, update_stream_indexed,
};
type Result<T> = windows_core::Result<T>;

fn dlg_item(hwnd: HWND, id: i32) -> Result<HWND> {
    let control = unsafe { GetDlgItem(hwnd, id) };
    if control.is_null() {
        Err(Error::new(
            HRESULT(E_FAIL),
            format!("후킹 대화상자 컨트롤 ID {id}를 찾을 수 없습니다"),
        ))
    } else {
        Ok(control)
    }
}

fn set_enabled(hwnd: HWND, enabled: bool) {
    unsafe {
        EnableWindow(hwnd, if enabled { 1 } else { 0 });
    }
}

mod ctrl_id {
    pub const DIALOG: u16 = 108;
    pub const TARGETS: u16 = 6001;
    pub const BTN_REFRESH: u16 = 6010;
    pub const BTN_CONNECT: u16 = 6011;
    pub const STATUS: u16 = 6100;
    pub const MANUAL_CODE: u16 = 6101;
    pub const BTN_MANUAL_INSTALL: u16 = 6110;
    pub const STREAMS: u16 = 6120;
    pub const PREVIEW: u16 = 6121;
    pub const BTN_STREAM_SELECT: u16 = 6122;
    pub const MERGE_WINDOW: u16 = 6123;
    pub const BTN_MERGE_WINDOW_APPLY: u16 = 6124;
    pub const SEARCH_TEXT: u16 = 6130;
    pub const CANDIDATES: u16 = 6131;
    pub const BTN_TEXT_SEARCH: u16 = 6140;
    pub const BTN_AUTO_SEARCH: u16 = 6141;
    pub const BTN_CANDIDATE_INSTALL: u16 = 6142;
    pub const BTN_DETACH: u16 = 6150;
    pub const GROUP_MANUAL: u16 = 6161;
    pub const GROUP_STREAMS: u16 = 6162;
    pub const GROUP_SEARCH: u16 = 6163;
}

const DISCONNECTED_TEXT: &str = "연결 안 됨";
const CONNECTING_TEXT: &str = "후킹 중....";
const DETACHING_TEXT: &str = "후킹 중지 중....";

thread_local! {
    static PENDING_CANDIDATES: RefCell<Vec<(u64, FoundHook)>> = const { RefCell::new(Vec::new()) };
    static SELECTED_SOURCE: Cell<Option<HookSource>> = const { Cell::new(None) };
    static AUTO_SELECT_HOOK_NAME: RefCell<Option<String>> = const { RefCell::new(None) };
    static INSTALLED_HOOK_CODES: RefCell<HashMap<u64, String>> = RefCell::new(HashMap::new());
    static SAVED_VIEW: RefCell<Option<SavedView>> = const { RefCell::new(None) };
}

/// 선택을 이 창의 상태와 후킹 워커가 읽는 스냅샷에 함께 기록한다.
///
/// 워커 스레드는 이 창의 thread_local을 볼 수 없어서 같은 값을 전역에도 둔다.
/// 두 곳이 어긋나면 워커가 엉뚱한 스레드의 텍스트를 디버그 로그에 남기므로
/// 선택을 바꾸는 자리는 반드시 이 함수를 거친다.
fn set_selected_source(source: Option<HookSource>) {
    SELECTED_SOURCE.with(|slot| slot.set(source));
    crate::hook::set_selected_text_source(source);
}

/// 새 세션에서는 이전 프로세스의 ThreadParam 선택을 재사용하지 않는다.
pub(crate) fn reset_session() {
    set_selected_source(None);
    AUTO_SELECT_HOOK_NAME.with(|slot| *slot.borrow_mut() = None);
    INSTALLED_HOOK_CODES.with(|slot| slot.borrow_mut().clear());
    PENDING_CANDIDATES.with(|slot| slot.borrow_mut().clear());
    SAVED_VIEW.with(|slot| *slot.borrow_mut() = None);
    DialogHost::<HookFindDialog>::with_state_mut(HookFindDialog::clear_session_view);
}

pub(crate) fn prepare_auto_select(hook_name: String) {
    AUTO_SELECT_HOOK_NAME.with(|slot| *slot.borrow_mut() = Some(hook_name));
}

pub(crate) fn note_hook_inserted(address: u64, hook_code: String) {
    if !hook_code.is_empty() {
        INSTALLED_HOOK_CODES.with(|codes| {
            codes.borrow_mut().insert(address, hook_code);
        });
    }
}

/// 사용자가 선택한 스레드만 번역 파이프라인으로 통과시킨다.
pub(crate) fn accepts(source: HookSource) -> bool {
    SELECTED_SOURCE.with(|slot| slot.get() == Some(source))
}

/// App UI 스레드에서 새 텍스트를 관리 창에 반영한다.
pub(crate) fn observe_text(text: &HookText) {
    if SELECTED_SOURCE.with(Cell::get).is_none() {
        let matches_saved = AUTO_SELECT_HOOK_NAME.with(|slot| {
            slot.borrow()
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(&text.hook_name))
        });
        if matches_saved {
            set_selected_source(Some(text.source));
            AUTO_SELECT_HOOK_NAME.with(|slot| *slot.borrow_mut() = None);
            tracing::info!(hook_name = %text.hook_name, "저장된 텍스트 스레드 자동 선택");
        }
    }
    if DialogHost::<HookFindDialog>::with_state_mut(|dialog| dialog.observe_text(text.clone()))
        .is_none()
    {
        SAVED_VIEW.with(|slot| {
            let mut slot = slot.borrow_mut();
            let view = slot.get_or_insert_with(SavedView::default);
            let _ =
                update_stream_indexed(&mut view.streams, &mut view.stream_indices, text.clone());
        });
    }
}

/// 발견된 후보를 적립하고, 창이 열려 있으면 즉시 표시한다.
pub(crate) fn add_candidate(generation: u64, found: FoundHook) {
    PENDING_CANDIDATES.with(|slot| {
        let mut pending = slot.borrow_mut();
        // 같은 주소의 중복 결과와 무제한 결과 누적을 막는다.
        if pending
            .iter()
            .any(|(_, old)| old.hook_address == found.hook_address)
            || pending.len() >= search::MAX_PENDING_CANDIDATES
        {
            return;
        }
        pending.push((generation, found));
    });
    DialogHost::<HookFindDialog>::with_state_mut(HookFindDialog::drain_candidates);
}

pub struct HookFindDialog {
    hwnd: HWND,
    targets_list: HWND,
    refresh_btn: HWND,
    connect_btn: HWND,
    status: HWND,
    detach_btn: HWND,
    streams_list: HWND,
    preview: HWND,
    stream_select_btn: HWND,
    candidates_list: HWND,
    candidate_install_btn: HWND,
    streams: Vec<StreamView>,
    stream_indices: HashMap<HookSource, usize>,
    candidates: Vec<FoundHook>,
    targets: Vec<ProcessEntry>,
    hook: Rc<HookWorker>,
    installed_candidate: Option<u64>,
    installed_manual: Option<u64>,
    target_label: Option<String>,
    actions: AppActionSender,
    merge_window_ms: u32,
    search_generation: u64,
    search_in_flight_until: Option<Instant>,
}

pub(crate) struct HookFindInit {
    hook: Rc<HookWorker>,
    target_label: Option<String>,
    actions: AppActionSender,
    merge_window_ms: u32,
}

impl HostedDialog for HookFindDialog {
    type Init = HookFindInit;
    const RESOURCE_ID: u16 = ctrl_id::DIALOG;

    fn create(hwnd: HWND, init: Self::Init) -> Result<Self> {
        let search_text = dlg_item(hwnd, ctrl_id::SEARCH_TEXT as i32)?;
        let search_cue = crate::win32::to_wide("게임 화면에 보이는 문장(비우면 자동 탐색)");
        unsafe {
            let _ = SendMessageW(
                search_text,
                EM_SETCUEBANNER,
                0,
                search_cue.as_ptr() as isize,
            );
        }

        let saved = SAVED_VIEW.with(|slot| slot.borrow_mut().take().unwrap_or_default());
        let mut dialog = Self {
            hwnd,
            targets_list: dlg_item(hwnd, ctrl_id::TARGETS as i32)?,
            refresh_btn: dlg_item(hwnd, ctrl_id::BTN_REFRESH as i32)?,
            connect_btn: dlg_item(hwnd, ctrl_id::BTN_CONNECT as i32)?,
            status: dlg_item(hwnd, ctrl_id::STATUS as i32)?,
            detach_btn: dlg_item(hwnd, ctrl_id::BTN_DETACH as i32)?,
            streams_list: dlg_item(hwnd, ctrl_id::STREAMS as i32)?,
            preview: dlg_item(hwnd, ctrl_id::PREVIEW as i32)?,
            stream_select_btn: dlg_item(hwnd, ctrl_id::BTN_STREAM_SELECT as i32)?,
            candidates_list: dlg_item(hwnd, ctrl_id::CANDIDATES as i32)?,
            candidate_install_btn: dlg_item(hwnd, ctrl_id::BTN_CANDIDATE_INSTALL as i32)?,
            streams: saved.streams,
            stream_indices: saved.stream_indices,
            candidates: saved.candidates,
            targets: Vec::new(),
            hook: init.hook,
            installed_candidate: saved.installed_candidate,
            installed_manual: saved.installed_manual,
            target_label: init.target_label,
            actions: init.actions,
            merge_window_ms: init.merge_window_ms,
            search_generation: 0,
            search_in_flight_until: None,
        };
        set_control_text(
            hwnd,
            ctrl_id::MERGE_WINDOW,
            &dialog.merge_window_ms.to_string(),
        );
        dialog.refresh_targets();
        dialog.restore_view(
            &saved.manual_code,
            &saved.search_text,
            saved.candidate_selection,
        );
        dialog.drain_candidates();
        dialog.sync_connection_ui();
        Ok(dialog)
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> DialogResult {
        match msg {
            WM_COMMAND => {
                let id = (wparam & 0xffff) as u16;
                let notification = ((wparam >> 16) & 0xffff) as u32;
                match id {
                    ctrl_id::BTN_REFRESH => self.refresh_targets(),
                    ctrl_id::BTN_CONNECT => self.connect_selected(),
                    ctrl_id::STREAMS if notification == LBN_SELCHANGE => {
                        self.preview_selected_stream()
                    }
                    ctrl_id::BTN_STREAM_SELECT => self.select_stream(),
                    ctrl_id::BTN_MERGE_WINDOW_APPLY => self.apply_merge_window(),
                    ctrl_id::BTN_MANUAL_INSTALL => self.install_manual_hook(),
                    ctrl_id::BTN_TEXT_SEARCH => self.start_text_search(),
                    ctrl_id::BTN_AUTO_SEARCH => self.start_auto_search(),
                    ctrl_id::BTN_CANDIDATE_INSTALL => self.install_candidate(),
                    ctrl_id::BTN_DETACH => self.detach(),
                    _ => {}
                }
                DialogResult::Handled(1)
            }
            WM_CLOSE => DialogResult::Close(1),
            _ => DialogResult::Unhandled,
        }
    }

    /// 텍스트 스레드 목록의 우클릭 메뉴는 state를 빌리기 전에 처리한다.
    /// `TrackPopupMenu`가 메시지를 다시 pump하는 동안 후킹 텍스트가 도착해도
    /// `observe_text`가 dialog state를 정상적으로 빌릴 수 있어야 하기 때문이다.
    fn handle_before_borrow(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<isize> {
        if msg != WM_CONTEXTMENU {
            return None;
        }
        // SAFETY: hwnd는 system이 넘긴 유효한 dialog handle이다.
        let streams_list = unsafe { GetDlgItem(hwnd, ctrl_id::STREAMS as i32) };
        if streams_list.is_null() || wparam != streams_list as WPARAM {
            return None;
        }
        streams::show_context_menu(hwnd, streams_list, lparam);
        Some(1)
    }

    fn destroy(&mut self) {
        let saved = SavedView {
            streams: std::mem::take(&mut self.streams),
            candidates: std::mem::take(&mut self.candidates),
            stream_indices: std::mem::take(&mut self.stream_indices),
            installed_candidate: self.installed_candidate,
            installed_manual: self.installed_manual,
            manual_code: read_control_text(self.hwnd, ctrl_id::MANUAL_CODE),
            search_text: read_control_text(self.hwnd, ctrl_id::SEARCH_TEXT),
            candidate_selection: list_selection(self.candidates_list),
        };
        SAVED_VIEW.with(|slot| *slot.borrow_mut() = Some(saved));
    }
}

impl HookFindDialog {
    pub(crate) fn show(
        parent: HWND,
        hook: Rc<HookWorker>,
        target_label: Option<String>,
        actions: AppActionSender,
        merge_window_ms: u32,
    ) -> Result<HWND> {
        DialogHost::<Self>::show(
            parent,
            HookFindInit {
                hook,
                target_label,
                actions,
                merge_window_ms,
            },
        )
    }

    /// 편집란의 값을 App에 넘긴다. 범위를 벗어난 값은 잘라서 되돌려 보여 준다.
    fn apply_merge_window(&mut self) {
        let raw = read_control_text(self.hwnd, ctrl_id::MERGE_WINDOW);
        let Ok(window_ms) = raw.trim().parse::<u32>() else {
            set_control_text(
                self.hwnd,
                ctrl_id::MERGE_WINDOW,
                &self.merge_window_ms.to_string(),
            );
            return;
        };
        let window_ms = window_ms.clamp(
            crate::config::MIN_MERGE_WINDOW_MS,
            crate::config::MAX_MERGE_WINDOW_MS,
        );
        self.merge_window_ms = window_ms;
        set_control_text(self.hwnd, ctrl_id::MERGE_WINDOW, &window_ms.to_string());
        self.actions.set_hook_merge_window(window_ms);
    }

    fn restore_view(
        &mut self,
        manual_code: &str,
        search_text: &str,
        candidate_selection: Option<usize>,
    ) {
        set_control_text(self.hwnd, ctrl_id::MANUAL_CODE, manual_code);
        set_control_text(self.hwnd, ctrl_id::SEARCH_TEXT, search_text);

        for stream in &self.streams {
            add_list_string(self.streams_list, &stream_label(stream));
        }
        for candidate in &self.candidates {
            add_list_string(self.candidates_list, &candidate_label(candidate));
        }

        if let Some(selected) = SELECTED_SOURCE.with(Cell::get)
            && let Some(index) = self
                .streams
                .iter()
                .position(|stream| stream.source == selected)
        {
            unsafe {
                let _ = SendMessageW(self.streams_list, LB_SETCURSEL, index, 0);
            }
            set_log_text(self.preview, &self.streams[index].history);
        }

        if let Some(index) = candidate_selection.filter(|index| *index < self.candidates.len()) {
            unsafe {
                let _ = SendMessageW(self.candidates_list, LB_SETCURSEL, index, 0);
            }
        }
    }
}

pub(crate) fn notify_attached(target_label: String) {
    DialogHost::<HookFindDialog>::with_state_mut(|dialog| dialog.attached(target_label));
}

pub(crate) fn notify_attach_failed() {
    DialogHost::<HookFindDialog>::with_state_mut(HookFindDialog::attach_failed);
}

pub(crate) fn notify_detaching() {
    DialogHost::<HookFindDialog>::with_state_mut(|dialog| {
        dialog.enter_pending_state(DETACHING_TEXT);
    });
}

pub(crate) fn notify_detached() {
    DialogHost::<HookFindDialog>::with_state_mut(HookFindDialog::detached);
}

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/hook_find/mod.rs"]
mod tests;
