//! 자석 모드
//!
//! 게임 윈도우에 부착되어 함께 이동하는 기능.
//! WinEvent 훅을 사용하여 타겟 윈도우의 위치 변화를 감지.

use std::cell::RefCell;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::{
    core::*,
    Win32::{
        Foundation::*,
        UI::Accessibility::*,
        UI::WindowsAndMessaging::*,
    },
};

use crate::config::Config;

// WinEvent 상수 (windows crate에서 누락될 수 있음)
const EVENT_SYSTEM_MINIMIZESTART: u32 = 0x0016;
const EVENT_SYSTEM_MINIMIZEEND: u32 = 0x0017;
const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const EVENT_OBJECT_LOCATIONCHANGE: u32 = 0x800B;
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;

/// 자석 상태
pub struct MagnetState {
    /// 부착 대상 윈도우
    pub target_hwnd: HWND,
    /// 부착 여부
    pub is_attached: AtomicBool,
    /// 타겟 최소화 여부
    pub is_minimized: AtomicBool,
    /// X 오프셋 (타겟 윈도우 좌상단 기준)
    pub offset_x: i32,
    /// Y 오프셋
    pub offset_y: i32,
    /// 최소화 전 X 좌표
    pub saved_x: i32,
    /// 최소화 전 Y 좌표
    pub saved_y: i32,
}

impl Default for MagnetState {
    fn default() -> Self {
        Self {
            target_hwnd: HWND::default(),
            is_attached: AtomicBool::new(false),
            is_minimized: AtomicBool::new(false),
            offset_x: 0,
            offset_y: 0,
            saved_x: 0,
            saved_y: 0,
        }
    }
}

/// 자석 모드 관리자
pub struct MagneticManager {
    main_hwnd: HWND,
    state: Rc<RefCell<MagnetState>>,
    event_hook: HWINEVENTHOOK,
    #[allow(dead_code)]
    config: Rc<RefCell<Config>>,
}

thread_local! {
    static MAGNETIC_INSTANCE: RefCell<Option<MagneticManagerData>> = const { RefCell::new(None) };
}

/// thread_local에 저장할 데이터 (Rc 없이)
struct MagneticManagerData {
    main_hwnd: HWND,
    target_hwnd: HWND,
    is_attached: bool,
    is_minimized: bool,
    offset_x: i32,
    offset_y: i32,
    saved_x: i32,
    saved_y: i32,
    minimize_with_target: bool,
}

impl MagneticManager {
    /// 자석 모드 관리자 생성
    pub fn new(main_hwnd: HWND, config: Rc<RefCell<Config>>) -> Self {
        Self {
            main_hwnd,
            state: Rc::new(RefCell::new(MagnetState::default())),
            event_hook: HWINEVENTHOOK::default(),
            config,
        }
    }

    /// 자석 모드 시작
    pub fn start(&mut self) -> Result<()> {
        // 이미 시작된 경우
        if !self.event_hook.0.is_null() {
            return Ok(());
        }

        // 포그라운드 윈도우를 타겟으로 설정
        let target = unsafe { GetForegroundWindow() };
        if target.is_invalid() || target == self.main_hwnd {
            return Err(Error::from_hresult(HRESULT::from_win32(ERROR_INVALID_WINDOW_HANDLE.0)));
        }

        // 현재 위치로 오프셋 계산
        let (offset_x, offset_y) = self.calculate_offset(target)?;

        // 상태 설정
        {
            let mut state = self.state.borrow_mut();
            state.target_hwnd = target;
            state.is_attached.store(true, Ordering::SeqCst);
            state.is_minimized.store(false, Ordering::SeqCst);
            state.offset_x = offset_x;
            state.offset_y = offset_y;
        }

        // thread_local 데이터 설정
        let minimize_with_target = self.config.borrow().magnetic_minimize;
        MAGNETIC_INSTANCE.with(|cell| {
            *cell.borrow_mut() = Some(MagneticManagerData {
                main_hwnd: self.main_hwnd,
                target_hwnd: target,
                is_attached: true,
                is_minimized: false,
                offset_x,
                offset_y,
                saved_x: 0,
                saved_y: 0,
                minimize_with_target,
            });
        });

        // WinEvent 훅 설정
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
            return Err(Error::from_hresult(HRESULT::from_win32(unsafe { GetLastError().0 })));
        }

