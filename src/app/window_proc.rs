use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT},
    UI::WindowsAndMessaging::{
        DefWindowProcW, GetClientRect, HTCAPTION, HTTRANSPARENT, KillTimer, MINMAXINFO,
        PostQuitMessage, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, WM_CLIPBOARDUPDATE, WM_CLOSE,
        WM_COMMAND, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENTERSIZEMOVE,
        WM_EXITSIZEMOVE, WM_GETMINMAXINFO, WM_HOTKEY, WM_NCHITTEST, WM_NCRBUTTONUP, WM_PAINT,
        WM_RBUTTONUP, WM_SIZE, WM_TIMER,
    },
};

use super::messages::{
    WM_APP_ACTION, WM_APP_MAGNETIC_TARGET_SELECTED, WM_APP_REFRESH, WM_APP_SET_MAGNETIC,
    WM_DEFERRED_PAINT, WM_DEFERRED_RESIZE, WM_TRANSLATION_COMPLETE, WM_TRAY_ICON,
    WM_UPDATE_PROGRESS, WM_UPDATE_RESULT,
};
use super::{
    APP, App, CLIPBOARD_DEBOUNCE_TIMER, CLIPBOARD_READ_RETRY_TIMER, COMPOSITION_RETRY_TIMER,
    MAGNETIC_NOTICE_TIMER,
};
use crate::window;

const MIN_WINDOW_SIZE: i32 = 100;
const RESIZE_BORDER_WIDTH: i32 = 8;

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
        | WM_HOTKEY | WM_TRAY_ICON | WM_CLIPBOARDUPDATE | WM_TIMER | WM_ENTERSIZEMOVE
        | WM_EXITSIZEMOVE => ReentryPolicy::DeferOwned,
        _ if matches!(
            msg,
            WM_APP_REFRESH
                | WM_APP_ACTION
                | WM_APP_MAGNETIC_TARGET_SELECTED
                | WM_APP_SET_MAGNETIC
                | WM_DEFERRED_RESIZE
                | WM_DEFERRED_PAINT
                | WM_TRANSLATION_COMPLETE
                | WM_UPDATE_RESULT
                | WM_UPDATE_PROGRESS
        ) =>
        {
            ReentryPolicy::DeferOwned
        }
        _ => ReentryPolicy::Default,
    }
}

