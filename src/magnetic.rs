//! WinEvent hook으로 주 창을 foreground target에 붙여 이동한다.
//! Callback은 등록 UI thread에서만 실행되므로 상태는 thread-local이다.

use std::cell::RefCell;
use std::ptr;

use windows::{
    Win32::{
        Foundation::*, System::Threading::GetCurrentProcessId, UI::Accessibility::*,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::app::messages::WM_APP_MAGNETIC_TARGET_SELECTED;

/// 자석 상태 (thread_local 보관)
struct MagneticState {
    main_hwnd: HWND,
    target_hwnd: HWND,
    selection_pending: bool,
    is_minimized: bool,
    offset_x: i32,
    offset_y: i32,
    minimize_with_target: bool,
    window_visible: bool,
}

thread_local! {
    static MAGNETIC_INSTANCE: RefCell<Option<MagneticState>> = const { RefCell::new(None) };
}

/// 자석 모드 관리자
pub struct MagneticManager {
    main_hwnd: HWND,
    event_hook: HWINEVENTHOOK,
    minimize_with_target: bool,
    window_visible: bool,
}

impl MagneticManager {
    /// 자석 모드 관리자 생성
    pub fn new(main_hwnd: HWND, minimize_with_target: bool, window_visible: bool) -> Self {
        Self {
            main_hwnd,
            event_hook: HWINEVENTHOOK::default(),
            minimize_with_target,
            window_visible,
        }
    }

    /// AppModel의 최신 정책 snapshot을 callback 상태에 반영한다.
    pub fn update_policy(&mut self, minimize_with_target: bool, window_visible: bool) {
        self.minimize_with_target = minimize_with_target;
        self.window_visible = window_visible;
        MAGNETIC_INSTANCE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.minimize_with_target = minimize_with_target;
                state.window_visible = window_visible;
            }
        });
    }

    /// 자석 대상 선택을 기다리기 시작한다.
    pub fn start(&mut self) -> Result<()> {
        if !self.event_hook.0.is_null() {
            return Ok(());
        }

        MAGNETIC_INSTANCE.with(|cell| {
            *cell.borrow_mut() = Some(MagneticState {
                main_hwnd: self.main_hwnd,
                target_hwnd: HWND::default(),
                selection_pending: false,
                is_minimized: false,
                offset_x: 0,
                offset_y: 0,
                minimize_with_target: self.minimize_with_target,
                window_visible: self.window_visible,
            });
        });

        // SAFETY: event 범위와 callback이 유효하며 callback은 등록 thread에서 실행된다.
        unsafe {
            self.event_hook = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_OBJECT_LOCATIONCHANGE,
                None,
                Some(Self::win_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
        }

        if self.event_hook.0.is_null() {
            MAGNETIC_INSTANCE.with(|cell| *cell.borrow_mut() = None);
            // SAFETY: GetLastError returns the last Win32 error code for the current thread.
            return Err(Error::from_hresult(HRESULT::from_win32(unsafe {
                GetLastError().0
            })));
        }

        Ok(())
    }

    /// 사용자가 활성화한 외부 창을 실제 자석 대상으로 연결한다.
    pub fn attach(&mut self, target: HWND) -> Result<()> {
        if !is_external_target_window(target) {
            return Err(Error::from_hresult(HRESULT::from_win32(
                ERROR_INVALID_WINDOW_HANDLE.0,
            )));
        }
        let (offset_x, offset_y) = self.calculate_offset(target)?;
        MAGNETIC_INSTANCE.with(|cell| {
            let mut state = cell.borrow_mut();
            let state = state.as_mut().ok_or_else(|| {
                Error::from_hresult(HRESULT::from_win32(ERROR_INVALID_WINDOW_HANDLE.0))
            })?;
            state.target_hwnd = target;
            state.selection_pending = false;
            state.is_minimized = false;
            state.offset_x = offset_x;
            state.offset_y = offset_y;
            Ok(())
        })
    }

    /// 자석 모드 중지
    pub fn stop(&mut self) {
        if !self.event_hook.0.is_null() {
            // SAFETY: self.event_hook is a valid hook handle from SetWinEventHook.
            unsafe {
                let _ = UnhookWinEvent(self.event_hook);
            }
            self.event_hook = HWINEVENTHOOK(ptr::null_mut());
        }

        MAGNETIC_INSTANCE.with(|cell| *cell.borrow_mut() = None);

        // SAFETY: self.main_hwnd is a valid window handle provided during construction.
        if self.window_visible {
            unsafe {
                let _ = ShowWindow(self.main_hwnd, SW_SHOW);
            }
        }
    }

    /// 현재 위치로 오프셋 계산
    fn calculate_offset(&self, target: HWND) -> Result<(i32, i32)> {
        // SAFETY: 두 hwnd와 출력 RECT가 유효하다.
        unsafe {
            let mut target_rect = RECT::default();
            let mut main_rect = RECT::default();

            GetWindowRect(target, &mut target_rect)?;
            GetWindowRect(self.main_hwnd, &mut main_rect)?;

            let offset_x = main_rect.left - target_rect.left;
            let offset_y = main_rect.top - target_rect.top;

            Ok((offset_x, offset_y))
        }
    }

    /// WinEvent 콜백
    // SAFETY: system이 유효한 인자를 등록 thread에 전달한다.
    unsafe extern "system" fn win_event_proc(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        id_object: i32,
        _id_child: i32,
        _id_event_thread: u32,
        _dwms_event_time: u32,
    ) {
        if id_object != 0 {
            return;
        }

        MAGNETIC_INSTANCE.with(|cell| {
            let Ok(mut state) = cell.try_borrow_mut() else {
                return;
            };
            let state = match state.as_mut() {
                Some(s) => s,
                None => return,
            };

            match event {
                EVENT_SYSTEM_FOREGROUND
                    if state.target_hwnd.is_invalid()
                        && !state.selection_pending
                        && is_external_target_window(hwnd) =>
                {
                    state.selection_pending = true;
                    // SAFETY: main HWND는 manager가 소유하고 target HWND 값은 message에
                    // 복사되어 UI thread에서 검증 후 사용된다.
                    if unsafe {
                        PostMessageW(
                            Some(state.main_hwnd),
                            WM_APP_MAGNETIC_TARGET_SELECTED,
                            WPARAM(hwnd.0 as usize),
                            LPARAM(0),
                        )
                    }
                    .is_err()
                    {
                        state.selection_pending = false;
                    }
                }

                EVENT_OBJECT_LOCATIONCHANGE if hwnd == state.target_hwnd && !state.is_minimized => {
                    let mut target_rect = RECT::default();
                    // SAFETY: Called within unsafe extern "system" fn
                    if unsafe { GetWindowRect(state.target_hwnd, &mut target_rect).is_ok() } {
                        let new_x = target_rect.left + state.offset_x;
                        let new_y = target_rect.top + state.offset_y;
                        // SAFETY: Called within unsafe extern "system" fn
                        unsafe {
                            let _ = SetWindowPos(
                                state.main_hwnd,
                                None,
                                new_x,
                                new_y,
                                0,
                                0,
                                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                            );
                        }
                    }
                }

                EVENT_SYSTEM_MINIMIZESTART
                    if hwnd == state.target_hwnd && state.minimize_with_target =>
                {
                    state.is_minimized = true;
                    // SAFETY: Called within unsafe extern "system" fn
                    unsafe {
                        let _ = ShowWindow(state.main_hwnd, SW_HIDE);
                    }
                }

                EVENT_SYSTEM_MINIMIZEEND if hwnd == state.target_hwnd && state.is_minimized => {
                    state.is_minimized = false;
                    // SAFETY: Called within unsafe extern "system" fn
                    if state.window_visible {
                        unsafe {
                            let _ = ShowWindow(state.main_hwnd, SW_SHOWNOACTIVATE);
                        }
                    }

                    let mut target_rect = RECT::default();
                    // SAFETY: Called within unsafe extern "system" fn
                    if unsafe { GetWindowRect(state.target_hwnd, &mut target_rect).is_ok() } {
                        let new_x = target_rect.left + state.offset_x;
                        let new_y = target_rect.top + state.offset_y;
                        // SAFETY: Called within unsafe extern "system" fn
                        unsafe {
                            let _ = SetWindowPos(
                                state.main_hwnd,
                                None,
                                new_x,
                                new_y,
                                0,
                                0,
                                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                            );
                        }
                    }
                }

                _ => {}
            }
        });
    }
}

