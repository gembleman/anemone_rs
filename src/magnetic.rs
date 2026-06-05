//! 자석 모드
//!
//! 게임 윈도우에 부착되어 함께 이동하는 기능.
//! WinEvent 훅을 사용하여 타겟 윈도우의 위치 변화를 감지.
//!
//! 단일 thread_local `MagneticState` 가 진리의 원천. WinEvent 콜백은
//! `WINEVENT_OUTOFCONTEXT` 로 등록되어 등록 스레드(메인 UI 스레드)에서만
//! 호출되므로 thread_local 접근이 안전하다.

use std::cell::RefCell;
use std::ptr;
use std::rc::Rc;

use windows::{
    Win32::{Foundation::*, UI::Accessibility::*, UI::WindowsAndMessaging::*},
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
    minimize_with_target: bool,
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

        // SAFETY: GetForegroundWindow returns a valid HWND or null (checked below).
        let target = unsafe { GetForegroundWindow() };
        if target.is_invalid() || target == self.main_hwnd {
            return Err(Error::from_hresult(HRESULT::from_win32(
                ERROR_INVALID_WINDOW_HANDLE.0,
            )));
        }

        let (offset_x, offset_y) = self.calculate_offset(target)?;
        let minimize_with_target = self.config.borrow().magnetic_minimize;

        MAGNETIC_INSTANCE.with(|cell| {
            *cell.borrow_mut() = Some(MagneticState {
                main_hwnd: self.main_hwnd,
                target_hwnd: target,
                is_minimized: false,
                offset_x,
                offset_y,
                minimize_with_target,
            });
        });

        // SAFETY: SetWinEventHook is called with valid event range and a valid callback function
        // pointer. WINEVENT_OUTOFCONTEXT means the callback runs in our thread context.
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
        unsafe {
            let _ = ShowWindow(self.main_hwnd, SW_SHOW);
        }
    }

    /// 현재 위치로 오프셋 계산
    fn calculate_offset(&self, target: HWND) -> Result<(i32, i32)> {
        // SAFETY: target and self.main_hwnd are valid window handles. GetWindowRect writes
        // to properly initialized RECT structs.
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
    // SAFETY: This is a WinEvent callback registered via SetWinEventHook. The system
    // guarantees valid parameters. Thread-local data is accessed only from the registering
    // thread (WINEVENT_OUTOFCONTEXT).
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
            let Ok(mut state) = cell.try_borrow_mut() else { return; };
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
                    unsafe {
                        let _ = ShowWindow(state.main_hwnd, SW_SHOWNOACTIVATE);
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

impl Drop for MagneticManager {
    fn drop(&mut self) {
        self.stop();
    }
}
