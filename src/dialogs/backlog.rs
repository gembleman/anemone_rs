//! 백로그 대화상자
//!
//! RichEdit 기반 이력 뷰어.
//! 원문/번역 필터링 및 파일 저장 지원.

use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::*,
        System::LibraryLoader::{GetModuleHandleW, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
        UI::Controls::RichEdit::{
            CFE_BOLD, CFE_ITALIC, CFM_BOLD, CFM_COLOR, CFM_FACE, CFM_ITALIC, CFM_SIZE,
            CHARFORMAT2W, EM_SETBKGNDCOLOR, EM_SETCHARFORMAT, SCF_SELECTION,
        },
        UI::Controls::*,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::file_dialog::{FileFilter, save_file};
use super::font::{FontDialog, FontDialogConfig, FontStyle};
use super::helpers::{
    center_dialog_on_monitor, register_resource_dialog, rescale_dialog_children_for_dpi,
    show_dialog_window, unregister_resource_dialog,
};
use crate::define_dialog_instance;
use crate::util::to_wide;

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

use crate::backlog::{BacklogFilter, BacklogStore, LogEntry, TextKind};

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
    store: Rc<RefCell<BacklogStore>>,
    /// RichEdit 본문에 적용 중인 폰트 패밀리
    font_face: Option<String>,
    /// 본문 포인트 크기 (pt). yHeight 는 twip 단위라 *20.
    font_point_size: i32,
    /// 본문 italic 여부 (bold 는 [name] 강조 용도라 별도 유지)
    font_italic: bool,
}

thread_local! {
    static RICHEDIT_LOADED: RefCell<bool> = const { RefCell::new(false) };
    static BACKLOG_PENDING: RefCell<Option<Rc<RefCell<BacklogStore>>>> = const { RefCell::new(None) };
    static BACKLOG_INIT_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

define_dialog_instance!(BACKLOG_INSTANCE: BacklogDialog);

unsafe extern "system" fn backlog_dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    unsafe {
        if msg == WM_INITDIALOG {
            let store = BACKLOG_PENDING.with(|slot| slot.borrow_mut().take());
            let Some(store) = store else {
                BACKLOG_INIT_ERROR.with(|slot| {
                    *slot.borrow_mut() = Some("백로그 창 초기화 인자가 없습니다".into());
                });
                return 0;
            };

            let dialog = Rc::new(RefCell::new(BacklogDialog::new(hwnd, store)));
            BACKLOG_INSTANCE.with(|slot| *slot.borrow_mut() = Some(dialog.clone()));
            if let Err(error) = dialog.borrow_mut().initialize_controls() {
                BACKLOG_INSTANCE.with(|slot| {
                    slot.borrow_mut().take();
                });
                BACKLOG_INIT_ERROR.with(|slot| {
                    *slot.borrow_mut() = Some(error.to_string());
                });
                return 0;
            }
            register_resource_dialog(hwnd);
            return 1;
        }

        let instance = BACKLOG_INSTANCE.with(|slot| {
            let Ok(guard) = slot.try_borrow() else {
                return None;
            };
            guard.clone()
        });
        let Some(dialog) = instance else {
            return 0;
        };

        match msg {
            WM_SIZE => {
                if let Ok(dialog) = dialog.try_borrow() {
                    dialog.on_size(
                        (lparam.0 & 0xFFFF) as i32,
                        ((lparam.0 >> 16) & 0xFFFF) as i32,
                    );
                }
                1
            }
            WM_DPICHANGED => {
                if let Ok(mut dialog) = dialog.try_borrow_mut() {
                    dialog.handle_dpi_changed(wparam, lparam);
                }
                1
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as u16;
                if id == IDCANCEL.0 as u16 {
                    let _ = DestroyWindow(hwnd);
                } else if let Ok(mut dialog) = dialog.try_borrow_mut() {
                    dialog.handle_command(id);
                }
                1
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                1
            }
            WM_DESTROY => {
                unregister_resource_dialog(hwnd);
                BACKLOG_INSTANCE.with(|slot| {
                    if let Ok(mut guard) = slot.try_borrow_mut() {
                        *guard = None;
                    }
                });
                1
            }
            _ => 0,
        }
    }
}

