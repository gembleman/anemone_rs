//! 백로그 대화상자
//!
//! RichEdit 기반 이력 뷰어.
//! 원문/번역 필터링 및 파일 저장 지원.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use windows::{
    Win32::{
        Foundation::*,
        System::LibraryLoader::{LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
        UI::Controls::RichEdit::{
            CFE_BOLD, CFE_ITALIC, CFM_BOLD, CFM_COLOR, CFM_FACE, CFM_ITALIC, CFM_SIZE,
            CHARFORMAT2W, EM_EXLIMITTEXT, EM_SETBKGNDCOLOR, EM_SETCHARFORMAT, SCF_SELECTION,
        },
        UI::Controls::*,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::file_dialog::{FileFilter, save_file};
use super::font::{FontDialog, FontDialogConfig, FontStyle};
use super::host::{DialogHost, DialogResult, HostedDialog};
use crate::app::action::AppActionSender;
use crate::win32::to_wide;

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

use crate::app::backlog::{
    BacklogFilter, BacklogStore, LogEntry, MAX_BACKLOG_ENTRIES, MAX_BACKLOG_TEXT_BYTES, TextKind,
};

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
                self.on_size(
                    (lparam.0 & 0xFFFF) as i32,
                    ((lparam.0 >> 16) & 0xFFFF) as i32,
                );
                DialogResult::Handled(LRESULT(1))
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as u16;
                if id == IDCANCEL.0 as u16 {
                    return DialogResult::Close(LRESULT(1));
                }
                self.handle_command(id);
                DialogResult::Handled(LRESULT(1))
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
                    LoadLibraryExW(w!("Msftedit.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)?;
                }
                *loaded.borrow_mut() = true;
            }
            Ok(())
        })?;

        DialogHost::<Self>::show(parent, BacklogInit { store, actions })
    }

    fn initialize_controls(&mut self) -> Result<()> {
        let get_control = |id| {
            unsafe { GetDlgItem(Some(self.hwnd), id) }.map_err(|_| {
                Error::new(E_FAIL, format!("백로그 컨트롤 ID {id}를 찾을 수 없습니다"))
            })
        };
        self.richedit = get_control(ctrl_id::RICHEDIT as i32)?;
        self.group_filter = get_control(ctrl_id::GROUP_FILTER as i32)?;
        self.group_action = get_control(ctrl_id::GROUP_ACTION as i32)?;
        for id in [
            ctrl_id::CHK_LINEFEED,
            ctrl_id::RADIO_ORIGINAL,
            ctrl_id::RADIO_TRANSLATION,
            ctrl_id::RADIO_ALL,
            ctrl_id::BTN_CLEAR,
            ctrl_id::BTN_SAVE,
            ctrl_id::BTN_FONT,
        ] {
            get_control(id as i32)?;
        }
        unsafe {
            let _ = CheckDlgButton(self.hwnd, ctrl_id::RADIO_ALL as i32, BST_CHECKED);
            let _ = CheckDlgButton(self.hwnd, ctrl_id::CHK_LINEFEED as i32, BST_CHECKED);
            let _ = SendMessageW(
                self.richedit,
                EM_SETBKGNDCOLOR,
                Some(WPARAM(1)),
                Some(LPARAM(0)),
            );
            // Store 상한에 포맷 label/newline 여유를 더한 명시적 문자 제한.
            let display_limit =
                MAX_BACKLOG_TEXT_BYTES.saturating_add(MAX_BACKLOG_ENTRIES.saturating_mul(128));
            let _ = SendMessageW(
                self.richedit,
                EM_EXLIMITTEXT,
                Some(WPARAM(0)),
                Some(LPARAM(display_limit as isize)),
            );
        }
        self.refresh_richedit();
        Ok(())
    }

    fn handle_command(&mut self, cmd: u16) {
        use ctrl_id::*;
        match cmd {
            CHK_LINEFEED => {
                self.add_linefeed = !self.add_linefeed;
                self.refresh_richedit();
            }
            RADIO_ORIGINAL => {
                self.filter = BacklogFilter::Original;
                self.refresh_richedit();
            }
            RADIO_TRANSLATION => {
                self.filter = BacklogFilter::Translation;
                self.refresh_richedit();
            }
            RADIO_ALL => {
                self.filter = BacklogFilter::All;
                self.refresh_richedit();
            }
            BTN_CLEAR => self.clear_richedit(),
            BTN_SAVE => self.save_to_file(),
            BTN_FONT => self.choose_font(),
            _ => {}
        }
        self.refresh_if_dirty();
    }

    /// RichEdit에 항목 추가
    fn append_styled_texts_to_richedit(
        &self,
        segments: impl IntoIterator<Item = crate::app::backlog::StyledText>,
    ) {
        // SAFETY: self.richedit is a valid RichEdit control handle from create_controls.
        // SendMessageW and append_styled_text use valid control handles.
        unsafe {
            let _ = SendMessageW(
                self.richedit,
                EM_SETSEL,
                Some(WPARAM(usize::MAX)),
                Some(LPARAM(-1)),
            );

            // COLORREF 는 0x00BBGGRR 순서.
            const COLOR_NAME: u32 = 0x00A00000; // #0000A0 진청색 ([name])
            const COLOR_ORIGINAL: u32 = 0x00000000; // #000000 검정 (원문)
            const COLOR_TRANSLATE: u32 = 0x00008000; // #008000 진녹색 (번역)

            for segment in segments {
                let (color, bold) = match segment.kind {
                    TextKind::Name => (COLOR_NAME, true),
                    TextKind::Original => (COLOR_ORIGINAL, false),
                    TextKind::Translation => (COLOR_TRANSLATE, false),
                };
                self.append_styled_text(&segment.text, color, bold);
            }

            let _ = SendMessageW(
                self.richedit,
                EM_SCROLLCARET,
                Some(WPARAM(0)),
                Some(LPARAM(0)),
            );
        }
    }

    /// 스타일 텍스트 추가
    unsafe fn append_styled_text(&self, text: &str, color: u32, bold: bool) {
        // SAFETY: self.richedit is a valid RichEdit control. CHARFORMAT2W::default() zeroes
        // the struct; we then fill in cbSize and the masked fields. The pointer cast to
        // isize for LPARAM is valid because the struct lives on the stack for the call.
        unsafe {
            let mut cf = CHARFORMAT2W::default();
            cf.Base.cbSize = std::mem::size_of::<CHARFORMAT2W>() as u32;
            cf.Base.dwMask = CFM_COLOR | CFM_SIZE | CFM_BOLD | CFM_ITALIC;
            cf.Base.crTextColor = COLORREF(color);
            cf.Base.yHeight = self.font_point_size.max(1) * 20; // pt → twip

            if bold {
                cf.Base.dwEffects |= CFE_BOLD;
            }
            if self.font_italic {
                cf.Base.dwEffects |= CFE_ITALIC;
            }

            if let Some(ref face) = self.font_face {
                cf.Base.dwMask |= CFM_FACE;
                let face_utf16: Vec<u16> = face.encode_utf16().collect();
                let copy_len = face_utf16.len().min(cf.Base.szFaceName.len() - 1);
                cf.Base.szFaceName[..copy_len].copy_from_slice(&face_utf16[..copy_len]);
            }

            let _ = SendMessageW(
                self.richedit,
                EM_SETCHARFORMAT,
                Some(WPARAM(SCF_SELECTION as usize)),
                Some(LPARAM(&cf as *const _ as isize)),
            );

            let wide = to_wide(text);
            let _ = SendMessageW(
                self.richedit,
                EM_REPLACESEL,
                Some(WPARAM(0)),
                Some(LPARAM(wide.as_ptr() as isize)),
            );
        }
    }

    /// RichEdit 내용 지우기
    fn clear_richedit(&mut self) {
        self.store.clear();
        self.actions.clear_backlog();
        // SAFETY: self.richedit is a valid RichEdit control handle.
        unsafe {
            let _ = SetWindowTextW(self.richedit, w!(""));
        }
    }

    /// RichEdit 다시 그리기 (필터 변경 시)
    fn refresh_richedit(&self) {
        // SAFETY: self.richedit is a valid RichEdit control handle.
        unsafe {
            let _ = SetWindowTextW(self.richedit, w!(""));
        }
        let segments = self.store.render(self.filter, self.add_linefeed);
        self.append_styled_texts_to_richedit(segments);
    }

    fn refresh_if_dirty(&mut self) {
        if BACKLOG_VIEW_DIRTY.with(|dirty| dirty.replace(false)) {
            drain_pending_entries(&mut self.store);
            self.refresh_richedit();
        }
    }

    /// 폰트 선택 대화상자
    fn choose_font(&mut self) {
        let cfg = FontDialogConfig {
            initial_face: self.font_face.clone(),
            initial_style: FontStyle {
                bold: false,
                italic: self.font_italic,
            },
            initial_point_size: self.font_point_size,
            no_activate: false,
        };

        let Some(result) = FontDialog::show(self.hwnd, cfg) else {
            return;
        };

        self.font_face = Some(result.face_name);
        self.font_italic = result.style.italic;
        if result.point_size > 0 {
            self.font_point_size = result.point_size;
        }
        self.refresh_richedit();
    }

    /// 파일로 저장
    fn save_to_file(&self) {
        let filters = [
            FileFilter {
                name: "텍스트 파일 (*.txt)",
                spec: "*.txt",
            },
            FileFilter {
                name: "모든 파일 (*.*)",
                spec: "*.*",
            },
        ];
        let path = match save_file(self.hwnd, "백로그 저장", &filters, Some("txt"), None) {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                tracing::error!("백로그 저장 대화상자 오류: {error}");
                let message = HSTRING::from(format!("저장 대화상자를 열 수 없습니다.\n{error}"));
                unsafe {
                    let _ = MessageBoxW(Some(self.hwnd), &message, w!("오류"), MB_ICONERROR);
                }
                return;
            }
        };

        if let Err(error) = self.store.export_utf8(&path) {
            tracing::error!("backlog save failed: {error}");
            super::helpers::show_error_message(
                self.hwnd,
                "백로그 저장 오류",
                &format!("백로그를 파일에 저장하지 못했습니다.\n\n{error}"),
            );
        }
    }

    /// 윈도우 크기 변경 시 컨트롤 재배치
    ///
    /// RichEdit는 새 client size에 맞춰 늘리고 하단 그룹은 아래쪽에 고정한다.
    fn on_size(&self, width: i32, height: i32) {
        unsafe {
            let dpi = crate::dpi::dpi_for_window(self.hwnd);
            let s = |v: i32| crate::dpi::scale(v, dpi);
            let margin = s(10);
            let group_y = height - s(100);
            let filter_width = (width - s(240)).max(s(250));
            let action_x = width - s(220);

            let _ = SetWindowPos(
                self.richedit,
                None,
                margin,
                margin,
                (width - margin * 2).max(1),
                (height - s(120)).max(1),
                SWP_NOZORDER,
            );

            let move_to = |ctrl: HWND, x: i32, y: i32, w: i32, h: i32| {
                let _ = SetWindowPos(ctrl, None, x, y, w, h, SWP_NOZORDER);
            };
            let move_ctrl = |id: u16, x: i32, y: i32, w: i32, h: i32| {
                if let Ok(ctrl) = GetDlgItem(Some(self.hwnd), id as i32) {
                    move_to(ctrl, x, y, w, h);
                }
            };

            move_to(self.group_filter, margin, group_y, filter_width, s(60));
            move_to(self.group_action, action_x, group_y, s(210), s(60));

            use ctrl_id::*;
            let option_y = group_y + s(20);
            move_ctrl(RADIO_ORIGINAL, margin + s(10), option_y, s(80), s(20));
            move_ctrl(RADIO_TRANSLATION, margin + s(95), option_y, s(80), s(20));
            move_ctrl(RADIO_ALL, margin + s(180), option_y, s(60), s(20));
            move_ctrl(
                CHK_LINEFEED,
                margin + filter_width - s(100),
                option_y,
                s(90),
                s(20),
            );
            let button_y = group_y + s(22);
            move_ctrl(BTN_CLEAR, action_x + s(10), button_y, s(55), s(28));
            move_ctrl(BTN_SAVE, action_x + s(75), button_y, s(55), s(28));
            move_ctrl(BTN_FONT, action_x + s(140), button_y, s(55), s(28));
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
mod tests {
    use super::*;

    #[test]
    fn deferred_entries_are_preserved_in_the_open_view_snapshot() {
        PENDING_BACKLOG_ENTRIES.with(|pending| {
            let mut pending = pending.borrow_mut();
            pending.clear();
            pending.push_back(LogEntry::new("first".into()).with_translation("첫째".into()));
            pending.push_back(LogEntry::new("second".into()).with_translation("둘째".into()));
        });

        let mut store = BacklogStore::new();
        assert!(drain_pending_entries(&mut store));
        let rendered = store
            .render(BacklogFilter::All, true)
            .into_iter()
            .map(|segment| segment.text)
            .collect::<String>();

        assert!(rendered.contains("first"));
        assert!(rendered.contains("첫째"));
        assert!(rendered.contains("second"));
        assert!(rendered.contains("둘째"));
        assert!(!drain_pending_entries(&mut store));
    }
}
