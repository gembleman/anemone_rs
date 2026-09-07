//! 백로그 대화상자
//!
//! RichEdit 기반 이력 뷰어.
//! 원문/번역 필터링 및 파일 저장 지원.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows_core::{Error, HRESULT};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::{LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
    UI::WindowsAndMessaging::*,
};

use super::host::{DialogHost, DialogResult, HostedDialog};
use crate::app::action::AppActionSender;
type Result<T> = windows_core::Result<T>;

mod actions;
mod format;
mod richedit;

// 컨트롤 ID
mod ctrl_id {
    pub const DIALOG: u16 = 106;
    pub const RICHEDIT: u16 = 3001;
    pub const CHK_LINEFEED: u16 = 3002;
    pub const RADIO_ORIGINAL: u16 = 3010;
    pub const RADIO_TRANSLATION: u16 = 3011;
    pub const RADIO_ALL: u16 = 3012;
    pub const BTN_CLEAR: u16 = 3020;
    pub const BTN_SAVE: u16 = 3021;
    pub const BTN_FONT: u16 = 3022;
    pub const GROUP_FILTER: u16 = 3030;
    pub const GROUP_ACTION: u16 = 3031;
}

use crate::app::backlog::{BacklogFilter, BacklogStore, LogEntry};

pub(super) const WM_BACKLOG_SAVE_RESULT: u32 = WM_APP + 0x36;
static NEXT_SAVE_TOKEN: AtomicU64 = AtomicU64::new(1);

struct SaveResult {
    result: std::result::Result<(), crate::app::backlog::BacklogExportError>,
}

/// 백로그 대화상자
pub struct BacklogDialog {
    hwnd: HWND,
    richedit: HWND,
    /// 필터 그룹박스 핸들 — `WM_SIZE` 재배치용.
    group_filter: HWND,
    /// 동작 그룹박스 핸들 — `WM_SIZE` 재배치용.
    group_action: HWND,
    applied_dpi: u32,
    filter: BacklogFilter,
    add_linefeed: bool,
    store: BacklogStore,
    actions: AppActionSender,
    /// RichEdit 본문에 적용 중인 폰트 패밀리
    font_face: Option<String>,
    /// 본문 포인트 크기 (pt). yHeight 는 twip 단위라 *20.
    font_point_size: i32,
    /// 본문 italic 여부 (bold 는 [name] 강조 용도라 별도 유지)
    font_italic: bool,
    /// 현재 RichEdit에 표시한 항목별 UTF-16 길이.
    rendered_lengths: VecDeque<usize>,
    save_in_progress: bool,
    save_result: Arc<Mutex<Option<SaveResult>>>,
    save_worker: Option<JoinHandle<()>>,
    save_token: u64,
}

thread_local! {
    static RICHEDIT_LOADED: RefCell<bool> = const { RefCell::new(false) };
    static BACKLOG_VIEW_DIRTY: Cell<bool> = const { Cell::new(false) };
    /// 모달 공통 대화상자의 중첩 message loop에서 state를 빌리지 못했을 때도
    /// 열린 view snapshot에 나중에 반영할 항목.
    static PENDING_BACKLOG_ENTRIES: RefCell<VecDeque<LogEntry>> =
        const { RefCell::new(VecDeque::new()) };
}

pub(crate) struct BacklogInit {
    store: BacklogStore,
    actions: AppActionSender,
}

impl HostedDialog for BacklogDialog {
    type Init = BacklogInit;
    const RESOURCE_ID: u16 = ctrl_id::DIALOG;

    fn create(hwnd: HWND, init: Self::Init) -> Result<Self> {
        // 새 snapshot은 AppModel에서 복제되므로 이전 창 수명의 pending 항목을
        // 다시 넣으면 중복된다.
        PENDING_BACKLOG_ENTRIES.with(|pending| pending.borrow_mut().clear());
        BACKLOG_VIEW_DIRTY.with(|dirty| dirty.set(false));
        let mut dialog = Self::new(hwnd, init.store, init.actions);
        dialog.initialize_controls()?;
        Ok(dialog)
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> DialogResult {
        match msg {
            WM_BACKLOG_SAVE_RESULT if wparam as u64 == self.save_token => {
                self.handle_save_result();
                DialogResult::Handled(1)
            }
            WM_SIZE => {
                self.on_size((lparam & 0xFFFF) as i32, ((lparam >> 16) & 0xFFFF) as i32);
                DialogResult::Handled(1)
            }
            WM_COMMAND => {
                let id = (wparam & 0xFFFF) as u16;
                if id == IDCANCEL as u16 {
                    return DialogResult::Close(1);
                }
                self.handle_command(id);
                DialogResult::Handled(1)
            }
            _ => DialogResult::Unhandled,
        }
    }

    fn applied_dpi(&mut self) -> Option<&mut u32> {
        Some(&mut self.applied_dpi)
    }

    fn destroy(&mut self) {
        // 저장 중인 파일은 백그라운드에서 끝내도록 두고, 이미 끝난 스레드만
        // 회수한다. 창이 사라진 뒤 결과 알림은 운영체제가 버린다.
        self.save_token = 0;
        if let Some(worker) = self.save_worker.take() {
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                // 활성 저장 워커는 파일 쓰기를 끝내도록 JoinHandle을 의도적으로
                // 버린다. UI 종료에서 파일 I/O를 기다리지 않는다.
                drop(worker);
            }
        }
    }

