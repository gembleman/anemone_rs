//! Raw Win32 helpers shared by modeless resource dialogs.

use crate::win32::to_wide;
use std::collections::VecDeque;
use windows_core::{Error, HRESULT};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::UpdateWindow,
    UI::Controls::{TCHITTESTINFO, TCM_HITTEST},
    UI::HiDpi::AdjustWindowRectExForDpi,
    UI::Input::KeyboardAndMouse::ReleaseCapture,
    UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
    UI::WindowsAndMessaging::*,
};
type Result<T> = windows_core::Result<T>;

pub fn show_error_message(owner: HWND, title: &str, message: &str) {
    let t = to_wide(title);
    let m = to_wide(message);
    unsafe {
        MessageBoxW(owner, m.as_ptr(), t.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

type Deferred = (isize, u32, usize, isize);
/// dialog proc 앞에서 메시지를 먼저 가로챌 기회를 주는 callback.
type PretranslateFn = unsafe fn(HWND, &MSG) -> bool;
type ResourceDialog = (HWND, PretranslateFn);
thread_local! {
    static DEFERRED_DIALOG_MESSAGES: std::cell::RefCell<VecDeque<Deferred>> = const { std::cell::RefCell::new(VecDeque::new()) };
    static RESOURCE_DIALOGS: std::cell::RefCell<Vec<ResourceDialog>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub unsafe fn defer_dialog_message(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) {
    DEFERRED_DIALOG_MESSAGES.with(|q| {
        q.borrow_mut()
            .push_back((hwnd as isize, msg, wparam, lparam))
    });
}
pub fn flush_deferred_dialog_messages(hwnd: HWND) {
    let raw = hwnd as isize;
    let pending = DEFERRED_DIALOG_MESSAGES.with(|q| {
        let mut q = q.borrow_mut();
        let mut out = Vec::new();
        let mut keep = VecDeque::new();
        while let Some(v) = q.pop_front() {
            if v.0 == raw {
                out.push(v)
            } else {
                keep.push_back(v)
            }
        }
        *q = keep;
        out
    });
    for (_, msg, wp, lp) in pending {
        unsafe {
            PostMessageW(hwnd, msg, wp, lp);
        }
    }
}
pub unsafe fn defer_dialog_dpi_change(hwnd: HWND, wparam: usize, _lparam: isize) {
    // SAFETY: 호출자가 넘긴 HWND와 복사 가능한 WPARAM만 지연 큐에 저장한다.
    unsafe { defer_dialog_message(hwnd, WM_DPICHANGED, wparam, 0) };
}

pub fn register_resource_dialog(hwnd: HWND, pretranslate_message: PretranslateFn) {
    RESOURCE_DIALOGS.with(|v| v.borrow_mut().push((hwnd, pretranslate_message)));
}
pub fn unregister_resource_dialog(hwnd: HWND) {
    RESOURCE_DIALOGS.with(|v| v.borrow_mut().retain(|(h, _)| *h != hwnd));
    flush_deferred_dialog_messages(hwnd);
}
pub unsafe fn dispatch_resource_dialog_message(msg: &MSG) -> bool {
    // pretranslation과 IsDialogMessageW는 dialog proc을 동기 호출할 수 있고, 그
    // 재진입에서 WM_DESTROY가 dialog를 목록에서 제거한다. callback을 부르기 전에
    // 스냅샷을 만들어 RESOURCE_DIALOGS의 Ref 대여가 재진입 경계를 넘지 않게 한다.
    let dialogs = RESOURCE_DIALOGS.with(|v| v.borrow().clone());
    dialogs
        .into_iter()
        .any(|(hwnd, f)| unsafe { f(hwnd, msg) || IsDialogMessageW(hwnd, msg) != 0 })
}

/// 빈 client 영역을 누른 것을 caption 드래그로 바꿔 창을 옮기게 한다.
///
/// # Safety
/// `hwnd`는 호출 thread가 소유한 유효한 top-level window handle이어야 한다.
/// `SendMessageW`가 modal 이동 loop에 들어가므로, 호출부는 재진입 가능한
/// 상태(대여 중인 `RefCell` 없음)여야 한다.
pub unsafe fn begin_client_drag(hwnd: HWND) {
    // SAFETY: 호출자 계약상 hwnd는 유효하다. ReleaseCapture는 이 thread가
    // 캡처를 잡고 있지 않아도 안전하게 실패한다.
    unsafe {
        let _ = ReleaseCapture();
        SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
    }
}

/// tab control이 자기 client 영역을 모두 삼키므로, 탭 항목이 아닌 곳을 누르면
/// 부모 창의 드래그로 넘긴다. 탭 본문 위에 놓인 `LTEXT`/`GROUPBOX`는
/// `HTTRANSPARENT`라 그 클릭도 여기로 떨어진다.
unsafe extern "system" fn tab_client_drag_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: usize,
    lparam: isize,
    subclass_id: usize,
    _ref_data: usize,
) -> isize {
    unsafe {
        match msg {
            WM_LBUTTONDOWN => {
                let mut hit = TCHITTESTINFO {
                    pt: POINT {
                        x: (lparam & 0xFFFF) as i16 as i32,
                        y: ((lparam >> 16) & 0xFFFF) as i16 as i32,
                    },
                    flags: 0,
                };
                if SendMessageW(
                    hwnd,
                    TCM_HITTEST,
                    0,
                    &mut hit as *mut TCHITTESTINFO as isize,
                ) >= 0
                {
                    return DefSubclassProc(hwnd, msg, wparam, lparam);
                }
                let parent = GetParent(hwnd);
                if parent.is_null() {
                    return DefSubclassProc(hwnd, msg, wparam, lparam);
                }
                begin_client_drag(parent);
                0
            }
            WM_NCDESTROY => {
                let _ =
                    RemoveWindowSubclass(hwnd, Some(tab_client_drag_subclass_proc), subclass_id);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

/// tab control의 빈 영역 드래그를 활성화한다.
pub fn enable_tab_client_drag(tab: HWND, subclass_id: usize) {
    // SAFETY: tab은 호출자가 리소스에서 얻은 유효한 자식 control이며, subclass는
    // WM_NCDESTROY에서 스스로 해제한다.
    unsafe {
        if SetWindowSubclass(tab, Some(tab_client_drag_subclass_proc), subclass_id, 0) == 0 {
            tracing::warn!("탭 여백 드래그 subclass 설치 실패");
        }
    }
}

pub unsafe fn center_dialog_on_monitor(hwnd: HWND, parent: HWND) {
    let mut r = RECT::default();
    let mut p = RECT::default();
    // SAFETY: 호출자 계약상 hwnd와 parent는 유효한 같은 UI thread의 창이다.
    unsafe {
        if GetWindowRect(hwnd, &mut r) == 0 || GetWindowRect(parent, &mut p) == 0 {
            return;
        }
        let w = r.right - r.left;
        let h = r.bottom - r.top;
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            p.left + (p.right - p.left - w) / 2,
            p.top + (p.bottom - p.top - h) / 2,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

pub fn design_to_window_size(hwnd: HWND, design_w: i32, design_h: i32) -> (i32, i32) {
    let mut r = RECT {
        left: 0,
        top: 0,
        right: design_w,
        bottom: design_h,
    };
    unsafe {
        AdjustWindowRectExForDpi(
            &mut r,
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            crate::dpi::dpi_for_window(hwnd),
        );
    }
    (r.right - r.left, r.bottom - r.top)
}
pub fn rescale_dialog_children_for_dpi(_hwnd: HWND, _old_dpi: u32, _new_dpi: u32) {}
pub unsafe fn show_dialog_window(hwnd: HWND) {
    // SAFETY: 호출자 계약상 hwnd는 유효한 dialog handle이다.
    unsafe {
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        UpdateWindow(hwnd);
    }
}

fn failed() -> Error {
    Error::new(HRESULT(0x80004005u32 as i32), "Win32 operation failed")
}
pub fn set_window_text(hwnd: HWND, text: &str) -> Result<()> {
    let w = to_wide(text);
    if unsafe { SetWindowTextW(hwnd, w.as_ptr()) } == 0 {
        Err(failed())
    } else {
        Ok(())
    }
}
pub fn get_window_text(hwnd: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(hwnd);
        if n <= 0 {
            return String::new();
        }
        let mut b = vec![0u16; n as usize + 1];
        let n = GetWindowTextW(hwnd, b.as_mut_ptr(), b.len() as i32);
        String::from_utf16_lossy(&b[..n.max(0) as usize])
    }
}
pub fn set_dlg_item_text(hwnd: HWND, ctrl_id: u16, text: &str) {
    let _ = set_window_text(unsafe { GetDlgItem(hwnd, ctrl_id as i32) }, text);
}
pub fn get_dlg_item_text(hwnd: HWND, ctrl_id: u16) -> String {
    get_window_text(unsafe { GetDlgItem(hwnd, ctrl_id as i32) })
}
pub fn listbox_add_item(hwnd: HWND, text: &str) {
    let w = to_wide(text);
    unsafe {
        SendMessageW(hwnd, LB_ADDSTRING, 0, w.as_ptr() as isize);
    }
}
pub fn listbox_reset(hwnd: HWND) {
    unsafe {
        SendMessageW(hwnd, LB_RESETCONTENT, 0, 0);
    }
}
pub fn listbox_get_sel(hwnd: HWND) -> i32 {
    unsafe { SendMessageW(hwnd, LB_GETCURSEL, 0, 0) as i32 }
}

#[cfg(test)]
#[path = "../../tests/unit/dialogs/helpers.rs"]
mod tests;