        Ok(())
    }

    /// 자석 모드 중지
    pub fn stop(&mut self) {
        if !self.event_hook.0.is_null() {
            unsafe {
                let _ = UnhookWinEvent(self.event_hook);
            }
            self.event_hook = HWINEVENTHOOK(ptr::null_mut());
        }

        // 상태 초기화
        {
            let mut state = self.state.borrow_mut();
            state.is_attached.store(false, Ordering::SeqCst);
            state.target_hwnd = HWND::default();
        }

        // thread_local 데이터 정리
        MAGNETIC_INSTANCE.with(|cell| {
            *cell.borrow_mut() = None;
        });

        // 윈도우 다시 표시 (숨겨져 있었다면)
        unsafe {
            let _ = ShowWindow(self.main_hwnd, SW_SHOW);
        }
    }

    /// 부착 여부
    pub fn is_attached(&self) -> bool {
        self.state.borrow().is_attached.load(Ordering::SeqCst)
    }

    /// 타겟 윈도우 핸들
    #[allow(dead_code)]
    pub fn target_hwnd(&self) -> HWND {
        self.state.borrow().target_hwnd
    }

    /// 현재 위치로 오프셋 계산
    fn calculate_offset(&self, target: HWND) -> Result<(i32, i32)> {
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

    /// 수동 위치 동기화
    pub fn sync_position(&self) {
        let state = self.state.borrow();
        if !state.is_attached.load(Ordering::SeqCst) {
            return;
        }

        unsafe {
            let mut target_rect = RECT::default();
            if GetWindowRect(state.target_hwnd, &mut target_rect).is_ok() {
                let new_x = target_rect.left + state.offset_x;
                let new_y = target_rect.top + state.offset_y;

                let _ = SetWindowPos(
                    self.main_hwnd,
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

    /// WinEvent 콜백
    unsafe extern "system" fn win_event_proc(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        id_object: i32,
        _id_child: i32,
        _id_event_thread: u32,
        _dwms_event_time: u32,
    ) {
        // OBJID_WINDOW만 처리
        if id_object != 0 {
            return;
        }

        MAGNETIC_INSTANCE.with(|cell| {
            let mut data = cell.borrow_mut();
            let data = match data.as_mut() {
                Some(d) => d,
                None => return,
            };

            if !data.is_attached {
                return;
            }

            match event {
                EVENT_OBJECT_LOCATIONCHANGE => {
                    // 타겟 윈도우 이동
                    if hwnd == data.target_hwnd && !data.is_minimized {
                        let mut target_rect = RECT::default();
                        // SAFETY: Called within unsafe extern "system" fn
                        if unsafe { GetWindowRect(data.target_hwnd, &mut target_rect).is_ok() } {
                            let new_x = target_rect.left + data.offset_x;
                            let new_y = target_rect.top + data.offset_y;

                            // SAFETY: Called within unsafe extern "system" fn
                            unsafe {
                                let _ = SetWindowPos(
                                    data.main_hwnd,
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
                }

                EVENT_SYSTEM_MINIMIZESTART => {
                    // 타겟 윈도우 최소화
                    if hwnd == data.target_hwnd && data.minimize_with_target {
                        data.is_minimized = true;

                        // 현재 위치 저장
                        let mut rect = RECT::default();
                        // SAFETY: Called within unsafe extern "system" fn
                        if unsafe { GetWindowRect(data.main_hwnd, &mut rect).is_ok() } {
                            data.saved_x = rect.left;
                            data.saved_y = rect.top;
                        }

                        // 윈도우 숨기기
                        // SAFETY: Called within unsafe extern "system" fn
                        unsafe {
                            let _ = ShowWindow(data.main_hwnd, SW_HIDE);
                        }
                    }
                }

                EVENT_SYSTEM_MINIMIZEEND => {
                    // 타겟 윈도우 복원
                    if hwnd == data.target_hwnd && data.is_minimized {
                        data.is_minimized = false;

                        // 윈도우 다시 표시
                        // SAFETY: Called within unsafe extern "system" fn
                        unsafe {
                            let _ = ShowWindow(data.main_hwnd, SW_SHOWNOACTIVATE);
                        }

                        // 위치 동기화
                        let mut target_rect = RECT::default();
                        // SAFETY: Called within unsafe extern "system" fn
                        if unsafe { GetWindowRect(data.target_hwnd, &mut target_rect).is_ok() } {
                            let new_x = target_rect.left + data.offset_x;
                            let new_y = target_rect.top + data.offset_y;

                            // SAFETY: Called within unsafe extern "system" fn
                            unsafe {
                                let _ = SetWindowPos(
                                    data.main_hwnd,
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
                }

                EVENT_SYSTEM_FOREGROUND => {
                    // 새 포그라운드 윈도우 (선택적 처리)
                    // 현재는 무시 - 타겟 전환 기능 필요 시 구현
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
