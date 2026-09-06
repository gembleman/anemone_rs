//! 백로그 대화상자
//!
//! RichEdit 기반 이력 뷰어.
//! 원문/번역 필터링 및 파일 저장 지원.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

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

    fn can_defer(msg: u32) -> bool {
        matches!(msg, WM_SIZE | WM_COMMAND)
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
        if model_evicted
            || view_evicted
            || had_pending
            || BACKLOG_VIEW_DIRTY.with(|dirty| dirty.replace(false))
        {
            dialog.refresh_richedit();
        } else {
            dialog.append_styled_texts_to_richedit(BacklogStore::render_entry(
                &entry_for_render,
                dialog.filter,
                dialog.add_linefeed,
            ));
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
