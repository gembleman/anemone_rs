//! 다이얼로그 공통 헬퍼
//!
//! 리소스 다이얼로그 생명주기, DPI, 텍스트와 ListBox 공통 처리.

use std::collections::HashMap;

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, UI::HiDpi::AdjustWindowRectExForDpi,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::util::to_wide;

thread_local! {
    static RESOURCE_DIALOGS: std::cell::RefCell<Vec<isize>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// 열린 리소스 기반 모델리스 다이얼로그를 메시지 루프에 등록한다.
pub fn register_resource_dialog(hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }
    RESOURCE_DIALOGS.with(|dialogs| {
        let mut dialogs = dialogs.borrow_mut();
        let raw = hwnd.0 as isize;
        if !dialogs.contains(&raw) {
            dialogs.push(raw);
        }
    });
}

/// 닫힌 리소스 기반 모델리스 다이얼로그를 메시지 루프에서 해제한다.
pub fn unregister_resource_dialog(hwnd: HWND) {
    RESOURCE_DIALOGS.with(|dialogs| {
        dialogs.borrow_mut().retain(|&raw| raw != hwnd.0 as isize);
    });
}

/// 열린 리소스 다이얼로그 중 하나가 메시지를 처리하면 `true`를 반환한다.
///
/// # Safety
/// `msg`는 현재 UI 스레드의 `GetMessageW`가 채운 유효한 메시지여야 한다.
pub unsafe fn dispatch_resource_dialog_message(msg: &MSG) -> bool {
    let dialogs = RESOURCE_DIALOGS.with(|dialogs| {
        let mut dialogs = dialogs.borrow_mut();
        dialogs.retain(|&raw| unsafe { IsWindow(Some(HWND(raw as *mut _))).as_bool() });
        dialogs.clone()
    });

    dialogs
        .into_iter()
        .any(|raw| unsafe { IsDialogMessageW(HWND(raw as *mut _), msg).as_bool() })
}

