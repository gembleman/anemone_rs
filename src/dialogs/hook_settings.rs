//! 후크 설정 대화상자
//!
//! 활성/비활성 후크를 관리하는 대화상자.
//! ListBox를 사용하여 후크를 이동하고 순서를 변경.

use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::config::Config;

// ListBox 메시지 상수
const LB_ADDSTRING: u32 = 0x0180;
const LB_DELETESTRING: u32 = 0x0182;
const LB_INSERTSTRING: u32 = 0x0181;
const LB_GETCURSEL: u32 = 0x0188;
const LB_SETCURSEL: u32 = 0x0186;
const LB_GETCOUNT: u32 = 0x018B;
const LB_GETTEXT: u32 = 0x0189;
const LB_GETTEXTLEN: u32 = 0x018A;
const LB_ERR: i32 = -1;

// ListBox 스타일 상수
const LBS_NOTIFY: u32 = 0x0001;
const LBS_NOINTEGRALHEIGHT: u32 = 0x0100;

// 컨트롤 ID
mod ctrl_id {
    pub const ACTIVE_LIST: u16 = 6001;
    pub const INACTIVE_LIST: u16 = 6002;
    pub const BTN_TO_INACTIVE: u16 = 6010; // -> (활성 -> 비활성)
    pub const BTN_TO_ACTIVE: u16 = 6011; // <- (비활성 -> 활성)
    pub const BTN_UP: u16 = 6020;
    pub const BTN_DOWN: u16 = 6021;
    pub const BTN_APPLY: u16 = 6030;
    pub const BTN_CLOSE: u16 = 6031;
}

const HOOK_SETTINGS_CLASS_NAME: PCWSTR = w!("AnemoneHookSettingsClass");
const DIALOG_WIDTH: i32 = 450;
const DIALOG_HEIGHT: i32 = 350;

/// 후크 설정 대화상자
pub struct HookSettingsDialog {
    hwnd: HWND,
    config: Rc<RefCell<Config>>,
    /// 임시 활성 후크 목록 (적용 전까지 Config에 반영하지 않음)
    active_hooks: Vec<String>,
    /// 임시 비활성 후크 목록
    inactive_hooks: Vec<String>,
}

thread_local! {
    static HOOK_SETTINGS_INSTANCE: RefCell<Option<Rc<RefCell<HookSettingsDialog>>>> = const { RefCell::new(None) };
}

impl HookSettingsDialog {
    /// 후크 설정 대화상자 생성 및 표시
    pub fn show(parent: HWND, config: Rc<RefCell<Config>>) -> Result<HWND> {
        unsafe { Self::show_impl(parent, config) }
    }

    unsafe fn show_impl(parent: HWND, config: Rc<RefCell<Config>>) -> Result<HWND> {
        unsafe {
            let instance = GetModuleHandleW(None)?;

            // 윈도우 클래스 등록
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(Self::wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance.into(),
                hIcon: LoadIconW(None, IDI_APPLICATION)?,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as *mut _),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: HOOK_SETTINGS_CLASS_NAME,
                hIconSm: HICON::default(),
            };

            let atom = RegisterClassExW(&wc);
            if atom == 0 {
                let err = GetLastError();
                if err != ERROR_CLASS_ALREADY_EXISTS {
                    return Err(Error::from_hresult(HRESULT::from_win32(err.0)));
                }
            }

            // 화면 중앙에 위치
            let cx = GetSystemMetrics(SM_CXSCREEN);
            let cy = GetSystemMetrics(SM_CYSCREEN);
            let x = (cx - DIALOG_WIDTH) / 2;
            let y = (cy - DIALOG_HEIGHT) / 2;

            // 윈도우 생성
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                HOOK_SETTINGS_CLASS_NAME,
                w!("후크 설정"),
                WS_POPUP | WS_CAPTION | WS_SYSMENU,
                x,
                y,
                DIALOG_WIDTH,
                DIALOG_HEIGHT,
                Some(parent),
                None,
                Some(instance.into()),
                None,
            )?;

            // Config에서 후크 목록 복사
            let (active, inactive) = {
                let cfg = config.borrow();
                (
                    cfg.hook.active_hooks.clone(),
                    cfg.hook.inactive_hooks.clone(),
                )
            };