fn is_external_target_window(candidate: HWND) -> bool {
    unsafe {
        if candidate.is_invalid() {
            return false;
        }
        let own_pid = GetCurrentProcessId();
        let shell_hwnd = GetShellWindow();
        let mut candidate_pid = 0;
        GetWindowThreadProcessId(candidate, Some(&mut candidate_pid));
        candidate_pid != 0
            && candidate_pid != own_pid
            && IsWindowVisible(candidate).as_bool()
            && candidate != shell_hwnd
            && is_targetable_extended_style(GetWindowLongW(candidate, GWL_EXSTYLE) as u32)
            && has_window_area(candidate)
    }
}

fn is_targetable_extended_style(style: u32) -> bool {
    style & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) == 0
}

fn has_window_area(hwnd: HWND) -> bool {
    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &mut rect).is_ok() && rect.right > rect.left && rect.bottom > rect.top
    }
}

impl Drop for MagneticManager {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_style_rejects_shell_and_nonactivating_windows() {
        assert!(is_targetable_extended_style(0));
        assert!(!is_targetable_extended_style(WS_EX_TOOLWINDOW.0));
        assert!(!is_targetable_extended_style(WS_EX_NOACTIVATE.0));
        assert!(!is_targetable_extended_style(
            WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0
        ));
    }
}
