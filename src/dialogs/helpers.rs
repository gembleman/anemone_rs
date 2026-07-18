//! Resource dialog 수명, DPI, text와 ListBox 공통 helper.

use std::collections::{HashMap, VecDeque};

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, UI::HiDpi::AdjustWindowRectExForDpi,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::util::to_wide;

/// owner를 가진 일관된 오류 대화상자를 표시한다.
pub fn show_error_message(owner: HWND, title: &str, message: &str) {
    let title = to_wide(title);
    let message = to_wide(message);
    unsafe {
        let _ = MessageBoxW(
            Some(owner),
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// 모델리스 dialog의 RefCell 재진입으로 처리하지 못한 pointer-free message를 재예약한다.
pub unsafe fn defer_dialog_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) {
    DEFERRED_DIALOG_MESSAGES.with(|queue| {
        queue
            .borrow_mut()
            .push_back((hwnd.0 as isize, msg, wparam.0, lparam.0));
    });
}

/// 바깥 dialog handler의 borrow가 해제된 뒤 해당 HWND의 deferred message를 게시한다.
pub fn flush_deferred_dialog_messages(hwnd: HWND) {
    let raw = hwnd.0 as isize;
    let pending = DEFERRED_DIALOG_MESSAGES.with(|queue| {
        let mut queue = queue.borrow_mut();
        let mut pending = Vec::new();
        let mut retained = VecDeque::new();
        while let Some(message) = queue.pop_front() {
            if message.0 == raw {
                pending.push(message);
            } else {
                retained.push_back(message);
            }
        }
        *queue = retained;
        pending
    });
    if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        return;
    }
    for (_, msg, wparam, lparam) in pending {
        if let Err(error) = unsafe { PostMessageW(Some(hwnd), msg, WPARAM(wparam), LPARAM(lparam)) }
        {
            tracing::warn!("failed to post deferred dialog message 0x{msg:04X}: {error}");
        }
    }
}

/// WM_DPICHANGED의 임시 RECT는 즉시 복사·적용하고 pointer를 제거한 후 재예약한다.
pub unsafe fn defer_dialog_dpi_change(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    if lparam.0 != 0 {
        let rect = unsafe { *(lparam.0 as *const RECT) };
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    unsafe { defer_dialog_message(hwnd, WM_DPICHANGED, wparam, LPARAM(0)) };
}

thread_local! {
    static RESOURCE_DIALOGS: std::cell::RefCell<Vec<isize>> = const { std::cell::RefCell::new(Vec::new()) };
    static DEFERRED_DIALOG_MESSAGES: std::cell::RefCell<VecDeque<(isize, u32, usize, isize)>> =
        const { std::cell::RefCell::new(VecDeque::new()) };
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
    DEFERRED_DIALOG_MESSAGES.with(|queue| {
        queue
            .borrow_mut()
            .retain(|message| message.0 != hwnd.0 as isize);
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

/// DPI별로 cache하며 process 종료 시 OS가 정리하는 dialog font.
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
                w!("맑은 고딕"),
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

/// 96-DPI client 디자인 크기를 title/border를 포함한 window 크기로 바꾼다.
/// API 실패 시 DPI scale만 적용한다.
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

/// Dialog의 현재 style과 DPI로 96-DPI client 크기를 전체 pixel 크기로 바꾼다.
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
    unsafe { SetWindowTextW(hwnd, &HSTRING::from(text)) }
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