    fn can_defer(msg: u32) -> bool {
        matches!(msg, WM_SIZE | WM_COMMAND | WM_BACKLOG_SAVE_RESULT)
    }
}

impl BacklogDialog {
    fn new(hwnd: HWND, store: BacklogStore, actions: AppActionSender) -> Self {
        Self {
            hwnd,
            richedit: HWND::default(),
            group_filter: HWND::default(),
            group_action: HWND::default(),
            applied_dpi: crate::dpi::dpi_for_window(hwnd),
            filter: BacklogFilter::All,
            add_linefeed: true,
            store,
            actions,
            font_face: None,
            font_point_size: 10,
            font_italic: false,
            rendered_lengths: VecDeque::new(),
            save_in_progress: false,
            save_result: Arc::new(Mutex::new(None)),
            save_worker: None,
            save_token: 0,
        }
    }

    pub fn show(parent: HWND, store: BacklogStore, actions: AppActionSender) -> Result<HWND> {
        // RichEdit 컨트롤은 창을 만들기 전에 클래스가 등록되어 있어야 한다.
        RICHEDIT_LOADED.with(|loaded| -> Result<()> {
            if !*loaded.borrow() {
                unsafe {
                    let module = LoadLibraryExW(
                        crate::win32::to_wide("Msftedit.dll").as_ptr(),
                        std::ptr::null_mut(),
                        LOAD_LIBRARY_SEARCH_SYSTEM32,
                    );
                    if module.is_null() {
                        return Err(Error::new(
                            HRESULT(E_FAIL),
                            "Msftedit.dll을 로드할 수 없습니다",
                        ));
                    }
                }
                *loaded.borrow_mut() = true;
            }
            Ok(())
        })?;

        DialogHost::<Self>::show(parent, BacklogInit { store, actions })
    }

    fn handle_save_result(&mut self) {
        let result = self
            .save_result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(result) = result else { return };
        if let Some(worker) = self.save_worker.take() {
            let _ = worker.join();
        }
        self.save_in_progress = false;
        if let Err(error) = result.result {
            tracing::error!("backlog save failed: {error}");
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "백로그 저장 오류",
                &format!("백로그를 파일에 저장하지 못했습니다.\n\n{error}"),
            );
        }
    }
}

/// AppModel이 소유한 저장소와 별개인 열린 view snapshot에 새 항목을 반영한다.
pub(crate) fn append_entry(entry: LogEntry, model_evicted: bool) {
    if DialogHost::<BacklogDialog>::current_hwnd().is_none() {
        return;
    }
    let entry_for_render = entry.clone();
    let applied = DialogHost::<BacklogDialog>::with_state_mut(|dialog| {
        let had_pending = drain_pending_entries(&mut dialog.store);
        let view_evicted = dialog.store.push(entry);
        let dirty = BACKLOG_VIEW_DIRTY.with(|dirty| dirty.replace(false));
        if model_evicted || view_evicted || had_pending || dirty {
            // 보류 항목이나 필터 변경이 있으면 길이 정보가 맞지 않으므로
            // 전체를 한 번만 다시 만든다. 일반적인 상한 퇴출은 앞부분만 지운다.
            if !had_pending && !dirty {
                let mut removed_chars = 0usize;
                while dialog.rendered_lengths.len() > dialog.store.len() {
                    if let Some(chars) = dialog.rendered_lengths.pop_front() {
                        removed_chars = removed_chars.saturating_add(chars);
                    }
                }
                dialog.remove_prefix_from_richedit(removed_chars);
                let rendered = BacklogStore::render_entry(
                    &entry_for_render,
                    dialog.filter,
                    dialog.add_linefeed,
                );
                let chars = rendered
                    .iter()
                    .map(|part| part.text.encode_utf16().count())
                    .sum();
                dialog.append_styled_texts_to_richedit(rendered);
                dialog.rendered_lengths.push_back(chars);
            } else {
                dialog.refresh_richedit();
                dialog.rendered_lengths = dialog
                    .store
                    .render_entry_lengths(dialog.filter, dialog.add_linefeed)
                    .into_iter()
                    .collect();
            }
        } else {
            let rendered =
                BacklogStore::render_entry(&entry_for_render, dialog.filter, dialog.add_linefeed);
            let chars = rendered
                .iter()
                .map(|part| part.text.encode_utf16().count())
                .sum();
            dialog.append_styled_texts_to_richedit(rendered);
            dialog.rendered_lengths.push_back(chars);
        }
    });
    // 재진입으로 state를 빌리지 못했다. 항목 자체를 보존한 뒤 바깥 handler가
    // 돌아왔을 때 snapshot에 넣고 전체를 다시 그린다.
    if applied.is_none() {
        PENDING_BACKLOG_ENTRIES.with(|pending| pending.borrow_mut().push_back(entry_for_render));
        BACKLOG_VIEW_DIRTY.with(|dirty| dirty.set(true));
    }
}

fn drain_pending_entries(store: &mut BacklogStore) -> bool {
    let pending =
        PENDING_BACKLOG_ENTRIES.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    let had_pending = !pending.is_empty();
    for entry in pending {
        store.push(entry);
    }
    had_pending
}

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/backlog/mod.rs"]
mod tests;
