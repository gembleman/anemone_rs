use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ptr::null_mut;
use std::rc::Rc;

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT},
    UI::WindowsAndMessaging::{
        DefWindowProcW, GetClientRect, GetWindowRect, HTCAPTION, HTTRANSPARENT, MINMAXINFO,
        PostMessageW, PostQuitMessage, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
        WM_CLIPBOARDUPDATE, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED,
        WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_GETMINMAXINFO, WM_HOTKEY, WM_NCHITTEST,
        WM_NCRBUTTONUP, WM_PAINT, WM_RBUTTONUP, WM_SIZE, WM_TIMER,
    },
};

use super::messages::{
    WM_APP_ACTION, WM_APP_DRAIN_DEFERRED, WM_APP_HOOK_STATE, WM_APP_MAGNETIC_REPOSITION,
    WM_APP_MAGNETIC_TARGET_SELECTED, WM_APP_REFRESH, WM_APP_SET_MAGNETIC, WM_DEFERRED_PAINT,
    WM_DEFERRED_RESIZE, WM_TRANSLATION_COMPLETE, WM_TRAY_ICON, WM_UPDATE_PROGRESS,
    WM_UPDATE_RESULT,
};
use super::{APP, App};
use crate::window;

pub(super) const MIN_WINDOW_SIZE: i32 = 100;
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
                | WM_APP_DRAIN_DEFERRED
                | WM_APP_MAGNETIC_REPOSITION
                | WM_APP_MAGNETIC_TARGET_SELECTED
                | WM_APP_SET_MAGNETIC
                | WM_APP_HOOK_STATE
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
            if self.taskbar_created_msg != 0 && msg == self.taskbar_created_msg {
                self.tray.restore();
                return Some(0);
            }
            if msg == WM_DESTROY {
                return Some(self.handle_wm_destroy(hwnd));
            }

            if let Some(result) = self.dispatch_size_message(hwnd, msg, wparam, lparam) {
                return Some(result);
            }
            if let Some(result) = self.dispatch_input_message(hwnd, msg, wparam, lparam) {
                return Some(result);
            }
            if let Some(result) = self.dispatch_timer_message(hwnd, msg, wparam, lparam) {
                return Some(result);
            }
            self.dispatch_app_message(hwnd, msg, wparam, lparam)
        }
    }

    /// `WM_DESTROY` 처리: 대기 중인 재진입 message를 비우고 종료 절차를 시작한다.
    fn handle_wm_destroy(&mut self, hwnd: HWND) -> LRESULT {
        DEFERRED_MESSAGES.with(|queue| queue.borrow_mut().clear());
        // 죽은 hwnd로 완료 message를 보내지 않도록 routing을 먼저 해제한다.
        self.services.translation_ui.unregister(hwnd);
        // HWND가 유효한 마지막 lifecycle 구간에서 listener를 해제한다.
        // App은 window보다 늦게 drop되므로 여기서 상태도 종료해야 한다.
        if let Err(error) = self.clipboard.stop_for_window_destroy() {
            tracing::warn!("Failed to stop clipboard listener during window destruction: {error}");
        }
        // SAFETY: 유효한 UI thread에서 호출되는 표준 종료 message 게시다.
        unsafe {
            PostQuitMessage(0);
        }
        0
    }

    pub(super) unsafe fn apply_dpi_rect(hwnd: HWND, lparam: LPARAM) {
        if lparam == 0 {
            return;
        }
        let rect = unsafe { *(lparam as *const RECT) };
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// 현재 창 위치와 크기를 config에 기록한다. 파일 저장은 종료 시점이나 다음
    /// `SaveConfig` effect가 맡는다.
    pub(super) fn remember_window_placement(&mut self, hwnd: HWND) {
        let mut rect = RECT::default();
        // SAFETY: hwnd는 system이 이 message와 함께 넘긴 유효한 창이다.
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            tracing::warn!("GetWindowRect failed while recording window placement");
            return;
        }
        self.model.config.window_x = Some(rect.left);
        self.model.config.window_y = Some(rect.top);
        self.model.config.window_width = Some(rect.right - rect.left);
        self.model.config.window_height = Some(rect.bottom - rect.top);
    }

    /// 실제 client 크기를 model/swap chain과 맞추고 크기가 바뀌었는지 반환한다.
    pub(super) fn sync_client_size(&mut self, hwnd: HWND) -> bool {
        let mut rect = RECT::default();
        if unsafe { GetClientRect(hwnd, &mut rect) } != 0 {
            let previous = self.model.runtime.client_size;
            if let Err(error) = self.resize(rect.right - rect.left, rect.bottom - rect.top) {
                tracing::warn!("client-size synchronization failed: {error}");
            }
            self.model.runtime.client_size != previous
        } else {
            tracing::warn!("GetClientRect failed after window resize");
            false
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
        const DEFERRED_BATCH_SIZE: usize = 32;
        for _ in 0..DEFERRED_BATCH_SIZE {
            let Some(message) = DEFERRED_MESSAGES.with(|queue| queue.borrow_mut().pop_front())
            else {
                return;
            };
            let Ok(mut app_ref) = app.try_borrow_mut() else {
                DEFERRED_MESSAGES.with(|queue| queue.borrow_mut().push_front(message));
                return;
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
        let has_more = DEFERRED_MESSAGES.with(|queue| !queue.borrow().is_empty());
        if has_more {
            let hwnd = app.borrow().hwnd;
            if unsafe { PostMessageW(hwnd, WM_APP_DRAIN_DEFERRED, 0, 0) } == 0 {
                tracing::warn!("보류 메시지의 다음 배치를 게시하지 못했습니다");
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
                0
            }
            ReentryPolicy::ApplyDpiThenResize => {
                unsafe { Self::apply_dpi_rect(hwnd, lparam) };
                Self::enqueue_deferred(DeferredMessage {
                    hwnd,
                    msg: WM_DEFERRED_RESIZE,
                    wparam: 0,
                    lparam: 0,
                });
                0
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
                    wparam: 0,
                    lparam: 0,
                });
                0
            }
            ReentryPolicy::Quit => {
                tracing::warn!("WM_DESTROY arrived during wndproc reentry; quitting safely");
                DEFERRED_MESSAGES.with(|queue| queue.borrow_mut().clear());
                unsafe { PostQuitMessage(0) };
                0
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
                    let x = (lparam & 0xFFFF) as i16 as i32;
                    let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
                    if let Ok(app_ref) = app.try_borrow() {
                        // 클릭 통과는 resize/drag 판정보다 항상 우선한다. 확장 스타일과
                        // 명시적 hit-test를 함께 적용해 다른 프로세스의 아래 창도 후보가 된다.
                        if app_ref.model.config.click_through {
                            return HTTRANSPARENT as isize;
                        }
                        if !app_ref.full_hit_region
                            && !window::point_in_any_rect(hwnd, x, y, &app_ref.hit_region)
                        {
                            return HTTRANSPARENT as isize;
                        }
                    }
                    let dpi = crate::dpi::dpi_for_window(hwnd);
                    let resize_border = crate::dpi::scale(RESIZE_BORDER_WIDTH, dpi).max(1);
                    if let Some(hit) = window::hit_test_resize_border(hwnd, x, y, resize_border) {
                        return hit as isize;
                    }
                    return HTCAPTION as isize;
                }

                WM_GETMINMAXINFO => {
                    let mm = &mut *(lparam as *mut MINMAXINFO);
                    let dpi = crate::dpi::dpi_for_window(hwnd);
                    let min_size = crate::dpi::scale(MIN_WINDOW_SIZE, dpi).max(1);
                    window::set_min_track_size(mm, min_size, min_size);
                    return 0;
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
