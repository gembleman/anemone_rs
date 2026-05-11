//! 다이얼로그 공통 헬퍼
//!
//! 윈도우 클래스 등록, 윈도우 생성 등 다이얼로그 간 공통 패턴 추출.

use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::Controls::*,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::constants::{CB_ADDSTRING, CB_SETCURSEL, LBS_NOINTEGRALHEIGHT, LBS_NOTIFY};
use crate::util::to_wide;

/// 다이얼로그 공용 한글 폰트 (Malgun Gothic 9pt).
///
/// 최초 호출 시 `CreateFontW`로 생성 후 캐시. 실패 시 DEFAULT_GUI_FONT로 폴백.
/// 프로세스 종료 시 OS가 핸들을 정리하므로 명시적 해제는 하지 않는다.
pub fn dialog_font() -> HFONT {
    thread_local! {
        static CACHED: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    }
    CACHED.with(|c| {
        let cur = c.get();
        if cur != 0 {
            return HFONT(cur as *mut _);
        }
        // SAFETY: CreateFontW is called with literal-safe parameters.
        // 시스템 DPI(Win10 1607+)에 맞춰 폰트 높이 스케일링.
        let height = crate::dpi::scale_font_for_system(-12);
        let hfont = unsafe {
            let face = to_wide("맑은 고딕");
            CreateFontW(
                height, 0, 0, 0,
                FW_NORMAL.0 as i32, 0, 0, 0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                PCWSTR(face.as_ptr()),
            )
        };
        if hfont.0.is_null() {
            // SAFETY: GetStockObject returns a process-wide stock handle.
            let stock = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
            return HFONT(stock.0 as *mut _);
        }
        c.set(hfont.0 as isize);
        hfont
    })
}

/// 표준 다이얼로그 윈도우 클래스를 등록한다.
///
/// 이미 등록된 클래스면 무시한다.
/// 모든 다이얼로그가 동일한 WNDCLASSEXW 설정을 사용하므로
/// `class_name`과 `wndproc`만 다르게 받는다.
// SAFETY: Caller must provide a valid class_name and wndproc function pointer.
pub unsafe fn register_dialog_class(
    class_name: PCWSTR,
    wndproc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
) -> Result<()> {
    // SAFETY: GetModuleHandleW(None) returns a valid module handle. RegisterClassExW is
    // called with a properly initialized WNDCLASSEXW. ERROR_CLASS_ALREADY_EXISTS is handled.
    unsafe {
        let instance = GetModuleHandleW(None)?;

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance.into(),
            hIcon: LoadIconW(None, IDI_APPLICATION)?,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as *mut _),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
            hIconSm: HICON::default(),
        };

        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            let err = GetLastError();
            if err != ERROR_CLASS_ALREADY_EXISTS {
                return Err(Error::from_hresult(HRESULT::from_win32(err.0)));
            }
        }

        Ok(())
    }
}

/// 다이얼로그 윈도우 생성 옵션
pub struct DialogWindowOptions {
    pub class_name: PCWSTR,
    pub title: PCWSTR,
    pub width: i32,
    pub height: i32,
    pub parent: HWND,
    /// 추가 윈도우 스타일 (WS_SIZEBOX 등). 기본: WS_POPUP | WS_CAPTION | WS_SYSMENU
    pub extra_style: WINDOW_STYLE,
}

