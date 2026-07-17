use super::*;

impl App {
    fn show_context_menu(&mut self, x: i32, y: i32) -> Result<()> {
        self.menu.build(&self.config.borrow())?;
        self.menu.show(self.hwnd, x, y)
    }

    pub(super) fn handle_menu_command(&mut self, cmd: u16) -> Result<()> {
        match cmd {
            menu::id::WINDOW_SHOW => {
                let mut cfg = self.config.borrow_mut();
                cfg.toggle_window_visible();
                let visible = cfg.window_visible;
                drop(cfg);
                window::set_window_visible(self.hwnd, visible);
            }
            menu::id::CLICK_THROUGH => {
                let mut cfg = self.config.borrow_mut();
                cfg.toggle_click_through();
                let click_through = cfg.click_through;
                drop(cfg);
                window::set_click_through(self.hwnd, click_through);
            }
            menu::id::CLIPBOARD_WATCH => {
                let mut cfg = self.config.borrow_mut();
                cfg.toggle_clipboard_watch();
                let watch = cfg.clipboard_watch;
                drop(cfg);
                if watch {
                    self.clipboard.start();
                } else {
                    self.clipboard.stop();
                }
            }
            menu::id::BACKGROUND_TOGGLE => {
                self.config.borrow_mut().toggle_background_visible();
                self.paint()?;
            }
            menu::id::BORDER_TOGGLE => {
                self.config.borrow_mut().toggle_border_visible();
                self.paint()?;
            }
            menu::id::MAGNETIC_MODE => {
                self.toggle_magnetic_mode();
            }
            menu::id::SETTINGS => {
                self.open_settings_dialog();
            }
            menu::id::TRANSLATE => {
                self.open_translate_dialog();
            }
            menu::id::BACKLOG => {
                self.open_backlog_dialog();
            }
            menu::id::FILE_TRANS => {
                self.open_file_trans_dialog();
            }
            menu::id::HOOK_SETTINGS => {
                self.open_hook_settings_dialog();
            }
            menu::id::TEXT_SIZE_UP => {
                let mut cfg = self.config.borrow_mut();
                let new_size = (cfg.translation_style.size + 1).min(100);
                cfg.translation_style.size = new_size;
                cfg.name_style.size = new_size;
                cfg.original_style.size = new_size;
                drop(cfg);
                self.paint()?;
            }
            menu::id::TEXT_SIZE_DOWN => {
                let mut cfg = self.config.borrow_mut();
                let new_size = (cfg.translation_style.size - 1).max(6);
                cfg.translation_style.size = new_size;
                cfg.name_style.size = new_size;
                cfg.original_style.size = new_size;
                drop(cfg);
                self.paint()?;
            }
            // EXIT 처리는 RefCell mutable borrow 가 활성인 상태에서 실행된다.
            // DestroyWindow 를 직접 부르면 같은 스레드에서 WM_DESTROY 가 동기 send
            // 되어 wndproc 가 재진입하는데, 그 시점 borrow_mut 이 실패해
            // dispatch_message 의 WM_DESTROY 분기 (PostQuitMessage 호출처) 를 못 타고
            // DefWindowProcW 로 빠진다 → 프로세스 hang.
            //
            // PostMessageW(WM_CLOSE) 로 메시지 큐에 넣어두면, 현재 wndproc 가
            // 끝나고 borrow 가 풀린 뒤 메시지 루프의 다음 패스에서 WM_CLOSE →
            // 기본 DefWindowProcW 처리 → DestroyWindow → WM_DESTROY → PostQuitMessage
            // 흐름이 정상 작동한다.
            menu::id::EXIT => unsafe {
                if let Err(e) = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) {
                    tracing::error!("PostMessageW(WM_CLOSE) failed: {e}");
                }
            },
            _ => {}
        }
        Ok(())
    }

    /// 대화상자 열기 헬퍼
    ///
    /// 이미 열려있으면 포커스, 아니면 새로 생성
    fn open_dialog_generic<F, E>(hwnd_storage: &mut Option<HWND>, dialog_name: &str, create_fn: F)
    where
        F: FnOnce() -> std::result::Result<HWND, E>,
        E: std::fmt::Display,
    {
        // 이미 열려있으면 포커스
        if let Some(hwnd) = *hwnd_storage {
            // SAFETY: hwnd was previously returned by a successful dialog creation call.
            // IsWindow validates it is still a valid window before use.
            unsafe {
                if IsWindow(Some(hwnd)).as_bool() {
                    let _ = SetForegroundWindow(hwnd);
                    return;
                }
            }
        }

        // 새 대화상자 열기
        match create_fn() {
            Ok(hwnd) => {
                *hwnd_storage = Some(hwnd);
            }
            Err(e) => {
                tracing::error!("Failed to open {} dialog: {}", dialog_name, e);
            }
        }
    }

    /// 설정 대화상자 열기
    fn open_settings_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let config = self.config.clone();
        Self::open_dialog_generic(&mut self.settings_hwnd, "settings", || {
            SettingsDialog::show(main_hwnd, config, None)
        });
    }

    /// 번역 대화상자 열기
    fn open_translate_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let config = self.config.clone();
        Self::open_dialog_generic(&mut self.translate_hwnd, "translate", || {
            TranslateDialog::show(main_hwnd, config)
        });
    }

    /// 백로그 대화상자 열기
    fn open_backlog_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let store = self.backlog_store.clone();
        Self::open_dialog_generic(&mut self.backlog_hwnd, "backlog", || {
            BacklogDialog::show(main_hwnd, store)
        });
    }

    /// 파일 번역 대화상자 열기
    fn open_file_trans_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let config = self.config.clone();
        Self::open_dialog_generic(&mut self.file_trans_hwnd, "file_trans", || {
            FileTransDialog::show(main_hwnd, config)
        });
    }

    /// 후크 설정 대화상자 열기
    fn open_hook_settings_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let config = self.config.clone();
        Self::open_dialog_generic(&mut self.hook_settings_hwnd, "hook_settings", || {
            HookSettingsDialog::show(main_hwnd, config)
        });
    }

    /// 설정 대화상자에서 변경된 윈도우 상태를 실제 윈도우에 반영
    pub(super) fn sync_window_state(&mut self) {
        let cfg = self.config.borrow();
        let click_through = cfg.click_through;
        let topmost = cfg.window_topmost;
        let visible = cfg.window_visible;
        let watch = cfg.clipboard_watch;
        drop(cfg);

        window::set_click_through(self.hwnd, click_through);
        window::set_topmost(self.hwnd, topmost);
        window::set_window_visible(self.hwnd, visible);

        if watch && !self.clipboard.is_watching() {
            self.clipboard.start();
        } else if !watch && self.clipboard.is_watching() {
            self.clipboard.stop();
        }
    }

    /// 자석 모드 토글
    fn toggle_magnetic_mode(&mut self) {
        let mut cfg = self.config.borrow_mut();
        let was_enabled = cfg.magnetic_mode;
        cfg.toggle_magnetic_mode();
        let is_enabled = cfg.magnetic_mode;
        drop(cfg);

        if is_enabled && !was_enabled {
            // 자석 모드 시작
            let mut magnetic = MagneticManager::new(self.hwnd, self.config.clone());
            if let Err(e) = magnetic.start() {
                tracing::error!("Failed to start magnetic mode: {e}");
                self.config.borrow_mut().toggle_magnetic_mode(); // 롤백
                return;
            }
            self.magnetic = Some(magnetic);
        } else if !is_enabled && was_enabled {
            // 자석 모드 중지
            if let Some(ref mut magnetic) = self.magnetic {
                magnetic.stop();
            }
            self.magnetic = None;
        }
    }

    pub(super) fn handle_hotkey(&mut self, id: i32) -> Result<()> {
        if let Some(cmd) = HotkeyManager::to_menu_command(id) {
            self.handle_menu_command(cmd)?;
        }
        Ok(())
    }

    /// 트레이 아이콘 이벤트 처리
    ///
    /// # Safety
    /// `hwnd`는 유효한 윈도우 핸들이어야 한다.
    pub(super) unsafe fn handle_tray_event(&mut self, lparam: LPARAM) {
        unsafe {
            match lparam.0 as u32 {
                WM_LBUTTONUP => {
                    self.config.borrow_mut().window_visible = true;
                    window::set_window_visible(self.hwnd, true);
                }
                WM_RBUTTONUP => {
                    let mut pt: POINT = zeroed();
                    GetCursorPos(&mut pt).ok();
                    if let Err(e) = self.show_context_menu(pt.x, pt.y) {
                        tracing::warn!("tray show_context_menu failed: {e}");
                    }
                }
                _ => {}
            }
        }
    }

    /// 우클릭 컨텍스트 메뉴 처리
    ///
    /// # Safety
    /// `hwnd`는 유효한 윈도우 핸들이어야 한다.
    pub(super) unsafe fn handle_right_click(&mut self, hwnd: HWND, msg: u32, lparam: LPARAM) {
        unsafe {
            let mut pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            if msg == WM_RBUTTONUP {
                let _ = ClientToScreen(hwnd, &mut pt);
            }
            if let Err(e) = self.show_context_menu(pt.x, pt.y) {
                tracing::warn!("show_context_menu failed: {e}");
            }
        }
    }
}
