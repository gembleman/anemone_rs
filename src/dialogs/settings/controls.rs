//! 설정 대화상자 컨트롤 생성 헬퍼 함수

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
        UI::Controls::*, UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::SettingsDialog;

impl SettingsDialog {
    pub(super) unsafe fn create_group_box(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        text: &str,
    ) -> Result<HWND> {
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

    pub(super) unsafe fn create_label(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        text: &str,
    ) -> Result<HWND> {
        unsafe { self.create_label_with_id(x, y, w, h, 0, text) }
    }

    pub(super) unsafe fn create_label_with_id(
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
                w!("STATIC"),
                PCWSTR(text_wide.as_ptr()),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
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

    pub(super) unsafe fn create_button(
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

    pub(super) unsafe fn create_color_button(
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

    pub(super) unsafe fn create_checkbox(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: u16,
        text: &str,
        checked: bool,
    ) -> Result<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(text_wide.as_ptr()),
                WINDOW_STYLE(BS_AUTOCHECKBOX as u32 | WS_CHILD.0 | WS_VISIBLE.0),
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

            if checked {
                let _ = SendMessageW(
                    hwnd,
                    BM_SETCHECK,
                    Some(WPARAM(BST_CHECKED.0 as usize)),
                    Some(LPARAM(0)),
                );
            }

            Ok(hwnd)
        }
    }

    pub(super) unsafe fn create_radio(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: u16,
        text: &str,
        checked: bool,
    ) -> Result<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(text_wide.as_ptr()),
                WINDOW_STYLE(BS_AUTORADIOBUTTON as u32 | WS_CHILD.0 | WS_VISIBLE.0),
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

            if checked {
                let _ = SendMessageW(
                    hwnd,
                    BM_SETCHECK,
                    Some(WPARAM(BST_CHECKED.0 as usize)),
                    Some(LPARAM(0)),
                );
            }

            Ok(hwnd)
        }
    }

    pub(super) unsafe fn create_trackbar(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: u16,
        min: i32,
        max: i32,
    ) -> Result<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("msctls_trackbar32"),
                w!(""),
                WINDOW_STYLE(TBS_HORZ as u32 | TBS_NOTICKS as u32 | WS_CHILD.0 | WS_VISIBLE.0),
                x,
                y,
                w,
                h,
                Some(self.hwnd),
                Some(HMENU(id as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;

            // 범위 설정
            let _ = SendMessageW(
                hwnd,
                TBM_SETRANGE,
                Some(WPARAM(1)),
                Some(LPARAM(((max << 16) | min) as isize)),
            );

            Ok(hwnd)
        }
    }

    pub(super) unsafe fn create_edit(
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
                WS_EX_CLIENTEDGE,
                w!("EDIT"),
                PCWSTR(text_wide.as_ptr()),
                WINDOW_STYLE(
                    ES_AUTOHSCROLL as u32 | WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0,
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

    pub(super) unsafe fn create_combobox(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: u16,
        items: &[&str],
        selected: usize,
    ) -> Result<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("COMBOBOX"),
                w!(""),
                WINDOW_STYLE(
                    CBS_DROPDOWNLIST as u32 | WS_CHILD.0 | WS_VISIBLE.0 | WS_VSCROLL.0,
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

            // 아이템 추가
            for item in items {
                let item_wide: Vec<u16> =
                    item.encode_utf16().chain(std::iter::once(0)).collect();
                let _ = SendMessageW(
                    hwnd,
                    super::CB_ADDSTRING,
                    Some(WPARAM(0)),
                    Some(LPARAM(item_wide.as_ptr() as isize)),
                );
            }

            // 선택 설정
            let _ = SendMessageW(
                hwnd,
                super::CB_SETCURSEL,
                Some(WPARAM(selected)),
                Some(LPARAM(0)),
            );

            Ok(hwnd)
        }
    }
}