/// 화면 중앙에 표준 다이얼로그 윈도우를 생성한다.
///
/// 스타일: WS_EX_TOOLWINDOW + (WS_POPUP | WS_CAPTION | WS_SYSMENU | extra_style)
// SAFETY: Caller must provide valid options (parent HWND, class name previously registered).
pub unsafe fn create_dialog_window(opts: &DialogWindowOptions) -> Result<HWND> {
    // SAFETY: All parameters are valid. The class was registered via register_dialog_class.
    unsafe {
        let instance = GetModuleHandleW(None)?;

        // 96-DPI 기준 크기를 부모 윈도우의 DPI 로 스케일링.
        // 자식 컨트롤은 create_child 에서 다이얼로그 자체 DPI 로 동일 비율 스케일된다.
        let dpi = crate::dpi::dpi_for_window(opts.parent);
        let w = crate::dpi::scale(opts.width, dpi);
        let h = crate::dpi::scale(opts.height, dpi);

        // 부모 윈도우가 위치한 모니터의 작업 영역(작업 표시줄 제외) 중앙에 배치.
        // 다중 모니터 / Per-Monitor V2 환경에서 primary 모니터로 튀는 문제를 방지.
        let monitor = MonitorFromWindow(opts.parent, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let work = if GetMonitorInfoW(monitor, &mut mi).as_bool() {
            mi.rcWork
        } else {
            RECT {
                left: 0,
                top: 0,
                right: GetSystemMetrics(SM_CXSCREEN),
                bottom: GetSystemMetrics(SM_CYSCREEN),
            }
        };
        let x = work.left + (work.right - work.left - w) / 2;
        let y = work.top + (work.bottom - work.top - h) / 2;

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            opts.class_name,
            opts.title,
            WS_POPUP | WS_CAPTION | WS_SYSMENU | opts.extra_style,
            x,
            y,
            w,
            h,
            Some(opts.parent),
            None,
            Some(instance.into()),
            None,
        )?;

        Ok(hwnd)
    }
}

/// 부모 윈도우 중앙에 다이얼로그 윈도우를 생성한다 (진행률 대화상자 등).
// SAFETY: Caller must provide valid options (parent HWND, class name previously registered).
pub unsafe fn create_dialog_window_centered_on_parent(
    opts: &DialogWindowOptions,
) -> Result<HWND> {
    // SAFETY: opts.parent is a valid window handle. GetWindowRect and CreateWindowExW use
    // valid parameters. The class was registered via register_dialog_class.
    unsafe {
        let instance = GetModuleHandleW(None)?;

        // 96-DPI 기준 크기를 부모 윈도우의 DPI 로 스케일링.
        let dpi = crate::dpi::dpi_for_window(opts.parent);
        let w = crate::dpi::scale(opts.width, dpi);
        let h = crate::dpi::scale(opts.height, dpi);

        let mut parent_rect = RECT::default();
        let _ = GetWindowRect(opts.parent, &mut parent_rect);
        let x = parent_rect.left + (parent_rect.right - parent_rect.left - w) / 2;
        let y = parent_rect.top + (parent_rect.bottom - parent_rect.top - h) / 2;

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            opts.class_name,
            opts.title,
            WS_POPUP | WS_CAPTION | WS_SYSMENU | opts.extra_style,
            x,
            y,
            w,
            h,
            Some(opts.parent),
            None,
            Some(instance.into()),
            None,
        )?;

        Ok(hwnd)
    }
}

/// 다이얼로그 윈도우를 표시한다.
// SAFETY: Caller must provide a valid hwnd from create_dialog_window.
pub unsafe fn show_dialog_window(hwnd: HWND) {
    // SAFETY: hwnd is a valid window handle from CreateWindowExW.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
    }
}

// ============================================================
// 컨트롤 생성 헬퍼
// ============================================================

/// 자식 컨트롤을 생성하고 기본 GUI 폰트를 설정한다.
///
/// 좌표(`x`, `y`, `w`, `h`) 는 96 DPI 디자인 단위로 받으며, 부모 윈도우의 DPI 로
/// 자동 스케일링된다.
// SAFETY: Caller must provide a valid parent HWND, class name, and text pointer.
unsafe fn create_child(
    parent: HWND,
    class: PCWSTR,
    text: PCWSTR,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u16,
) -> Result<HWND> {
    // SAFETY: parent is a valid window handle. CreateWindowExW creates a child control
    // with valid class and style. GetStockObject returns a valid font handle.
    unsafe {
        let hinst = GetModuleHandleW(None)?;

        let dpi = crate::dpi::dpi_for_window(parent);
        let sx = crate::dpi::scale(x, dpi);
        let sy = crate::dpi::scale(y, dpi);
        let sw = crate::dpi::scale(w, dpi);
        let sh = crate::dpi::scale(h, dpi);

        let hwnd = CreateWindowExW(
            ex_style,
            class,
            text,
            style,
            sx,
            sy,
            sw,
            sh,
            Some(parent),
            Some(HMENU(id as isize as *mut _)),
            Some(hinst.into()),
            None,
        )?;

        let hfont = dialog_font();
        let _ = SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(WPARAM(hfont.0 as usize)),
            Some(LPARAM(0)),
        );

        Ok(hwnd)
    }
}