            // 인스턴스 생성
            let dialog = Rc::new(RefCell::new(HookSettingsDialog {
                hwnd,
                config,
                active_hooks: active,
                inactive_hooks: inactive,
            }));

            // 전역 인스턴스 설정
            HOOK_SETTINGS_INSTANCE.with(|cell| {
                *cell.borrow_mut() = Some(dialog.clone());
            });

            // 컨트롤 생성
            dialog.borrow_mut().create_controls()?;

            // 윈도우 표시
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);

            Ok(hwnd)
        }
    }

    /// 컨트롤 생성
    fn create_controls(&mut self) -> Result<()> {
        unsafe {
            // ====== 활성 후크 그룹 ======
            self.create_group_box(10, 10, 180, 240, "활성 후크")?;
            self.create_listbox(20, 30, 160, 180, ctrl_id::ACTIVE_LIST)?;

            // 위로/아래로 버튼
            self.create_button(20, 215, 75, 25, ctrl_id::BTN_UP, "위로")?;
            self.create_button(105, 215, 75, 25, ctrl_id::BTN_DOWN, "아래로")?;

            // ====== 이동 버튼 (중앙) ======
            self.create_button(200, 80, 50, 30, ctrl_id::BTN_TO_INACTIVE, "->")?;
            self.create_button(200, 120, 50, 30, ctrl_id::BTN_TO_ACTIVE, "<-")?;

            // ====== 비활성 후크 그룹 ======
            self.create_group_box(260, 10, 180, 240, "비활성 후크")?;
            self.create_listbox(270, 30, 160, 210, ctrl_id::INACTIVE_LIST)?;

            // ====== 적용/닫기 버튼 ======
            self.create_button(270, 270, 80, 30, ctrl_id::BTN_APPLY, "적용")?;
            self.create_button(360, 270, 80, 30, ctrl_id::BTN_CLOSE, "닫기")?;

            // ListBox 초기화
            self.populate_listboxes();

            Ok(())
        }
    }

    /// ListBox에 데이터 채우기
    fn populate_listboxes(&self) {
        unsafe {
            if let Ok(active_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::ACTIVE_LIST as i32) {
                for hook in &self.active_hooks {
                    self.listbox_add_item(active_lb, hook);
                }
            }

            if let Ok(inactive_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::INACTIVE_LIST as i32) {
                for hook in &self.inactive_hooks {
                    self.listbox_add_item(inactive_lb, hook);
                }
            }
        }
    }

    // ====== 헬퍼 함수들 ======

    unsafe fn create_group_box(&self, x: i32, y: i32, w: i32, h: i32, text: &str) -> Result<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(text_wide.as_ptr()),
                WINDOW_STYLE(BS_GROUPBOX as u32 | WS_CHILD.0 | WS_VISIBLE.0),
                x,
                y,
                w,
                h,
                Some(self.hwnd),
                None,
                Some(hinst.into()),
                None,
            )?;

            let hfont = GetStockObject(DEFAULT_GUI_FONT);
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            Ok(hwnd)
        }
    }

    unsafe fn create_button(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: u16,
        text: &str,
    ) -> Result<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(text_wide.as_ptr()),
                WINDOW_STYLE(BS_PUSHBUTTON as u32 | WS_CHILD.0 | WS_VISIBLE.0),
                x,
                y,
                w,
                h,
                Some(self.hwnd),
                Some(HMENU(id as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;

            let hfont = GetStockObject(DEFAULT_GUI_FONT);
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            Ok(hwnd)
        }
    }

    unsafe fn create_listbox(&self, x: i32, y: i32, w: i32, h: i32, id: u16) -> Result<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;

            let hwnd = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("LISTBOX"),
                w!(""),
                WINDOW_STYLE(
                    LBS_NOTIFY
                        | LBS_NOINTEGRALHEIGHT
                        | WS_CHILD.0
                        | WS_VISIBLE.0
                        | WS_VSCROLL.0
                        | WS_TABSTOP.0,
                ),
                x,
                y,
                w,
                h,
                Some(self.hwnd),
                Some(HMENU(id as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;

            let hfont = GetStockObject(DEFAULT_GUI_FONT);
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(hfont.0 as usize)),
                Some(LPARAM(0)),
            );

            Ok(hwnd)
        }
    }

    /// ListBox에 아이템 추가
    unsafe fn listbox_add_item(&self, hwnd: HWND, text: &str) {
        unsafe {
            let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = SendMessageW(
                hwnd,
                LB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(text_wide.as_ptr() as isize)),
            );
        }
    }

    /// ListBox 아이템 삽입
    unsafe fn listbox_insert_item(&self, hwnd: HWND, index: i32, text: &str) {
        unsafe {
            let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = SendMessageW(
                hwnd,
                LB_INSERTSTRING,
                Some(WPARAM(index as usize)),
                Some(LPARAM(text_wide.as_ptr() as isize)),
            );
        }
    }

    /// ListBox 아이템 삭제
    unsafe fn listbox_delete_item(&self, hwnd: HWND, index: i32) {
        unsafe {
            let _ = SendMessageW(
                hwnd,
                LB_DELETESTRING,
                Some(WPARAM(index as usize)),
                Some(LPARAM(0)),
            );
        }
    }

    /// ListBox 선택된 인덱스 가져오기
    unsafe fn listbox_get_sel(&self, hwnd: HWND) -> i32 {
        unsafe { SendMessageW(hwnd, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32 }
    }

    /// ListBox 선택 설정
    unsafe fn listbox_set_sel(&self, hwnd: HWND, index: i32) {
        unsafe {
            let _ = SendMessageW(
                hwnd,
                LB_SETCURSEL,
                Some(WPARAM(index as usize)),
                Some(LPARAM(0)),
            );
        }
    }

    /// ListBox 아이템 개수 가져오기
    unsafe fn listbox_get_count(&self, hwnd: HWND) -> i32 {
        unsafe { SendMessageW(hwnd, LB_GETCOUNT, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32 }
    }

    /// ListBox 아이템 텍스트 가져오기
    unsafe fn listbox_get_text(&self, hwnd: HWND, index: i32) -> Option<String> {
        unsafe {
            let len = SendMessageW(
                hwnd,
                LB_GETTEXTLEN,
                Some(WPARAM(index as usize)),
                Some(LPARAM(0)),
            )
            .0 as i32;
            if len == LB_ERR || len <= 0 {
                return None;
            }

            let mut buffer: Vec<u16> = vec![0; (len + 1) as usize];
            let result = SendMessageW(
                hwnd,
                LB_GETTEXT,
                Some(WPARAM(index as usize)),
                Some(LPARAM(buffer.as_mut_ptr() as isize)),
            );

            if result.0 as i32 == LB_ERR {
                return None;
            }

            String::from_utf16(&buffer[..len as usize]).ok()
        }
    }

    /// 내부 후크 목록 동기화 (ListBox -> 내부 Vec)
    fn sync_from_listboxes(&mut self) {
        unsafe {
            if let Ok(active_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::ACTIVE_LIST as i32) {
                self.active_hooks.clear();
                let count = self.listbox_get_count(active_lb);
                for i in 0..count {
                    if let Some(text) = self.listbox_get_text(active_lb, i) {
                        self.active_hooks.push(text);
                    }
                }
            }

            if let Ok(inactive_lb) = GetDlgItem(Some(self.hwnd), ctrl_id::INACTIVE_LIST as i32) {
                self.inactive_hooks.clear();
                let count = self.listbox_get_count(inactive_lb);
                for i in 0..count {
                    if let Some(text) = self.listbox_get_text(inactive_lb, i) {
                        self.inactive_hooks.push(text);
                    }
                }
            }
        }
    }

    /// 명령 처리
    fn handle_command(&mut self, cmd: u16) {
        use ctrl_id::*;

        match cmd {
            BTN_CLOSE => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },

            BTN_APPLY => {
                // 내부 목록 동기화
                self.sync_from_listboxes();

                // Config에 반영
                {
                    let mut cfg = self.config.borrow_mut();
                    cfg.hook.active_hooks = self.active_hooks.clone();
                    cfg.hook.inactive_hooks = self.inactive_hooks.clone();
                }
            }

            BTN_TO_INACTIVE => {
                // 활성 -> 비활성
                unsafe {
                    let active_lb = match GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) {
                        Ok(h) => h,
                        Err(_) => return,
                    };
                    let inactive_lb = match GetDlgItem(Some(self.hwnd), INACTIVE_LIST as i32) {
                        Ok(h) => h,
                        Err(_) => return,
                    };

                    let sel = self.listbox_get_sel(active_lb);
                    if sel == LB_ERR {
                        return;
                    }

                    if let Some(text) = self.listbox_get_text(active_lb, sel) {
                        self.listbox_delete_item(active_lb, sel);
                        self.listbox_add_item(inactive_lb, &text);

                        // 선택 유지
                        let count = self.listbox_get_count(active_lb);
                        if count > 0 {
                            let new_sel = if sel >= count { count - 1 } else { sel };
                            self.listbox_set_sel(active_lb, new_sel);
                        }
                    }
                }
            }

            BTN_TO_ACTIVE => {
                // 비활성 -> 활성
                unsafe {
                    let active_lb = match GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) {
                        Ok(h) => h,
                        Err(_) => return,
                    };
                    let inactive_lb = match GetDlgItem(Some(self.hwnd), INACTIVE_LIST as i32) {
                        Ok(h) => h,
                        Err(_) => return,
                    };

                    let sel = self.listbox_get_sel(inactive_lb);
                    if sel == LB_ERR {
                        return;
                    }

                    if let Some(text) = self.listbox_get_text(inactive_lb, sel) {
                        self.listbox_delete_item(inactive_lb, sel);
                        self.listbox_add_item(active_lb, &text);

                        // 선택 유지
                        let count = self.listbox_get_count(inactive_lb);
                        if count > 0 {
                            let new_sel = if sel >= count { count - 1 } else { sel };
                            self.listbox_set_sel(inactive_lb, new_sel);
                        }
                    }
                }
            }

            BTN_UP => {
                // 활성 목록에서 위로
                unsafe {
                    let active_lb = match GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) {
                        Ok(h) => h,
                        Err(_) => return,
                    };
                    let sel = self.listbox_get_sel(active_lb);
                    if sel == LB_ERR || sel == 0 {
                        return;
                    }

                    if let Some(text) = self.listbox_get_text(active_lb, sel) {
                        self.listbox_delete_item(active_lb, sel);
                        self.listbox_insert_item(active_lb, sel - 1, &text);
                        self.listbox_set_sel(active_lb, sel - 1);
                    }
                }
            }

            BTN_DOWN => {
                // 활성 목록에서 아래로
                unsafe {
                    let active_lb = match GetDlgItem(Some(self.hwnd), ACTIVE_LIST as i32) {
                        Ok(h) => h,
                        Err(_) => return,
                    };
                    let sel = self.listbox_get_sel(active_lb);
                    let count = self.listbox_get_count(active_lb);

                    if sel == LB_ERR || sel >= count - 1 {
                        return;
                    }

                    if let Some(text) = self.listbox_get_text(active_lb, sel) {
                        self.listbox_delete_item(active_lb, sel);
                        self.listbox_insert_item(active_lb, sel + 1, &text);
                        self.listbox_set_sel(active_lb, sel + 1);
                    }
                }
            }

            _ => {}
        }
    }

    /// WndProc
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe {
            let instance = HOOK_SETTINGS_INSTANCE.with(|cell| cell.borrow().clone());

            if let Some(dialog) = instance {
                match msg {
                    WM_COMMAND => {
                        let id = (wparam.0 & 0xFFFF) as u16;
                        dialog.borrow_mut().handle_command(id);
                        return LRESULT(0);
                    }

                    WM_CLOSE => {
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }

                    WM_DESTROY => {
                        HOOK_SETTINGS_INSTANCE.with(|cell| {
                            *cell.borrow_mut() = None;
                        });
                        return LRESULT(0);
                    }

                    WM_LBUTTONDOWN => {
                        // 창 드래그
                        let _ = SendMessageW(
                            hwnd,
                            WM_NCLBUTTONDOWN,
                            Some(WPARAM(HTCAPTION as usize)),
                            Some(LPARAM(0)),
                        );
                        return LRESULT(0);
                    }

                    _ => {}
                }
            }

            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}