impl BacklogDialog {
    fn new(hwnd: HWND, store: Rc<RefCell<BacklogStore>>) -> Self {
        Self {
            hwnd,
            richedit: HWND::default(),
            group_filter: HWND::default(),
            group_action: HWND::default(),
            applied_dpi: crate::dpi::dpi_for_window(hwnd),
            filter: BacklogFilter::All,
            add_linefeed: true,
            store,
            font_face: None,
            font_point_size: 10,
            font_italic: false,
        }
    }

    pub fn show(parent: HWND, store: Rc<RefCell<BacklogStore>>) -> Result<HWND> {
        let existing =
            BACKLOG_INSTANCE.with(|slot| slot.borrow().as_ref().map(|dialog| dialog.borrow().hwnd));
        if let Some(hwnd) = existing
            && unsafe { IsWindow(Some(hwnd)).as_bool() }
        {
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
            return Ok(hwnd);
        }

        RICHEDIT_LOADED.with(|loaded| -> Result<()> {
            if !*loaded.borrow() {
                unsafe {
                    LoadLibraryExW(w!("Msftedit.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)?;
                }
                *loaded.borrow_mut() = true;
            }
            Ok(())
        })?;

        let instance = unsafe { GetModuleHandleW(None)? };
        BACKLOG_INIT_ERROR.with(|slot| {
            slot.borrow_mut().take();
        });
        BACKLOG_PENDING.with(|slot| *slot.borrow_mut() = Some(store));
        let result = unsafe {
            CreateDialogParamW(
                Some(instance.into()),
                PCWSTR(ctrl_id::DIALOG as usize as *const u16),
                Some(parent),
                Some(backlog_dialog_proc),
                LPARAM(0),
            )
        };
        let hwnd = match result {
            Ok(hwnd) => hwnd,
            Err(error) => {
                BACKLOG_PENDING.with(|slot| {
                    slot.borrow_mut().take();
                });
                return Err(error);
            }
        };
        if let Some(message) = BACKLOG_INIT_ERROR.with(|slot| slot.borrow_mut().take()) {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(Error::new(E_FAIL, message));
        }

        unsafe {
            center_dialog_on_monitor(hwnd, parent);
            show_dialog_window(hwnd);
        }
        Ok(hwnd)
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
    }

    fn handle_dpi_changed(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let new_dpi = (wparam.0 & 0xFFFF) as u32;
        rescale_dialog_children_for_dpi(self.hwnd, self.applied_dpi, new_dpi);
        self.applied_dpi = new_dpi;
        if lparam.0 != 0 {
            unsafe {
                let rect = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }

    /// RichEdit에 항목 추가
    fn append_styled_texts_to_richedit(
        &self,
        segments: impl IntoIterator<Item = crate::backlog::StyledText>,
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
        self.store.borrow_mut().clear();
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
        let segments = self.store.borrow().render(self.filter, self.add_linefeed);
        self.append_styled_texts_to_richedit(segments);
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
                let message = to_wide(&format!("저장 대화상자를 열 수 없습니다.\n{error}"));
                unsafe {
                    let _ = MessageBoxW(
                        Some(self.hwnd),
                        PCWSTR(message.as_ptr()),
                        w!("오류"),
                        MB_ICONERROR,
                    );
                }
                return;
            }
        };

        if let Err(error) = self.store.borrow().export_utf8(&path) {
            tracing::error!("backlog save failed: {error}");
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

/// 저장소에 항목을 추가하고, 백로그 창이 열려 있으면 즉시 화면에도 반영한다.
pub fn add_to_backlog(store: &Rc<RefCell<BacklogStore>>, entry: LogEntry) {
    let entry_for_render = entry.clone();
    store.borrow_mut().push(entry);
    BACKLOG_INSTANCE.with(|cell| {
        let Ok(guard) = cell.try_borrow() else {
            return;
        };
        if let Some(ref dialog) = *guard
            && let Ok(d) = dialog.try_borrow()
        {
            d.append_styled_texts_to_richedit(BacklogStore::render_entry(
                &entry_for_render,
                d.filter,
                d.add_linefeed,
            ));
        }
    });
}