/// 부모 윈도우가 있는 모니터의 작업 영역 중앙에 다이얼로그를 배치한다.
///
/// # Safety
/// `hwnd`와 `parent`는 유효한 윈도우 핸들이어야 한다.
pub unsafe fn center_dialog_on_monitor(hwnd: HWND, parent: HWND) {
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return;
        }
        let monitor = MonitorFromWindow(parent, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let work = if GetMonitorInfoW(monitor, &mut info).as_bool() {
            info.rcWork
        } else {
            RECT {
                left: 0,
                top: 0,
                right: GetSystemMetrics(SM_CXSCREEN),
                bottom: GetSystemMetrics(SM_CYSCREEN),
            }
        };
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let x = work.left + (work.right - work.left - width) / 2;
        let y = work.top + (work.bottom - work.top - height) / 2;
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// 지정 DPI 용 다이얼로그 폰트.
///
/// 컨트롤은 DPI 변경 시 새 폰트를 다시 받아야 하므로 DPI 별로 캐시한다.
/// 프로세스 종료 시 OS 가 정리하므로 명시적 해제는 하지 않는다.
pub fn dialog_font_for_dpi(dpi: u32) -> HFONT {
    thread_local! {
        static CACHED: std::cell::RefCell<HashMap<u32, isize>> =
            std::cell::RefCell::new(HashMap::new());
    }
    let dpi = if dpi > 0 { dpi } else { crate::dpi::BASE_DPI };
    CACHED.with(|cache| {
        if let Some(&cur) = cache.borrow().get(&dpi) {
            return HFONT(cur as *mut _);
        }
        // SAFETY: CreateFontW is called with literal-safe parameters.
        // 지정 DPI 에 맞춰 폰트 높이 스케일링.
        let height = crate::dpi::scale(-12, dpi);
        let hfont = unsafe {
            let face = to_wide("맑은 고딕");
            CreateFontW(
                height,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
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
        cache.borrow_mut().insert(dpi, hfont.0 as isize);
        hfont
    })
}

/// `(width, height)` 를 클라이언트 영역 크기로 보고 타이틀/테두리를 더한
/// 전체 윈도우 크기로 변환한다. DPI 와 윈도우 스타일을 함께 반영해, 자식
/// 컨트롤이 디자인 좌표 (96 DPI, 클라이언트 기준) 그대로 배치돼도 잘리지
/// 않도록 보장한다.
///
/// 실패 시(예: 매우 옛 OS) DPI 스케일링만 적용한 값을 폴백으로 돌려준다.
fn client_size_to_window_size(
    width: i32,
    height: i32,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    dpi: u32,
) -> (i32, i32) {
    let w = crate::dpi::scale(width, dpi);
    let h = crate::dpi::scale(height, dpi);
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    // SAFETY: rect 는 스택의 유효한 RECT. style/ex_style 은 호출자 제공값,
    // dpi 는 GetDpiForWindow 결과로 양수.
    let ok = unsafe { AdjustWindowRectExForDpi(&mut rect, style, false, ex_style, dpi).is_ok() };
    if ok {
        (rect.right - rect.left, rect.bottom - rect.top)
    } else {
        (w, h)
    }
}

/// 다이얼로그의 현재 스타일·DPI 기준으로 디자인 좌표(96 DPI 클라이언트
/// 크기)를 윈도우 전체 픽셀 크기로 변환한다.
///
/// 리소스 다이얼로그의 디자인 크기로 창을 재조정할 때 타이틀과 테두리를
/// 포함한 전체 크기를 계산하는 공용 헬퍼.
pub fn design_to_window_size(hwnd: HWND, design_w: i32, design_h: i32) -> (i32, i32) {
    let dpi = crate::dpi::dpi_for_window(hwnd);
    // SAFETY: hwnd 는 유효 윈도우. GetWindowLongPtrW 는 표준 GDI 호출.
    let (style_val, ex_val) = unsafe {
        (
            GetWindowLongPtrW(hwnd, GWL_STYLE) as u32,
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32,
        )
    };
    client_size_to_window_size(
        design_w,
        design_h,
        WINDOW_STYLE(style_val),
        WINDOW_EX_STYLE(ex_val),
        dpi,
    )
}

struct DpiRescaleContext {
    parent: HWND,
    old_dpi: u32,
    new_dpi: u32,
    font: HFONT,
}

#[inline]
fn scale_between_dpi(value: i32, old_dpi: u32, new_dpi: u32) -> i32 {
    ((value as i64) * (new_dpi as i64) / (old_dpi as i64)) as i32
}

unsafe extern "system" fn rescale_child_for_dpi(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = unsafe { &*(lparam.0 as *const DpiRescaleContext) };

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return TRUE;
    }

    let mut top_left = POINT {
        x: rect.left,
        y: rect.top,
    };
    let mut bottom_right = POINT {
        x: rect.right,
        y: rect.bottom,
    };
    unsafe {
        let _ = ScreenToClient(ctx.parent, &mut top_left);
        let _ = ScreenToClient(ctx.parent, &mut bottom_right);
    }

    let x = scale_between_dpi(top_left.x, ctx.old_dpi, ctx.new_dpi);
    let y = scale_between_dpi(top_left.y, ctx.old_dpi, ctx.new_dpi);
    let w = scale_between_dpi(bottom_right.x - top_left.x, ctx.old_dpi, ctx.new_dpi).max(1);
    let h = scale_between_dpi(bottom_right.y - top_left.y, ctx.old_dpi, ctx.new_dpi).max(1);

    unsafe {
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        let _ = SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(WPARAM(ctx.font.0 as usize)),
            Some(LPARAM(1)),
        );
    }

    TRUE
}

pub(super) fn rescale_dialog_children_for_dpi(hwnd: HWND, old_dpi: u32, new_dpi: u32) {
    if old_dpi == 0 || new_dpi == 0 || old_dpi == new_dpi {
        return;
    }

    let ctx = DpiRescaleContext {
        parent: hwnd,
        old_dpi,
        new_dpi,
        font: dialog_font_for_dpi(new_dpi),
    };

    // SAFETY: ctx lives until EnumChildWindows returns; the callback only reads it.
    unsafe {
        let _ = EnumChildWindows(
            Some(hwnd),
            Some(rescale_child_for_dpi),
            LPARAM((&ctx as *const DpiRescaleContext) as isize),
        );
    }
}

/// 다이얼로그 윈도우를 표시한다.
// SAFETY: Caller must provide a valid dialog hwnd.
pub unsafe fn show_dialog_window(hwnd: HWND) {
    // SAFETY: hwnd is a valid window handle from CreateWindowExW.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
    }
}

pub fn set_window_text(hwnd: HWND, text: &str) -> Result<()> {
    let wide = to_wide(text);
    unsafe { SetWindowTextW(hwnd, PCWSTR(wide.as_ptr())) }
}

pub fn get_window_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buffer = vec![0; (len + 1) as usize];
        let copied = GetWindowTextW(hwnd, &mut buffer);
        String::from_utf16_lossy(&buffer[..copied as usize])
    }
}

pub fn listbox_add_item(hwnd: HWND, text: &str) {
    let wide = to_wide(text);
    unsafe {
        let _ = SendMessageW(
            hwnd,
            LB_ADDSTRING,
            Some(WPARAM(0)),
            Some(LPARAM(wide.as_ptr() as isize)),
        );
    }
}

pub fn listbox_reset(hwnd: HWND) {
    unsafe {
        let _ = SendMessageW(hwnd, LB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
    }
}

pub fn listbox_get_sel(hwnd: HWND) -> i32 {
    unsafe { SendMessageW(hwnd, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32 }
}

pub fn listbox_set_sel(hwnd: HWND, index: i32) {
    unsafe {
        let _ = SendMessageW(
            hwnd,
            LB_SETCURSEL,
            Some(WPARAM(index as usize)),
            Some(LPARAM(0)),
        );
    }
}

/// 다이얼로그 타입별 thread-local 인스턴스 슬롯을 선언한다.
///
/// 사용:
/// ```ignore
/// define_dialog_instance!(GLOSSARY_INSTANCE: GlossaryDialog);
/// ```
#[macro_export]
macro_rules! define_dialog_instance {
    ($name:ident : $Dialog:ty) => {
        thread_local! {
            #[allow(non_upper_case_globals)]
            static $name: std::cell::RefCell<Option<std::rc::Rc<std::cell::RefCell<$Dialog>>>>
                = const { std::cell::RefCell::new(None) };
        }
    };
}