/// 그룹 박스 생성
// SAFETY: Caller must provide a valid parent HWND.
pub unsafe fn create_group_box(
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    text: &str,
) -> Result<HWND> {
    let text_wide = to_wide(text);
    // SAFETY: parent is valid; text_wide is a valid null-terminated UTF-16 string.
    unsafe {
        create_child(
            parent,
            w!("BUTTON"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(BS_GROUPBOX as u32 | WS_CHILD.0 | WS_VISIBLE.0),
            WINDOW_EX_STYLE::default(),
            x, y, w, h, 0,
        )
    }
}

/// 라벨(STATIC) 생성
// SAFETY: Caller must provide a valid parent HWND.
pub unsafe fn create_label(
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u16,
    text: &str,
) -> Result<HWND> {
    let text_wide = to_wide(text);
    // SAFETY: parent is valid; text_wide is a valid null-terminated UTF-16 string.
    unsafe {
        create_child(
            parent,
            w!("STATIC"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
            WINDOW_EX_STYLE::default(),
            x, y, w, h, id,
        )
    }
}

/// 푸시 버튼 생성
// SAFETY: Caller must provide a valid parent HWND.
pub unsafe fn create_button(
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u16,
    text: &str,
) -> Result<HWND> {
    let text_wide = to_wide(text);
    // SAFETY: parent is valid; text_wide is a valid null-terminated UTF-16 string.
    unsafe {
        create_child(
            parent,
            w!("BUTTON"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(BS_PUSHBUTTON as u32 | WS_CHILD.0 | WS_VISIBLE.0),
            WINDOW_EX_STYLE::default(),
            x, y, w, h, id,
        )
    }
}

/// 체크박스 생성
// SAFETY: Caller must provide a valid parent HWND.
pub unsafe fn create_checkbox(
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u16,
    text: &str,
    checked: bool,
) -> Result<HWND> {
    let text_wide = to_wide(text);
    // SAFETY: parent is valid; text_wide is a valid null-terminated UTF-16 string.
    // SendMessageW with BM_SETCHECK uses a valid control handle returned by create_child.
    unsafe {
        let hwnd = create_child(
            parent,
            w!("BUTTON"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(BS_AUTOCHECKBOX as u32 | WS_CHILD.0 | WS_VISIBLE.0),
            WINDOW_EX_STYLE::default(),
            x, y, w, h, id,
        )?;

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

/// 라디오 버튼 생성
// SAFETY: Caller must provide a valid parent HWND.
pub unsafe fn create_radio(
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u16,
    text: &str,
    checked: bool,
) -> Result<HWND> {
    let text_wide = to_wide(text);
    // SAFETY: parent is valid; text_wide is a valid null-terminated UTF-16 string.
    unsafe {
        let hwnd = create_child(
            parent,
            w!("BUTTON"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(BS_AUTORADIOBUTTON as u32 | WS_CHILD.0 | WS_VISIBLE.0),
            WINDOW_EX_STYLE::default(),
            x, y, w, h, id,
        )?;

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

/// 콤보박스 생성 (아이템 + 초기 선택)
// SAFETY: Caller must provide a valid parent HWND.
pub unsafe fn create_combobox(
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u16,
    items: &[&str],
    selected: usize,
) -> Result<HWND> {
    // SAFETY: parent is valid. Item strings are converted to valid null-terminated UTF-16.
    // SendMessageW with CB_ADDSTRING/CB_SETCURSEL uses valid control handle and string pointers.
    unsafe {
        let hwnd = create_child(
            parent,
            w!("COMBOBOX"),
            w!(""),
            WINDOW_STYLE(
                CBS_DROPDOWNLIST as u32 | CBS_HASSTRINGS as u32
                    | WS_CHILD.0 | WS_VISIBLE.0 | WS_VSCROLL.0,
            ),
            WINDOW_EX_STYLE::default(),
            x, y, w, h, id,
        )?;

        for item in items {
            let item_wide = to_wide(item);
            let _ = SendMessageW(
                hwnd,
                CB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(item_wide.as_ptr() as isize)),
            );
        }

        let _ = SendMessageW(
            hwnd,
            CB_SETCURSEL,
            Some(WPARAM(selected)),
            Some(LPARAM(0)),
        );

        Ok(hwnd)
    }
}

/// 에디트 컨트롤 생성
// SAFETY: Caller must provide a valid parent HWND.
pub unsafe fn create_edit(
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u16,
    text: &str,
) -> Result<HWND> {
    let text_wide = to_wide(text);
    // SAFETY: parent is valid; text_wide is a valid null-terminated UTF-16 string.
    unsafe {
        create_child(
            parent,
            w!("EDIT"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(ES_AUTOHSCROLL as u32 | WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0),
            WS_EX_CLIENTEDGE,
            x, y, w, h, id,
        )
    }
}

/// 숫자 전용 에디트 컨트롤 생성 (ES_NUMBER)
// SAFETY: Caller must provide a valid parent HWND.
pub unsafe fn create_edit_numeric(
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u16,
    text: &str,
) -> Result<HWND> {
    let text_wide = to_wide(text);
    // SAFETY: parent is valid; text_wide is a valid null-terminated UTF-16 string.
    unsafe {
        create_child(
            parent,
            w!("EDIT"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(
                ES_AUTOHSCROLL as u32 | ES_NUMBER as u32
                    | WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0,
            ),
            WS_EX_CLIENTEDGE,
            x, y, w, h, id,
        )
    }
}

/// 멀티라인 에디트 컨트롤 생성 (수직 스크롤 + 줄바꿈 보존)
// SAFETY: Caller must provide a valid parent HWND.
pub unsafe fn create_multiline_edit(
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u16,
    text: &str,
) -> Result<HWND> {
    let text_wide = to_wide(text);
    // SAFETY: parent is valid; text_wide is a valid null-terminated UTF-16 string.
    unsafe {
        create_child(
            parent,
            w!("EDIT"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(
                ES_MULTILINE as u32
                    | ES_AUTOVSCROLL as u32
                    | ES_WANTRETURN as u32
                    | WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_VSCROLL.0
                    | WS_TABSTOP.0,
            ),
            WS_EX_CLIENTEDGE,
            x, y, w, h, id,
        )
    }
}

/// 리스트박스 생성
// SAFETY: Caller must provide a valid parent HWND.
pub unsafe fn create_listbox(
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u16,
) -> Result<HWND> {
    // SAFETY: parent is valid; delegating to create_child with valid parameters.
    unsafe {
        create_child(
            parent,
            w!("LISTBOX"),
            w!(""),
            WINDOW_STYLE(
                LBS_NOTIFY | LBS_NOINTEGRALHEIGHT
                    | WS_CHILD.0 | WS_VISIBLE.0 | WS_VSCROLL.0 | WS_TABSTOP.0,
            ),
            WS_EX_CLIENTEDGE,
            x, y, w, h, id,
        )
    }
}

// ============================================================
// DialogControls 트레이트
// ============================================================

/// `dialog_hwnd()`만 구현하면 모든 컨트롤 생성 래퍼를 자동으로 사용할 수 있는 트레이트.
pub trait DialogControls {
    fn dialog_hwnd(&self) -> HWND;

    unsafe fn create_group_box(&self, x: i32, y: i32, w: i32, h: i32, text: &str) -> Result<HWND> {
        unsafe { create_group_box(self.dialog_hwnd(), x, y, w, h, text) }
    }

    unsafe fn create_label(&self, x: i32, y: i32, w: i32, h: i32, text: &str) -> Result<HWND> {
        unsafe { create_label(self.dialog_hwnd(), x, y, w, h, 0, text) }
    }

    unsafe fn create_label_with_id(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str) -> Result<HWND> {
        unsafe { create_label(self.dialog_hwnd(), x, y, w, h, id, text) }
    }

    unsafe fn create_button(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str) -> Result<HWND> {
        unsafe { create_button(self.dialog_hwnd(), x, y, w, h, id, text) }
    }

    unsafe fn create_checkbox(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str, checked: bool) -> Result<HWND> {
        unsafe { create_checkbox(self.dialog_hwnd(), x, y, w, h, id, text, checked) }
    }

    unsafe fn create_radio(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str, checked: bool) -> Result<HWND> {
        unsafe { create_radio(self.dialog_hwnd(), x, y, w, h, id, text, checked) }
    }

    unsafe fn create_combobox(&self, x: i32, y: i32, w: i32, h: i32, id: u16, items: &[&str], selected: usize) -> Result<HWND> {
        unsafe { create_combobox(self.dialog_hwnd(), x, y, w, h, id, items, selected) }
    }

    unsafe fn create_edit(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str) -> Result<HWND> {
        unsafe { create_edit(self.dialog_hwnd(), x, y, w, h, id, text) }
    }

    unsafe fn create_edit_numeric(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str) -> Result<HWND> {
        unsafe { create_edit_numeric(self.dialog_hwnd(), x, y, w, h, id, text) }
    }

    unsafe fn create_multiline_edit(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str) -> Result<HWND> {
        unsafe { create_multiline_edit(self.dialog_hwnd(), x, y, w, h, id, text) }
    }

    unsafe fn create_listbox(&self, x: i32, y: i32, w: i32, h: i32, id: u16) -> Result<HWND> {
        unsafe { create_listbox(self.dialog_hwnd(), x, y, w, h, id) }
    }

    unsafe fn create_trackbar(&self, x: i32, y: i32, w: i32, h: i32, id: u16, min: i32, max: i32) -> Result<HWND> {
        unsafe { create_trackbar(self.dialog_hwnd(), x, y, w, h, id, min, max) }
    }

    unsafe fn create_tab_control(&self, x: i32, y: i32, w: i32, h: i32, id: u16, tabs: &[&str]) -> Result<HWND> {
        unsafe { create_tab_control(self.dialog_hwnd(), x, y, w, h, id, tabs) }
    }
}

// ============================================================
// impl_dialog! 매크로
// ============================================================

/// 다이얼로그 보일러플레이트를 생성하는 매크로.
///
/// thread_local, show(), show_impl(), wndproc를 자동 생성한다.
/// 각 다이얼로그는 `create_controls(&mut self)`, `handle_command(&mut self, id, notify_code)`,
/// 그리고 선택적으로 `handle_message(&mut self, msg, wparam, lparam) -> Option<LRESULT>`를 구현한다.
#[macro_export]
macro_rules! impl_dialog {
    (
        dialog: $Dialog:ident,
        instance: $INSTANCE:ident,
        class_name: $class_name:expr,
        title: $title:expr,
        width: $width:expr,
        height: $height:expr,
        extra_style: $extra_style:expr,
        params: (parent: $parent_type:ty $(, $param_name:ident : $param_type:ty)*),
        init: |$hwnd_arg:ident, $parent_arg:ident $(, $init_param:ident)*| $init_body:expr,
    ) => {
        thread_local! {
            static $INSTANCE: std::cell::RefCell<Option<std::rc::Rc<std::cell::RefCell<$Dialog>>>> = const { std::cell::RefCell::new(None) };
        }

        #[allow(dead_code)]
        impl $Dialog {
            pub fn show($parent_arg: $parent_type $(, $param_name: $param_type)*) -> windows::core::Result<windows::Win32::Foundation::HWND> {
                unsafe { Self::show_impl($parent_arg $(, $param_name)*) }
            }

            unsafe fn show_impl($parent_arg: $parent_type $(, $param_name: $param_type)*) -> windows::core::Result<windows::Win32::Foundation::HWND> {
                unsafe {
                    use crate::dialogs::helpers::{self, DialogWindowOptions};

                    helpers::register_dialog_class($class_name, Self::wndproc)?;

                    let $hwnd_arg = helpers::create_dialog_window(&DialogWindowOptions {
                        class_name: $class_name,
                        title: $title,
                        width: $width,
                        height: $height,
                        parent: $parent_arg,
                        extra_style: $extra_style,
                    })?;

                    let dialog = std::rc::Rc::new(std::cell::RefCell::new($init_body));

                    $INSTANCE.with(|cell| {
                        *cell.borrow_mut() = Some(dialog.clone());
                    });

                    dialog.borrow_mut().create_controls()?;

                    helpers::show_dialog_window($hwnd_arg);

                    Ok($hwnd_arg)
                }
            }

            unsafe extern "system" fn wndproc(
                hwnd: windows::Win32::Foundation::HWND,
                msg: u32,
                wparam: windows::Win32::Foundation::WPARAM,
                lparam: windows::Win32::Foundation::LPARAM,
            ) -> windows::Win32::Foundation::LRESULT {
                unsafe {
                    use windows::Win32::Foundation::{WPARAM, LPARAM, LRESULT};
                    use windows::Win32::UI::WindowsAndMessaging::*;

                    let instance = $INSTANCE.with(|cell| {
                        let Ok(guard) = cell.try_borrow() else { return None; };
                        guard.clone()
                    });

                    if let Some(dialog) = instance {
                        // 먼저 커스텀 메시지 핸들러 확인
                        if let Ok(mut d) = dialog.try_borrow_mut() {
                            if let Some(result) = d.handle_message(msg, wparam, lparam) {
                                return result;
                            }
                        }

                        match msg {
                            WM_COMMAND => {
                                let id = (wparam.0 & 0xFFFF) as u16;
                                let notify_code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                                if let Ok(mut d) = dialog.try_borrow_mut() {
                                    d.handle_command(id, notify_code);
                                }
                                return LRESULT(0);
                            }

                            WM_CLOSE => {
                                let _ = DestroyWindow(hwnd);
                                return LRESULT(0);
                            }

                            WM_DESTROY => {
                                $INSTANCE.with(|cell| {
                                    if let Ok(mut guard) = cell.try_borrow_mut() {
                                        *guard = None;
                                    }
                                });
                                return LRESULT(0);
                            }

                            WM_LBUTTONDOWN => {
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
    };
}

/// 탭 컨트롤 생성
// SAFETY: Caller must provide a valid parent HWND.
pub unsafe fn create_tab_control(
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u16,
    tabs: &[&str],
) -> Result<HWND> {
    // SAFETY: parent is valid. SysTabControl32 is a standard common control class.
    unsafe {
        let hwnd = create_child(
            parent,
            w!("SysTabControl32"),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0),
            WINDOW_EX_STYLE::default(),
            x, y, w, h, id,
        )?;

        for (i, tab_text) in tabs.iter().enumerate() {
            let mut text_wide = to_wide(tab_text);
            let item = TCITEMW {
                mask: TCIF_TEXT,
                pszText: PWSTR(text_wide.as_mut_ptr()),
                iImage: -1,
                ..Default::default()
            };
            let _ = SendMessageW(
                hwnd,
                TCM_INSERTITEMW,
                Some(WPARAM(i)),
                Some(LPARAM(&item as *const TCITEMW as isize)),
            );
        }

        Ok(hwnd)
    }
}

/// 트랙바(슬라이더) 생성
// SAFETY: Caller must provide a valid parent HWND.
pub unsafe fn create_trackbar(
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u16,
    min: i32,
    max: i32,
) -> Result<HWND> {
    // SAFETY: parent is valid. create_child creates a valid trackbar control.
    // TBM_SETRANGE uses valid control handle with packed min/max in lparam.
    unsafe {
        let hwnd = create_child(
            parent,
            w!("msctls_trackbar32"),
            w!(""),
            WINDOW_STYLE(TBS_HORZ as u32 | TBS_NOTICKS as u32 | WS_CHILD.0 | WS_VISIBLE.0),
            WINDOW_EX_STYLE::default(),
            x, y, w, h, id,
        )?;

        let _ = SendMessageW(
            hwnd,
            TBM_SETRANGE,
            Some(WPARAM(1)),
            Some(LPARAM(((max << 16) | min) as isize)),
        );

        Ok(hwnd)
    }
}
