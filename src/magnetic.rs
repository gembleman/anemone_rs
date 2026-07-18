//! WinEvent hook으로 주 창을 foreground target에 붙여 이동한다.
//! Callback은 등록 UI thread에서만 실행되므로 상태는 thread-local이다.

use std::cell::RefCell;
use std::ptr;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::*, System::Threading::GetCurrentProcessId, UI::Accessibility::*,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::config::Config;

/// 자석 상태 (thread_local 보관)
struct MagneticState {
    main_hwnd: HWND,
    target_hwnd: HWND,
    is_minimized: bool,
    offset_x: i32,
    offset_y: i32,
    config: Rc<RefCell<Config>>,
}

thread_local! {
    static MAGNETIC_INSTANCE: RefCell<Option<MagneticState>> = const { RefCell::new(None) };
}

/// 자석 모드 관리자
pub struct MagneticManager {
    main_hwnd: HWND,
    event_hook: HWINEVENTHOOK,
    config: Rc<RefCell<Config>>,
}

impl MagneticManager {
    /// 자석 모드 관리자 생성
    pub fn new(main_hwnd: HWND, config: Rc<RefCell<Config>>) -> Self {
        Self {
            main_hwnd,
            event_hook: HWINEVENTHOOK::default(),
            config,
        }
    }

    /// 자석 모드 시작
    pub fn start(&mut self) -> Result<()> {
        if !self.event_hook.0.is_null() {
            return Ok(());
        }

        let Some(target) = find_external_target_window() else {
            return Err(Error::from_hresult(HRESULT::from_win32(
                ERROR_INVALID_WINDOW_HANDLE.0,
            )));
        };

        let (offset_x, offset_y) = self.calculate_offset(target)?;

        MAGNETIC_INSTANCE.with(|cell| {
            *cell.borrow_mut() = Some(MagneticState {
                main_hwnd: self.main_hwnd,
                target_hwnd: target,
                is_minimized: false,
                offset_x,
                offset_y,
                config: self.config.clone(),
            });
        });

        // SAFETY: event 범위와 callback이 유효하며 callback은 등록 thread에서 실행된다.
        unsafe {
            self.event_hook = SetWinEventHook(
                EVENT_SYSTEM_MINIMIZESTART,
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
        if self.config.borrow().window_visible {
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
                    if hwnd == state.target_hwnd && state.config.borrow().magnetic_minimize =>
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
                    if state.config.borrow().window_visible {
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

/// 현재 foreground가 앱 자신의 메뉴/대화상자여도 z-order 아래의 첫 외부 top-level
/// 창을 찾는다. 메뉴와 설정창이 foreground를 가져간 뒤 자석 모드를 켜는 경로를 함께 지원한다.
fn find_external_target_window() -> Option<HWND> {
    unsafe {
        let own_pid = GetCurrentProcessId();
        let mut candidate = GetForegroundWindow();
        while !candidate.is_invalid() {
            let mut candidate_pid = 0;
            GetWindowThreadProcessId(candidate, Some(&mut candidate_pid));
            if candidate_pid != 0
                && candidate_pid != own_pid
                && IsWindowVisible(candidate).as_bool()
            {
                return Some(candidate);
            }
            candidate = GetWindow(candidate, GW_HWNDNEXT).ok()?;
        }
        None
    }
}

impl Drop for MagneticManager {
    fn drop(&mut self) {
        self.stop();
    }
}
