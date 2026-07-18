use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT},
    UI::WindowsAndMessaging::{
        DefWindowProcW, GetClientRect, HTCAPTION, HTTRANSPARENT, KillTimer, MINMAXINFO,
        PostQuitMessage, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, WM_CLIPBOARDUPDATE, WM_CLOSE,
        WM_COMMAND, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_GETMINMAXINFO, WM_HOTKEY,
        WM_NCHITTEST, WM_NCRBUTTONUP, WM_PAINT, WM_RBUTTONUP, WM_SIZE, WM_TIMER,
    },
};

use super::{APP, App, CLIPBOARD_DEBOUNCE_TIMER, COMPOSITION_RETRY_TIMER};
use crate::constants::{
    MIN_WINDOW_SIZE, RESIZE_BORDER_WIDTH, WM_APP_REFRESH, WM_APP_SET_MAGNETIC, WM_DEFERRED_PAINT,
    WM_DEFERRED_RESIZE, WM_TRANSLATION_COMPLETE, WM_TRAY_ICON,
};
use crate::translation::unregister_translation_hwnd;
use crate::window;

#[derive(Clone, Copy)]
struct DeferredMessage {
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
}

thread_local! {
    static DEFERRED_MESSAGES: RefCell<VecDeque<DeferredMessage>> =
        const { RefCell::new(VecDeque::new()) };
    static TASKBAR_CREATED_MESSAGE: Cell<u32> = const { Cell::new(0) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReentryPolicy {
    DeferOwned,
    ApplyDpiThenResize,
    ValidatePaintThenRepaint,
    Quit,
    Default,
}

fn reentry_policy(msg: u32, taskbar_created_msg: u32) -> ReentryPolicy {
    if taskbar_created_msg != 0 && msg == taskbar_created_msg {
        return ReentryPolicy::DeferOwned;
    }

    match msg {
        WM_DESTROY => ReentryPolicy::Quit,
        WM_DPICHANGED => ReentryPolicy::ApplyDpiThenResize,
        WM_PAINT => ReentryPolicy::ValidatePaintThenRepaint,
        WM_CLOSE | WM_SIZE | WM_DISPLAYCHANGE | WM_RBUTTONUP | WM_NCRBUTTONUP | WM_COMMAND
        | WM_HOTKEY | WM_TRAY_ICON | WM_CLIPBOARDUPDATE | WM_TIMER => ReentryPolicy::DeferOwned,
        _ if matches!(
            msg,
            WM_APP_REFRESH
                | WM_APP_SET_MAGNETIC
                | WM_DEFERRED_RESIZE
                | WM_DEFERRED_PAINT
                | WM_TRANSLATION_COMPLETE
        ) =>
        {
            ReentryPolicy::DeferOwned
        }
        _ => ReentryPolicy::Default,
    }
}

impl App {
    /// WndProc에서 호출되는 메시지 디스패처
    ///
    /// `Some(LRESULT)`를 반환하면 해당 값을 wndproc 반환값으로 사용.
    /// `None`을 반환하면 DefWindowProcW로 위임.
    ///
    /// # Safety
    /// Win32 메시지 파라미터가 유효해야 한다.
    unsafe fn dispatch_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        // SAFETY: All Win32 API calls use valid parameters from the system-provided
        // hwnd/wparam/lparam. Pointer casts are valid for their respective message types.
        unsafe {
            match msg {
                _ if self.taskbar_created_msg != 0 && msg == self.taskbar_created_msg => {
                    self.tray.restore();
                    Some(LRESULT(0))
                }
                WM_DESTROY => {
                    DEFERRED_MESSAGES.with(|queue| queue.borrow_mut().clear());
                    // 클립보드 자동 번역으로 등록된 라우팅 슬롯 정리. shutdown()
                    // 이전 in-flight 응답이 죽은 HWND 로 PostMessage 시도하는 것을
                    // 막는다. (PostMessage 자체는 안전하지만 silent fail.)
                    unregister_translation_hwnd(hwnd);
                    PostQuitMessage(0);
                    Some(LRESULT(0))
                }

                WM_SIZE => {
                    let width = (lparam.0 & 0xFFFF) as i32;
                    let height = ((lparam.0 >> 16) & 0xFFFF) as i32;
                    if let Err(e) = self.resize(width, height) {
                        tracing::warn!("resize failed: {e}");
                    }
                    Some(LRESULT(0))
                }

                WM_DISPLAYCHANGE => {
                    // 합성 경로에서는 DComp/DXGI 가 모니터 변경에 자체 대응한다.
                    // 즉시 다시 그려주기만 해도 갱신 효과로 충분.
                    if let Err(e) = self.paint() {
                        tracing::warn!("paint failed on display change: {e}");
                    }
                    Some(LRESULT(0))
                }

                WM_DPICHANGED => {
                    // Per-Monitor V2: 모니터 간 이동 또는 OS DPI 변경 시 호출된다.
                    // 자식 컨트롤이 없는 합성 윈도우이므로 권장 RECT 로 위치/크기만 갱신.
                    // 위치/크기 변경은 WM_SIZE 를 유발해 거기서 swap chain resize + paint 가 이어진다.
                    Self::apply_dpi_rect(hwnd, lparam);
                    self.sync_client_size(hwnd);
                    Some(LRESULT(0))
                }

                WM_RBUTTONUP | WM_NCRBUTTONUP => {
                    self.handle_right_click(hwnd, msg, lparam);
                    Some(LRESULT(0))
                }

                WM_COMMAND => {
                    let cmd = (wparam.0 & 0xFFFF) as u16;
                    if let Err(e) = self.handle_menu_command(cmd) {
                        tracing::warn!("handle_menu_command failed: {e}");
                    }
                    Some(LRESULT(0))
                }

                WM_HOTKEY => {
                    let id = wparam.0 as i32;
                    if let Err(e) = self.handle_hotkey(id) {
                        tracing::warn!("handle_hotkey failed: {e}");
                    }
                    Some(LRESULT(0))
                }

                WM_TRAY_ICON => {
                    self.handle_tray_event(lparam);
                    Some(LRESULT(0))
                }

                WM_PAINT => {
                    let mut ps = PAINTSTRUCT::default();
                    let _ = BeginPaint(hwnd, &mut ps);
                    if let Err(e) = self.paint() {
                        tracing::warn!("paint failed on WM_PAINT: {e}");
                    }
                    let _ = EndPaint(hwnd, &ps);
                    Some(LRESULT(0))
                }

                WM_TIMER if wparam.0 == COMPOSITION_RETRY_TIMER => {
                    let _ = KillTimer(Some(hwnd), COMPOSITION_RETRY_TIMER);
                    self.composition_retry_scheduled = false;
                    if let Err(e) = self.paint() {
                        tracing::warn!("composition retry paint failed: {e}");
                    }
                    Some(LRESULT(0))
                }

                WM_TIMER if wparam.0 == CLIPBOARD_DEBOUNCE_TIMER => {
                    self.handle_clipboard_debounce_timer();
                    Some(LRESULT(0))
                }

                WM_CLIPBOARDUPDATE => {
                    self.handle_clipboard_change();
                    Some(LRESULT(0))
                }

                _ if msg == WM_APP_REFRESH => {
                    self.sync_window_state();
                    if let Err(e) = self.paint() {
                        tracing::warn!("paint failed on refresh: {e}");
                    }
                    Some(LRESULT(0))
                }

                _ if msg == WM_APP_SET_MAGNETIC => {
                    self.apply_magnetic_request(wparam.0 != 0);
                    Some(LRESULT(0))
                }

                _ if msg == WM_DEFERRED_RESIZE => {
                    self.sync_client_size(hwnd);
                    Some(LRESULT(0))
                }

                _ if msg == WM_DEFERRED_PAINT => {
                    if let Err(error) = self.paint() {
                        tracing::warn!("deferred paint failed: {error}");
                    }
                    Some(LRESULT(0))
                }

                _ if msg == WM_TRANSLATION_COMPLETE => {
                    self.handle_translation_complete(wparam.0 as u64);
                    Some(LRESULT(0))
                }

                _ => None,
            }
        }
    }

    unsafe fn apply_dpi_rect(hwnd: HWND, lparam: LPARAM) {
        if lparam.0 == 0 {
            return;
        }
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

    fn sync_client_size(&mut self, hwnd: HWND) {
        let mut rect = RECT::default();
        match unsafe { GetClientRect(hwnd, &mut rect) } {
            Ok(()) => {
                if let Err(error) = self.resize(rect.right - rect.left, rect.bottom - rect.top) {
                    tracing::warn!("client-size synchronization failed: {error}");
                }
            }
            Err(error) => tracing::warn!("GetClientRect failed after window resize: {error}"),
        }
    }

    // SAFETY: This is a Win32 window procedure callback. The system guarantees hwnd is valid
    // and msg/wparam/lparam contain valid message data when called.
    pub(super) unsafe extern "system" fn parent_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: Forwarding valid parameters directly to DefWindowProcW.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    pub(super) fn set_taskbar_created_message(msg: u32) {
        TASKBAR_CREATED_MESSAGE.with(|slot| slot.set(msg));
    }

    fn enqueue_deferred(message: DeferredMessage) {
        tracing::debug!("Deferring reentrant app message: 0x{:04X}", message.msg);
        DEFERRED_MESSAGES.with(|queue| queue.borrow_mut().push_back(message));
    }

    /// Drain only after the current `RefMut<App>` has been dropped. Each deferred message owns
    /// all of its parameters; messages whose LPARAM points to temporary system memory are never
    /// put in this queue.
    pub(super) fn drain_deferred_messages(app: &Rc<RefCell<App>>) {
        while let Some(message) = DEFERRED_MESSAGES.with(|queue| queue.borrow_mut().pop_front()) {
            let Ok(mut app_ref) = app.try_borrow_mut() else {
                DEFERRED_MESSAGES.with(|queue| queue.borrow_mut().push_front(message));
                break;
            };
            let result = unsafe {
                app_ref.dispatch_message(message.hwnd, message.msg, message.wparam, message.lparam)
            };
            drop(app_ref);

            if result.is_none() {
                unsafe {
                    DefWindowProcW(message.hwnd, message.msg, message.wparam, message.lparam);
                }
            }
        }
    }

    unsafe fn handle_reentry(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let taskbar_created_msg = TASKBAR_CREATED_MESSAGE.with(Cell::get);
        match reentry_policy(msg, taskbar_created_msg) {
            ReentryPolicy::DeferOwned => {
                Self::enqueue_deferred(DeferredMessage {
                    hwnd,
                    msg,
                    wparam,
                    lparam,
                });
                LRESULT(0)
            }
            ReentryPolicy::ApplyDpiThenResize => {
                unsafe { Self::apply_dpi_rect(hwnd, lparam) };
                Self::enqueue_deferred(DeferredMessage {
                    hwnd,
                    msg: WM_DEFERRED_RESIZE,
                    wparam: WPARAM(0),
                    lparam: LPARAM(0),
                });
                LRESULT(0)
            }
            ReentryPolicy::ValidatePaintThenRepaint => {
                let mut ps = PAINTSTRUCT::default();
                unsafe {
                    let _ = BeginPaint(hwnd, &mut ps);
                    let _ = EndPaint(hwnd, &ps);
                }
                Self::enqueue_deferred(DeferredMessage {
                    hwnd,
                    msg: WM_DEFERRED_PAINT,
                    wparam: WPARAM(0),
                    lparam: LPARAM(0),
                });
                LRESULT(0)
            }
            ReentryPolicy::Quit => {
                tracing::warn!("WM_DESTROY arrived during wndproc reentry; quitting safely");
                DEFERRED_MESSAGES.with(|queue| queue.borrow_mut().clear());
                unregister_translation_hwnd(hwnd);
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            ReentryPolicy::Default => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    // SAFETY: This is a Win32 window procedure callback. The system guarantees hwnd is valid
    // and msg/wparam/lparam contain valid message data when called.
    pub(super) unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let app = APP.with(|cell| cell.try_borrow().ok().and_then(|guard| guard.clone()));
        let Some(app) = app else {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        };

        // Messages with pointer parameters that can be handled without mutable App state.
        unsafe {
            match msg {
                WM_NCHITTEST => {
                    let x = (lparam.0 & 0xFFFF) as i16 as i32;
                    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                    if let Some(hit) =
                        window::hit_test_resize_border(hwnd, x, y, RESIZE_BORDER_WIDTH)
                    {
                        return LRESULT(hit as isize);
                    }
                    if let Ok(app_ref) = app.try_borrow()
                        && !app_ref.hit_region.is_empty()
                        && !window::point_in_any_rect(hwnd, x, y, &app_ref.hit_region)
                    {
                        return LRESULT(HTTRANSPARENT as isize);
                    }
                    return LRESULT(HTCAPTION as isize);
                }
                WM_GETMINMAXINFO => {
                    let mm = &mut *(lparam.0 as *mut MINMAXINFO);
                    window::set_min_track_size(mm, MIN_WINDOW_SIZE, MIN_WINDOW_SIZE);
                    return LRESULT(0);
                }
                _ => {}
            }
        }

        let Ok(mut app_ref) = app.try_borrow_mut() else {
            return unsafe { Self::handle_reentry(hwnd, msg, wparam, lparam) };
        };
        let result = unsafe { app_ref.dispatch_message(hwnd, msg, wparam, lparam) };
        drop(app_ref);

        let result = match result {
            Some(result) => result,
            None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        };
        Self::drain_deferred_messages(&app);
        result
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app/window_proc.rs"]
mod tests;
