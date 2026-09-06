//! [`super::window_proc::App::dispatch_message`]이 메시지 종류별로 위임하는 케이스 본문.
//!
//! 카테고리(크기/DPI, 입력, 타이머/그리기, 커스텀 app message)별로 나눠
//! `dispatch_message` 자체의 순환/인지 복잡도를 낮춘다. 각 함수의 분기·순서·
//! 반환값은 원래 `dispatch_message`의 해당 `match` arm과 동일하다.

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT},
    UI::WindowsAndMessaging::{
        KillTimer, WM_CLIPBOARDUPDATE, WM_COMMAND, WM_DISPLAYCHANGE, WM_DPICHANGED,
        WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_HOTKEY, WM_NCRBUTTONUP, WM_PAINT, WM_RBUTTONUP,
        WM_SIZE, WM_TIMER,
    },
};

use super::messages::{
    WM_APP_ACTION, WM_APP_HOOK_STATE, WM_APP_MAGNETIC_TARGET_SELECTED, WM_APP_REFRESH,
    WM_APP_SET_MAGNETIC, WM_DEFERRED_PAINT, WM_DEFERRED_RESIZE, WM_TRANSLATION_COMPLETE,
    WM_TRAY_ICON, WM_UPDATE_PROGRESS, WM_UPDATE_RESULT,
};
use super::{
    App, CLIPBOARD_DEBOUNCE_TIMER, CLIPBOARD_READ_RETRY_TIMER, COMPOSITION_RETRY_TIMER,
    HOOK_MERGE_TIMER, MAGNETIC_NOTICE_TIMER,
};

