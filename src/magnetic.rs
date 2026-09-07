//! WinEvent hook으로 주 창을 foreground target에 붙여 이동한다.
//! Callback은 등록 UI thread에서만 실행되므로 상태는 thread-local이다.

use std::cell::RefCell;
use std::io;
use std::ptr;

use windows_sys::Win32::{
    Foundation::{ERROR_INVALID_WINDOW_HANDLE, GetLastError, HWND, LPARAM, RECT, WPARAM},
    System::Threading::GetCurrentProcessId,
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        WindowsAndMessaging::{
            EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND,
            EVENT_SYSTEM_MINIMIZESTART, GWL_EXSTYLE, GetShellWindow, GetWindowLongW, GetWindowRect,
            GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SW_HIDE, SW_SHOW,
            SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, ShowWindow,
            WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        },
    },
};

use crate::app::messages::{WM_APP_MAGNETIC_REPOSITION, WM_APP_MAGNETIC_TARGET_SELECTED};

/// 자석 상태 (thread_local 보관)
struct MagneticState {
    main_hwnd: HWND,
    target_hwnd: HWND,
    selection_pending: bool,
    reposition_pending: bool,
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
            event_hook: std::ptr::null_mut(),
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
    pub fn start(&mut self) -> io::Result<()> {
        if !self.event_hook.is_null() {
            return Ok(());
        }

        MAGNETIC_INSTANCE.with(|cell| {
            *cell.borrow_mut() = Some(MagneticState {
                main_hwnd: self.main_hwnd,
                target_hwnd: std::ptr::null_mut(),
                selection_pending: false,
                reposition_pending: false,
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
                std::ptr::null_mut(),
                Some(Self::win_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
        }

        if self.event_hook.is_null() {
            MAGNETIC_INSTANCE.with(|cell| *cell.borrow_mut() = None);
            // SAFETY: GetLastError returns the last Win32 error code for the current thread.
            return Err(io::Error::from_raw_os_error(
                unsafe { GetLastError() } as i32
            ));
        }

        Ok(())
    }

    /// 사용자가 활성화한 외부 창을 실제 자석 대상으로 연결한다.
    pub fn attach(&mut self, target: HWND) -> io::Result<()> {
        if !is_external_target_window(target) {
            return Err(io::Error::from_raw_os_error(
                ERROR_INVALID_WINDOW_HANDLE as i32,
            ));
        }
        let (offset_x, offset_y) = self.calculate_offset(target)?;
        MAGNETIC_INSTANCE.with(|cell| {
            let mut state = cell.borrow_mut();
            let state = state
                .as_mut()
                .ok_or_else(|| io::Error::from_raw_os_error(ERROR_INVALID_WINDOW_HANDLE as i32))?;
            state.target_hwnd = target;
            state.selection_pending = false;
            state.reposition_pending = false;
            state.is_minimized = false;
            state.offset_x = offset_x;
            state.offset_y = offset_y;
            Ok(())
        })
    }

    /// WinEvent 폭주 중 합쳐 둔 마지막 위치를 한 번 반영한다.
    pub fn apply_pending_reposition(&mut self) {
        MAGNETIC_INSTANCE.with(|cell| {
            let mut state = cell.borrow_mut();
            let Some(state) = state.as_mut() else {
                return;
            };
            state.reposition_pending = false;
            if !state.is_minimized && !state.target_hwnd.is_null() {
                reposition_main_to_target(state);
            }
        });
    }

    /// 자석 모드 중지
    pub fn stop(&mut self) {
        if !self.event_hook.is_null() {
            // SAFETY: self.event_hook is a valid hook handle from SetWinEventHook.
            unsafe {
                let _ = UnhookWinEvent(self.event_hook);
            }
            self.event_hook = ptr::null_mut();
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
    fn calculate_offset(&self, target: HWND) -> io::Result<(i32, i32)> {
        // SAFETY: 두 hwnd와 출력 RECT가 유효하다.
        unsafe {
            let mut target_rect: RECT = std::mem::zeroed();
            let mut main_rect: RECT = std::mem::zeroed();

            if GetWindowRect(target, &mut target_rect) == 0 {
                return Err(io::Error::last_os_error());
            }
            if GetWindowRect(self.main_hwnd, &mut main_rect) == 0 {
                return Err(io::Error::last_os_error());
            }

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
            let Some(state) = state.as_mut() else {
                return;
            };

            match event {
                EVENT_SYSTEM_FOREGROUND => try_select_foreground_target(state, hwnd),
                EVENT_OBJECT_LOCATIONCHANGE => on_target_location_changed(state, hwnd),
                EVENT_SYSTEM_MINIMIZESTART => on_target_minimize_start(state, hwnd),
                EVENT_SYSTEM_MINIMIZEEND => on_target_minimize_end(state, hwnd),
                _ => {}
            }
        });
    }
}

/// 아직 자석 대상이 없을 때, 사용자가 전면으로 올린 외부 창을 후보로 선정한다.
/// 실제 연결은 UI thread가 message를 받아 검증 후 `attach`로 확정한다.
fn try_select_foreground_target(state: &mut MagneticState, hwnd: HWND) {
    if !state.target_hwnd.is_null() || state.selection_pending || !is_external_target_window(hwnd) {
        return;
    }

    state.selection_pending = true;
    // SAFETY: main HWND는 manager가 소유하고 target HWND 값은 message에
    // 복사되어 UI thread에서 검증 후 사용된다.
    if unsafe {
        PostMessageW(
            state.main_hwnd,
            WM_APP_MAGNETIC_TARGET_SELECTED,
            hwnd as WPARAM,
            0 as LPARAM,
        )
    } == 0
    {
        state.selection_pending = false;
    }
}

/// 자석 대상이 움직이면 저장해둔 오프셋만큼 주 창을 같이 옮긴다.
fn on_target_location_changed(state: &mut MagneticState, hwnd: HWND) {
    if hwnd != state.target_hwnd || state.is_minimized || state.reposition_pending {
        return;
    }
    state.reposition_pending = true;
    if unsafe { PostMessageW(state.main_hwnd, WM_APP_MAGNETIC_REPOSITION, 0, 0) } == 0 {
        state.reposition_pending = false;
    }
}

/// 자석 대상이 최소화되면(정책이 켜져 있을 때) 주 창도 함께 숨긴다.
fn on_target_minimize_start(state: &mut MagneticState, hwnd: HWND) {
    if hwnd != state.target_hwnd || !state.minimize_with_target {
        return;
    }
    state.is_minimized = true;
    // SAFETY: Called within unsafe extern "system" fn
    unsafe {
        let _ = ShowWindow(state.main_hwnd, SW_HIDE);
    }
}

/// 자석 대상이 복원되면 주 창을 다시 보이고 최신 위치로 맞춘다.
fn on_target_minimize_end(state: &mut MagneticState, hwnd: HWND) {
    if hwnd != state.target_hwnd || !state.is_minimized {
        return;
    }
    state.is_minimized = false;
    // SAFETY: Called within unsafe extern "system" fn
    if state.window_visible {
        unsafe {
            let _ = ShowWindow(state.main_hwnd, SW_SHOWNOACTIVATE);
        }
    }
    reposition_main_to_target(state);
}

/// 대상 창의 현재 위치 + 저장된 오프셋으로 주 창을 옮긴다.
fn reposition_main_to_target(state: &MagneticState) {
    let mut target_rect = RECT::default();
    // SAFETY: Called within unsafe extern "system" fn
    if unsafe { GetWindowRect(state.target_hwnd, &mut target_rect) != 0 } {
        let new_x = target_rect.left + state.offset_x;
        let new_y = target_rect.top + state.offset_y;
        // SAFETY: Called within unsafe extern "system" fn
        unsafe {
            let _ = SetWindowPos(
                state.main_hwnd,
                std::ptr::null_mut(),
                new_x,
                new_y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

fn is_external_target_window(candidate: HWND) -> bool {
    unsafe {
        if candidate.is_null() {
            return false;
        }
        let own_pid = GetCurrentProcessId();
        let shell_hwnd = GetShellWindow();
        let mut candidate_pid = 0;
        GetWindowThreadProcessId(candidate, &mut candidate_pid);
        candidate_pid != 0
            && candidate_pid != own_pid
            && IsWindowVisible(candidate) != 0
            && candidate != shell_hwnd
            && is_targetable_extended_style(GetWindowLongW(candidate, GWL_EXSTYLE) as u32)
            && has_window_area(candidate)
    }
}

fn is_targetable_extended_style(style: u32) -> bool {
    style & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) == 0
}

fn has_window_area(hwnd: HWND) -> bool {
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    unsafe {
        GetWindowRect(hwnd, &mut rect) != 0 && rect.right > rect.left && rect.bottom > rect.top
    }
}

impl Drop for MagneticManager {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[path = "../tests/unit/magnetic.rs"]
mod tests;