impl App {
    /// 처리 결과를 반환하거나 `None`으로 `DefWindowProcW`에 위임한다.
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
        // SAFETY: system이 message별로 유효한 hwnd와 인자를 제공한다.
        unsafe {
            match msg {
                _ if self.taskbar_created_msg != 0 && msg == self.taskbar_created_msg => {
                    self.tray.restore();
                    Some(LRESULT(0))
                }
                WM_DESTROY => {
                    DEFERRED_MESSAGES.with(|queue| queue.borrow_mut().clear());
                    // 죽은 hwnd로 완료 message를 보내지 않도록 routing을 먼저 해제한다.
                    self.services.translation_ui.unregister(hwnd);
                    // HWND가 유효한 마지막 lifecycle 구간에서 listener를 해제한다.
                    // App은 window보다 늦게 drop되므로 여기서 상태도 종료해야 한다.
                    if let Err(error) = self.clipboard.stop_for_window_destroy() {
                        tracing::warn!(
                            "Failed to stop clipboard listener during window destruction: {error}"
                        );
                    }
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

                WM_ENTERSIZEMOVE => {
                    self.model.runtime.resizing = true;
                    Some(LRESULT(0))
                }

                WM_EXITSIZEMOVE => {
                    self.model.runtime.resizing = false;
                    if let Some(size) = self.model.runtime.pending_resize.take() {
                        if let Err(e) = self.resize(size.width, size.height) {
                            // take()로 목표 크기는 이미 사라졌다 — 그대로 두면 model의
                            // client_size가 실제 창 크기/swap chain과 어긋난 채 남는다.
                            // 실제 client 영역을 다시 읽어 재동기화한다.
                            tracing::warn!("exit-resize failed: {e}");
                            self.sync_client_size(hwnd);
                        }
                    } else {
                        self.sync_client_size(hwnd);
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
                    // 권장 RECT 적용 후 WM_SIZE가 swap chain resize와 paint를 잇는다.
                    Self::apply_dpi_rect(hwnd, lparam);
                    if !self.sync_client_size(hwnd)
                        && let Err(e) = self.paint()
                    {
                        // 권장 rect가 위치만 바꾸거나 같은 pixel 크기여도 render target
                        // DPI와 DPI 종속 cache는 반드시 갱신해야 한다.
                        tracing::warn!("paint failed on DPI change: {e}");
                    }
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

                WM_TIMER if wparam.0 == CLIPBOARD_READ_RETRY_TIMER => {
                    let _ = KillTimer(Some(hwnd), CLIPBOARD_READ_RETRY_TIMER);
                    self.handle_clipboard_change();
                    Some(LRESULT(0))
                }

                WM_TIMER if wparam.0 == MAGNETIC_NOTICE_TIMER => {
                    self.handle_magnetic_notice_timer();
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

                _ if msg == WM_APP_ACTION => {
                    self.process_actions();
                    Some(LRESULT(0))
                }

                _ if msg == WM_APP_SET_MAGNETIC => {
                    self.apply_magnetic_request(wparam.0 != 0);
                    Some(LRESULT(0))
                }

                _ if msg == WM_APP_MAGNETIC_TARGET_SELECTED => {
                    self.handle_magnetic_target_selected(HWND(wparam.0 as _));
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
                    self.handle_translation_complete();
                    Some(LRESULT(0))
                }

                _ if msg == WM_UPDATE_RESULT => {
                    self.handle_update_result();
                    Some(LRESULT(0))
                }

                _ if msg == WM_UPDATE_PROGRESS => {
                    self.handle_update_progress();
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

    /// 실제 client 크기를 model/swap chain과 맞추고 크기가 바뀌었는지 반환한다.
    fn sync_client_size(&mut self, hwnd: HWND) -> bool {
        let mut rect = RECT::default();
        match unsafe { GetClientRect(hwnd, &mut rect) } {
            Ok(()) => {
                let previous = self.model.runtime.client_size;
                if let Err(error) = self.resize(rect.right - rect.left, rect.bottom - rect.top) {
                    tracing::warn!("client-size synchronization failed: {error}");
                }
                self.model.runtime.client_size != previous
            }
            Err(error) => {
                tracing::warn!("GetClientRect failed after window resize: {error}");
                false
            }
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

    /// `RefMut<App>` 해제 후, 임시 system pointer를 포함하지 않은 message만 처리한다.
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
                    if let Ok(app_ref) = app.try_borrow() {
                        // 클릭 통과는 resize/drag 판정보다 항상 우선한다. 확장 스타일과
                        // 명시적 hit-test를 함께 적용해 다른 프로세스의 아래 창도 후보가 된다.
                        if app_ref.model.config.click_through {
                            return LRESULT(HTTRANSPARENT as isize);
                        }
                        if !app_ref.full_hit_region
                            && !window::point_in_any_rect(hwnd, x, y, &app_ref.hit_region)
                        {
                            return LRESULT(HTTRANSPARENT as isize);
                        }
                    }
                    let dpi = crate::dpi::dpi_for_window(hwnd);
                    let resize_border = crate::dpi::scale(RESIZE_BORDER_WIDTH, dpi).max(1);
                    if let Some(hit) = window::hit_test_resize_border(hwnd, x, y, resize_border) {
                        return LRESULT(hit as isize);
                    }
                    return LRESULT(HTCAPTION as isize);
                }

                WM_GETMINMAXINFO => {
                    let mm = &mut *(lparam.0 as *mut MINMAXINFO);
                    let dpi = crate::dpi::dpi_for_window(hwnd);
                    let min_size = crate::dpi::scale(MIN_WINDOW_SIZE, dpi).max(1);
                    window::set_min_track_size(mm, min_size, min_size);
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