impl App {
    /// `WM_SIZE` / `WM_ENTERSIZEMOVE` / `WM_EXITSIZEMOVE` / `WM_DISPLAYCHANGE` /
    /// `WM_DPICHANGED` 처리.
    ///
    /// # Safety
    /// Win32 메시지 파라미터가 유효해야 한다.
    pub(super) unsafe fn dispatch_size_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        _wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        // SAFETY: system이 message별로 유효한 hwnd와 인자를 제공한다.
        unsafe {
            match msg {
                WM_SIZE => {
                    let width = (lparam & 0xFFFF) as i32;
                    let height = ((lparam >> 16) & 0xFFFF) as i32;
                    if let Err(e) = self.resize(width, height) {
                        tracing::warn!("resize failed: {e}");
                    }
                    Some(0)
                }

                WM_ENTERSIZEMOVE => {
                    self.model.runtime.resizing = true;
                    Some(0)
                }

                WM_EXITSIZEMOVE => {
                    self.model.runtime.resizing = false;
                    // 드래그가 끝난 위치와 크기만 기억한다. 자석 모드가
                    // SetWindowPos로 옮긴 위치는 이 메시지를 만들지 않으므로
                    // 저장되지 않는다.
                    self.remember_window_placement(hwnd);
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
                        // 이동 전용 드래그 중 모니터를 건너간 경우(WM_DPICHANGED만 도착,
                        // pending_resize 없음) 여기가 새 DPI를 반영하는 유일한 시점이다.
                        // paint는 DPI가 바뀐 경우에만 캐시를 재구축하므로 평소에는 저렴하다.
                        if let Err(e) = self.paint() {
                            tracing::warn!("paint failed after size-move exit: {e}");
                        }
                    }
                    Some(0)
                }

                WM_DISPLAYCHANGE => {
                    // 합성 경로에서는 DComp/DXGI 가 모니터 변경에 자체 대응한다.
                    // 즉시 다시 그려주기만 해도 갱신 효과로 충분.
                    if let Err(e) = self.paint() {
                        tracing::warn!("paint failed on display change: {e}");
                    }
                    Some(0)
                }

                WM_DPICHANGED => {
                    // 권장 RECT 적용 후 WM_SIZE가 swap chain resize와 paint를 잇는다.
                    Self::apply_dpi_rect(hwnd, lparam);
                    if self.model.runtime.resizing {
                        // 인터랙티브 리사이즈 중이면 즉시 그리지 않는다 — 구식
                        // client_size에 새 DPI를 조합한 프레임이 나가고, 리사이즈에서
                        // 미룬 무거운 재구축이 드래그 도중 다시 돌아온다. apply_dpi_rect의
                        // SetWindowPos가 유발한 WM_SIZE는 pending_resize로 기록되고,
                        // 크기가 같아 기록이 없더라도 WM_EXITSIZEMOVE의 paint가 마무리한다.
                        return Some(0);
                    }
                    if !self.sync_client_size(hwnd)
                        && let Err(e) = self.paint()
                    {
                        // 권장 rect가 위치만 바꾸거나 같은 pixel 크기여도 render target
                        // DPI와 DPI 종속 cache는 반드시 갱신해야 한다.
                        tracing::warn!("paint failed on DPI change: {e}");
                    }
                    Some(0)
                }

                _ => None,
            }
        }
    }

    /// `WM_RBUTTONUP`/`WM_NCRBUTTONUP`, `WM_COMMAND`, `WM_HOTKEY`, `WM_TRAY_ICON` 처리.
    ///
    /// # Safety
    /// Win32 메시지 파라미터가 유효해야 한다.
    pub(super) unsafe fn dispatch_input_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        // SAFETY: system이 message별로 유효한 hwnd와 인자를 제공한다.
        unsafe {
            match msg {
                WM_RBUTTONUP | WM_NCRBUTTONUP => {
                    self.handle_right_click(hwnd, msg, lparam);
                    Some(0)
                }

                WM_COMMAND => {
                    let cmd = (wparam & 0xFFFF) as u16;
                    if let Err(e) = self.handle_menu_command(cmd) {
                        tracing::warn!("handle_menu_command failed: {e}");
                    }
                    Some(0)
                }

                WM_HOTKEY => {
                    let id = wparam as i32;
                    if let Err(e) = self.handle_hotkey(id) {
                        tracing::warn!("handle_hotkey failed: {e}");
                    }
                    Some(0)
                }

                WM_TRAY_ICON => {
                    self.handle_tray_event(lparam);
                    Some(0)
                }

                _ => None,
            }
        }
    }

    /// `WM_PAINT`, `WM_TIMER`(각 타이머 ID), `WM_CLIPBOARDUPDATE` 처리.
    ///
    /// # Safety
    /// Win32 메시지 파라미터가 유효해야 한다.
    pub(super) unsafe fn dispatch_timer_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Option<LRESULT> {
        // SAFETY: system이 message별로 유효한 hwnd와 인자를 제공한다.
        unsafe {
            match msg {
                WM_PAINT => {
                    let mut ps = PAINTSTRUCT::default();
                    let _ = BeginPaint(hwnd, &mut ps);
                    if let Err(e) = self.paint() {
                        tracing::warn!("paint failed on WM_PAINT: {e}");
                    }
                    let _ = EndPaint(hwnd, &ps);
                    Some(0)
                }

                WM_TIMER if wparam == COMPOSITION_RETRY_TIMER => {
                    let _ = KillTimer(hwnd, COMPOSITION_RETRY_TIMER);
                    self.composition_retry_scheduled = false;
                    if let Err(e) = self.paint() {
                        tracing::warn!("composition retry paint failed: {e}");
                    }
                    Some(0)
                }

                WM_TIMER if wparam == CLIPBOARD_DEBOUNCE_TIMER => {
                    self.handle_clipboard_debounce_timer();
                    Some(0)
                }

                WM_TIMER if wparam == CLIPBOARD_READ_RETRY_TIMER => {
                    let _ = KillTimer(hwnd, CLIPBOARD_READ_RETRY_TIMER);
                    self.handle_clipboard_change();
                    Some(0)
                }

                WM_TIMER if wparam == MAGNETIC_NOTICE_TIMER => {
                    self.handle_magnetic_notice_timer();
                    Some(0)
                }

                WM_TIMER if wparam == HOOK_MERGE_TIMER => {
                    self.handle_hook_merge_timer();
                    Some(0)
                }

                WM_CLIPBOARDUPDATE => {
                    self.handle_clipboard_change();
                    Some(0)
                }

                _ => None,
            }
        }
    }

    /// custom `WM_APP_*`/`WM_DEFERRED_*` 및 번역·업데이트 완료 message 처리.
    ///
    /// # Safety
    /// Win32 메시지 파라미터가 유효해야 한다.
    pub(super) unsafe fn dispatch_app_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Option<LRESULT> {
        match msg {
            _ if msg == WM_APP_REFRESH => {
                self.sync_window_state();
                if let Err(e) = self.paint() {
                    tracing::warn!("paint failed on refresh: {e}");
                }
                Some(0)
            }

            _ if msg == WM_APP_ACTION => {
                self.process_actions();
                Some(0)
            }

            _ if msg == WM_APP_SET_MAGNETIC => {
                self.apply_magnetic_request(wparam != 0);
                Some(0)
            }

            _ if msg == WM_APP_MAGNETIC_TARGET_SELECTED => {
                self.handle_magnetic_target_selected(wparam as HWND);
                Some(0)
            }

            _ if msg == WM_DEFERRED_RESIZE => {
                self.sync_client_size(hwnd);
                Some(0)
            }

            _ if msg == WM_DEFERRED_PAINT => {
                if let Err(error) = self.paint() {
                    tracing::warn!("deferred paint failed: {error}");
                }
                Some(0)
            }

            _ if msg == WM_TRANSLATION_COMPLETE => {
                self.handle_translation_complete();
                Some(0)
            }

            _ if msg == WM_UPDATE_RESULT => {
                self.handle_update_result();
                Some(0)
            }

            _ if msg == WM_UPDATE_PROGRESS => {
                self.handle_update_progress();
                Some(0)
            }

            _ if msg == WM_APP_HOOK_STATE => {
                self.handle_hook_state();
                Some(0)
            }

            _ => None,
        }
    }
}
