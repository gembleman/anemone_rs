use super::*;

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
                WM_DESTROY => {
                    if let Err(e) = self.config.borrow().save() {
                        tracing::error!("설정 저장 실패: {}", e);
                    }
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
                    if lparam.0 != 0 {
                        let rect = &*(lparam.0 as *const RECT);
                        let w = rect.right - rect.left;
                        let h = rect.bottom - rect.top;
                        let _ = SetWindowPos(
                            hwnd,
                            None,
                            rect.left,
                            rect.top,
                            w,
                            h,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
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

                WM_CLIPBOARDUPDATE => {
                    self.handle_clipboard_change();
                    Some(LRESULT(0))
                }

                _ if msg == WM_DEFERRED_CLIPBOARD => {
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

                _ if msg == WM_TRANSLATION_COMPLETE => {
                    self.handle_translation_complete(wparam.0 as u64);
                    Some(LRESULT(0))
                }

                _ => None,
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

    // SAFETY: This is a Win32 window procedure callback. The system guarantees hwnd is valid
    // and msg/wparam/lparam contain valid message data when called.
    pub(super) unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let app = APP.with(|cell| cell.try_borrow().ok().and_then(|g| g.clone()));

        if let Some(app) = app {
            // TaskbarCreated 메시지 체크
            if let Ok(app_ref) = app.try_borrow() {
                let taskbar_msg = app_ref.taskbar_created_msg;
                drop(app_ref);
                if msg == taskbar_msg {
                    if let Ok(mut app_ref) = app.try_borrow_mut() {
                        app_ref.tray.restore();
                    }
                    return LRESULT(0);
                }
            }

            // App 인스턴스 불필요한 메시지 처리
            // SAFETY: hwnd/lparam are valid system-provided parameters.
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
                        // DComp 합성 경로 회귀 보완: 투명 배경 모드에서 텍스트
                        // 라인 사각형 밖이면 HTTRANSPARENT 로 클릭 통과.
                        // try_borrow 실패 (paint 등 mutable borrow 진행 중) 시는
                        // 안전한 fallback 으로 HTCAPTION 유지. hit_region 이
                        // 비어 있으면 (배경 표시 또는 텍스트 없음) 기존 동작.
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

            // App 인스턴스가 필요한 메시지: dispatch_message로 위임
            // WM_CLIPBOARDUPDATE는 borrow 실패 시 지연 처리
            if msg == WM_CLIPBOARDUPDATE {
                if let Ok(mut app_ref) = app.try_borrow_mut() {
                    // SAFETY: Valid system parameters forwarded to dispatch_message.
                    if let Some(result) =
                        unsafe { app_ref.dispatch_message(hwnd, msg, wparam, lparam) }
                    {
                        return result;
                    }
                } else {
                    unsafe {
                        let _ =
                            PostMessageW(Some(hwnd), WM_DEFERRED_CLIPBOARD, WPARAM(0), LPARAM(0));
                    }
                    return LRESULT(0);
                }
            } else if let Ok(mut app_ref) = app.try_borrow_mut() {
                // SAFETY: Valid system parameters forwarded to dispatch_message.
                if let Some(result) = unsafe { app_ref.dispatch_message(hwnd, msg, wparam, lparam) }
                {
                    return result;
                }
            } else {
                // borrow_mut 실패 = wndproc 재진입 (예: dispatch_message 처리 중에
                // Win32 가 동기 send 한 메시지). 종료 메시지만은 fallback 처리해
                // 메시지 루프가 멈추지 않도록 보장.
                if msg == WM_DESTROY {
                    tracing::warn!(
                        "WM_DESTROY arrived during wndproc reentry (App borrowed) — \
                         posting quit directly"
                    );
                    unsafe { PostQuitMessage(0) };
                    return LRESULT(0);
                }
            }
        }

        // SAFETY: Forwarding valid system-provided parameters to DefWindowProcW.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}
