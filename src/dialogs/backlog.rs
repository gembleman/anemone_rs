//! 백로그 대화상자
//!
//! RichEdit 기반 이력 뷰어.
//! 원문/번역 필터링 및 파일 저장 지원.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;

use windows::{
    Win32::{
        Foundation::*,
        System::LibraryLoader::{
            GetModuleHandleW, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
        },
        UI::Controls::*,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::config::Config;
use crate::constants::{
    CFE_BOLD, CFM_BOLD, CFM_COLOR, CFM_SIZE, EM_REPLACESEL, EM_SCROLLCARET, EM_SETBKGNDCOLOR,
    EM_SETCHARFORMAT, SCF_SELECTION,
};
use crate::impl_dialog;
use crate::util::to_wide;
use super::file_dialog::{FileFilter, save_file};
use super::helpers::DialogControls;

// 컨트롤 ID
mod ctrl_id {
    pub const RICHEDIT: u16 = 3001;
    pub const CHK_LINEFEED: u16 = 3002;
    pub const RADIO_ORIGINAL: u16 = 3010;
    pub const RADIO_TRANSLATION: u16 = 3011;
    pub const RADIO_ALL: u16 = 3012;
    pub const BTN_CLEAR: u16 = 3020;
    pub const BTN_SAVE: u16 = 3021;
    pub const BTN_FONT: u16 = 3022;
}

const BACKLOG_WIDTH: i32 = 600;
const BACKLOG_HEIGHT: i32 = 500;

/// 백로그 필터
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum BacklogFilter {
    Original,
    Translation,
    #[default]
    All,
}

/// 로그 항목
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub name: Option<String>,
    pub original: String,
    pub translation: Option<String>,
    pub timestamp: SystemTime,
}

impl LogEntry {
    pub fn new(original: String) -> Self {
        Self {
            name: None,
            original,
            translation: None,
            timestamp: SystemTime::now(),
        }
    }

    pub fn with_name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn with_translation(mut self, translation: String) -> Self {
        self.translation = Some(translation);
        self
    }
}

/// CHARFORMAT2W 구조체 (windows crate에서 직접 사용하기 어려워 수동 정의)
#[repr(C)]
#[derive(Clone, Copy)]
struct CHARFORMAT2W {
    cb_size: u32,
    dw_mask: u32,
    dw_effects: u32,
    y_height: i32,
    y_offset: i32,
    cr_text_color: u32,
    b_char_set: u8,
    b_pitch_and_family: u8,
    sz_face_name: [u16; 32],
    w_weight: u16,
    s_spacing: i16,
    cr_back_color: u32,
    lcid: u32,
    dw_reserved: u32,
    s_style: i16,
    w_kerning: u16,
    b_underline_type: u8,
    b_animation: u8,
    b_rev_author: u8,
    b_reserved1: u8,
}

impl Default for CHARFORMAT2W {
    fn default() -> Self {
        Self {
            cb_size: std::mem::size_of::<CHARFORMAT2W>() as u32,
            dw_mask: 0, dw_effects: 0, y_height: 0, y_offset: 0,
            cr_text_color: 0, b_char_set: 0, b_pitch_and_family: 0,
            sz_face_name: [0; 32], w_weight: 0, s_spacing: 0,
            cr_back_color: 0, lcid: 0, dw_reserved: 0,
            s_style: 0, w_kerning: 0, b_underline_type: 0,
            b_animation: 0, b_rev_author: 0, b_reserved1: 0,
        }
    }
}

/// 백로그 대화상자
pub struct BacklogDialog {
    hwnd: HWND,
    main_hwnd: HWND,
    config: Rc<RefCell<Config>>,
    richedit: HWND,
    filter: BacklogFilter,
    add_linefeed: bool,
    entries: Vec<LogEntry>,
}

impl DialogControls for BacklogDialog {
    fn dialog_hwnd(&self) -> HWND { self.hwnd }
}

thread_local! {
    static RICHEDIT_LOADED: RefCell<bool> = const { RefCell::new(false) };
}

impl_dialog! {
    dialog: BacklogDialog,
    instance: BACKLOG_INSTANCE,
    class_name: w!("AnemoneBacklogClass"),
    title: w!("백로그"),
    width: BACKLOG_WIDTH,
    height: BACKLOG_HEIGHT,
    extra_style: WS_SIZEBOX,
    params: (parent: HWND, config: Rc<RefCell<Config>>),
    init: |hwnd, parent, config| {
        // RichEdit 4.1+ DLL 로드 (Msftedit.dll, Vista+)
        //
        // System32 한정 검색으로 DLL hijacking 방어 (cwd/PATH 무시).
        RICHEDIT_LOADED.with(|loaded| {
            if !*loaded.borrow() {
                if let Err(e) = LoadLibraryExW(
                    w!("Msftedit.dll"),
                    None,
                    LOAD_LIBRARY_SEARCH_SYSTEM32,
                ) {
                    tracing::error!("Failed to load Msftedit.dll: {e}");
                }
                *loaded.borrow_mut() = true;
            }
        });
        BacklogDialog {
            hwnd,
            main_hwnd: parent,
            config,
            richedit: HWND::default(),
            filter: BacklogFilter::All,
            add_linefeed: true,
            entries: Vec::new(),
        }
    },
}

impl BacklogDialog {
    /// 컨트롤 생성
    fn create_controls(&mut self) -> Result<()> {
        // SAFETY: self.hwnd is a valid window handle from show_impl. CreateWindowExW and
        // SendMessageW use valid handles and parameters.
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let dpi = crate::dpi::dpi_for_window(self.hwnd);
            let s = |v: i32| crate::dpi::scale(v, dpi);

            // RichEdit 4.1+ 컨트롤 생성 (MSFTEDIT_CLASS)
            self.richedit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("RICHEDIT50W"),
                w!(""),
                WINDOW_STYLE(
                    WS_CHILD.0 | WS_VISIBLE.0 | WS_VSCROLL.0 | WS_HSCROLL.0
                        | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32
                        | ES_AUTOHSCROLL as u32 | ES_READONLY as u32,
                ),
                s(10), s(10), s(BACKLOG_WIDTH - 30), s(BACKLOG_HEIGHT - 120),
                Some(self.hwnd),
                Some(HMENU(ctrl_id::RICHEDIT as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;

            // 배경색 설정 (어두운 색상)
            let _ = SendMessageW(
                self.richedit,
                EM_SETBKGNDCOLOR,
                Some(WPARAM(0)),
                Some(LPARAM(0x282828)), // BGR
            );

            // ====== 옵션 그룹 ======
            self.create_group_box(10, BACKLOG_HEIGHT - 100, 350, 60, "필터")?;

            self.create_radio(20, BACKLOG_HEIGHT - 80, 80, 20,
                ctrl_id::RADIO_ORIGINAL, "원문만", self.filter == BacklogFilter::Original)?;
            self.create_radio(105, BACKLOG_HEIGHT - 80, 80, 20,
                ctrl_id::RADIO_TRANSLATION, "번역만", self.filter == BacklogFilter::Translation)?;
            self.create_radio(190, BACKLOG_HEIGHT - 80, 60, 20,
                ctrl_id::RADIO_ALL, "전체", self.filter == BacklogFilter::All)?;

            self.create_checkbox(260, BACKLOG_HEIGHT - 80, 90, 20,
                ctrl_id::CHK_LINEFEED, "줄바꿈 추가", self.add_linefeed)?;

            // ====== 버튼 그룹 ======
            self.create_group_box(370, BACKLOG_HEIGHT - 100, 200, 60, "동작")?;

            self.create_button(380, BACKLOG_HEIGHT - 78, 55, 28, ctrl_id::BTN_CLEAR, "초기화")?;
            self.create_button(445, BACKLOG_HEIGHT - 78, 55, 28, ctrl_id::BTN_SAVE, "저장")?;
            self.create_button(510, BACKLOG_HEIGHT - 78, 55, 28, ctrl_id::BTN_FONT, "폰트")?;

            Ok(())
        }
    }

    /// 커스텀 메시지 핸들러
    fn handle_message(&mut self, msg: u32, _wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_SIZE => {
                let width = (lparam.0 & 0xFFFF) as i32;
                let height = ((lparam.0 >> 16) & 0xFFFF) as i32;
                self.on_size(width, height);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    /// 로그 항목 추가
    pub fn add_entry(&mut self, entry: LogEntry) {
        self.entries.push(entry.clone());
        self.append_entry_to_richedit(&entry);
    }

    /// RichEdit에 항목 추가
    fn append_entry_to_richedit(&self, entry: &LogEntry) {
        // SAFETY: self.richedit is a valid RichEdit control handle from create_controls.
        // SendMessageW and append_styled_text use valid control handles.
        unsafe {
            let _ = SendMessageW(
                self.richedit, EM_SETSEL,
                Some(WPARAM(usize::MAX)), Some(LPARAM(-1)),
            );

            if let Some(ref name) = entry.name {
                if self.filter != BacklogFilter::Translation {
                    self.append_styled_text(&format!("[{}] ", name), 0x00FFFF, true);
                }
            }

            if self.filter != BacklogFilter::Translation {
                self.append_styled_text(&entry.original, 0xFFFFFF, false);
                if self.add_linefeed {
                    self.append_styled_text("\r\n", 0xFFFFFF, false);
                }
            }

            if let Some(ref translation) = entry.translation {
                if self.filter != BacklogFilter::Original {
                    self.append_styled_text(translation, 0x90EE90, false);
                    if self.add_linefeed {
                        self.append_styled_text("\r\n", 0x90EE90, false);
                    }
                }
            }

            if self.filter == BacklogFilter::All && self.add_linefeed {
                self.append_styled_text("\r\n", 0x808080, false);
            }

            let _ = SendMessageW(
                self.richedit, EM_SCROLLCARET,
                Some(WPARAM(0)), Some(LPARAM(0)),
            );
        }
    }

    /// 스타일 텍스트 추가
    unsafe fn append_styled_text(&self, text: &str, color: u32, bold: bool) {
        // SAFETY: self.richedit is a valid RichEdit control. CHARFORMAT2W is properly
        // initialized with correct cbSize. The pointer cast to isize for LPARAM is valid
        // because the CHARFORMAT2W struct lives on the stack for the duration of the call.
        unsafe {
            let mut cf = CHARFORMAT2W::default();
            cf.dw_mask = CFM_COLOR | CFM_SIZE;
            cf.cr_text_color = color;
            cf.y_height = 200;

            if bold {
                cf.dw_mask |= CFM_BOLD;
                cf.dw_effects |= CFE_BOLD;
            }

            let _ = SendMessageW(
                self.richedit, EM_SETCHARFORMAT,
                Some(WPARAM(SCF_SELECTION as usize)),
                Some(LPARAM(&cf as *const _ as isize)),
            );

            let wide = to_wide(text);
            let _ = SendMessageW(
                self.richedit, EM_REPLACESEL,
                Some(WPARAM(0)),
                Some(LPARAM(wide.as_ptr() as isize)),
            );
        }
    }

    /// RichEdit 내용 지우기
    fn clear_richedit(&mut self) {
        self.entries.clear();
        // SAFETY: self.richedit is a valid RichEdit control handle.
        unsafe { let _ = SetWindowTextW(self.richedit, w!("")); }
    }

    /// RichEdit 다시 그리기 (필터 변경 시)
    fn refresh_richedit(&self) {
        // SAFETY: self.richedit is a valid RichEdit control handle.
        unsafe { let _ = SetWindowTextW(self.richedit, w!("")); }
        for entry in &self.entries.clone() {
            self.append_entry_to_richedit(entry);
        }
    }

    /// 파일로 저장
    fn save_to_file(&self) {
        use std::io::Write;

        let filters = [
            FileFilter { name: "텍스트 파일 (*.txt)", spec: "*.txt" },
            FileFilter { name: "모든 파일 (*.*)", spec: "*.*" },
        ];
        let Some(path) = save_file(self.hwnd, "백로그 저장", &filters, Some("txt"), None) else {
            return;
        };

        let mut content = String::new();
        for entry in &self.entries {
            if let Some(ref name) = entry.name {
                content.push_str(&format!("[{}] ", name));
            }
            content.push_str(&entry.original);
            content.push_str("\r\n");
            if let Some(ref trans) = entry.translation {
                content.push_str(trans);
                content.push_str("\r\n");
            }
            content.push_str("\r\n");
        }

        match std::fs::File::create(&path) {
            Ok(mut file) => {
                if let Err(e) = file.write_all(&[0xEF, 0xBB, 0xBF])
                    .and_then(|_| file.write_all(content.as_bytes()))
                {
                    tracing::error!("backlog save write failed: {e}");
                }
            }
            Err(e) => {
                tracing::error!("backlog save file create failed: {e}");
            }
        }
    }

    /// 명령 처리
    fn handle_command(&mut self, cmd: u16, _notify_code: u32) {
        use ctrl_id::*;

        match cmd {
            CHK_LINEFEED => { self.add_linefeed = !self.add_linefeed; self.refresh_richedit(); }
            RADIO_ORIGINAL => { self.filter = BacklogFilter::Original; self.refresh_richedit(); }
            RADIO_TRANSLATION => { self.filter = BacklogFilter::Translation; self.refresh_richedit(); }
            RADIO_ALL => { self.filter = BacklogFilter::All; self.refresh_richedit(); }
            BTN_CLEAR => self.clear_richedit(),
            BTN_SAVE => self.save_to_file(),
            BTN_FONT => { /* TODO: 폰트 선택 대화상자 */ }
            _ => {}
        }
    }

    /// 윈도우 크기 변경 시 컨트롤 재배치
    fn on_size(&self, width: i32, height: i32) {
        // SAFETY: self.richedit is a valid control handle from create_controls.
        unsafe {
            let dpi = crate::dpi::dpi_for_window(self.hwnd);
            let s = |v: i32| crate::dpi::scale(v, dpi);
            let _ = SetWindowPos(
                self.richedit, None,
                s(10), s(10), width - s(30), height - s(120),
                SWP_NOZORDER,
            );
        }
    }
}

/// 공개 인터페이스: 기존 백로그에 항목 추가
pub fn add_to_backlog(entry: LogEntry) {
    BACKLOG_INSTANCE.with(|cell| {
        let Ok(guard) = cell.try_borrow() else { return; };
        if let Some(ref dialog) = *guard {
            if let Ok(mut d) = dialog.try_borrow_mut() {
                d.add_entry(entry);
            }
        }
    });
}
