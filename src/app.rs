use std::cell::RefCell;
use std::mem::zeroed;
use std::ptr::null_mut;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::clipboard::ClipboardWatcher;
use crate::config::Config;
use crate::constants::{
    INITIAL_WINDOW_HEIGHT, INITIAL_WINDOW_WIDTH, INITIAL_WINDOW_X, INITIAL_WINDOW_Y,
    MIN_WINDOW_SIZE, RESIZE_BORDER_WIDTH, TRANSPARENT_ALPHA,
    WM_DEFERRED_CLIPBOARD, WM_TRANSLATION_COMPLETE, WM_TRAY_ICON,
};
use crate::d2d::D2DRenderer;
use crate::dialogs::{BacklogDialog, LogEntry, SettingsDialog, TranslateDialog, add_to_backlog};
use crate::hotkey::HotkeyManager;
use crate::magnetic::MagneticManager;
use crate::menu::{self, ContextMenu};
use crate::translation::{request_translation, take_response};
use crate::tray::{self, TrayIcon};
use crate::window::{self, DoubleBuffer, TextRenderStyle};

const CLASS_NAME: PCWSTR = w!("AnemoneWindowClass");
const PARENT_CLASS_NAME: PCWSTR = w!("AnemoneParentClass");
const WINDOW_TITLE: PCWSTR = w!("아네모네");

pub struct App {
    hwnd: HWND,
    width: i32,
    height: i32,
    buffer: Option<DoubleBuffer>,
    config: Rc<RefCell<Config>>,
    tray: TrayIcon,
    menu: ContextMenu,
    hotkey: Option<HotkeyManager>,
    clipboard: ClipboardWatcher,
    taskbar_created_msg: u32,
    settings_hwnd: Option<HWND>,
    translate_hwnd: Option<HWND>,
    backlog_hwnd: Option<HWND>,
    magnetic: Option<MagneticManager>,
    current_text: String,
    d2d_renderer: Option<D2DRenderer>,
    /// 대기 중인 번역의 원문 (번역 완료 시 백로그에 추가)
    pending_original_text: Option<String>,
}

// 전역 앱 인스턴스 (WndProc에서 접근용)
thread_local! {
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

impl App {
    pub fn run() -> Result<()> {
        // SAFETY: All Win32 API calls use valid parameters; GetModuleHandleW(None) returns the
        // current process handle, CreateWindowExW creates windows with valid class/instance,
        // and the message loop runs on the main thread as required by Win32.
        unsafe {
            let instance = GetModuleHandleW(None)?;

            // 윈도우 클래스 등록
            Self::register_class(instance, PARENT_CLASS_NAME, Some(Self::parent_wndproc))?;
            Self::register_class(instance, CLASS_NAME, Some(Self::wndproc))?;

            // 부모 윈도우 생성 (숨김)
            let hwnd_parent = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                PARENT_CLASS_NAME,
                WINDOW_TITLE,
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )?;

            // 메인 레이어드 윈도우 생성
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                WINDOW_TITLE,
                WS_POPUP,
                INITIAL_WINDOW_X,
                INITIAL_WINDOW_Y,
                INITIAL_WINDOW_WIDTH,
                INITIAL_WINDOW_HEIGHT,
                Some(hwnd_parent),
                None,
                Some(instance.into()),
                None,
            )?;

            // TaskbarCreated 메시지 등록
            let taskbar_created_msg = tray::register_taskbar_created_message();

            // D2D 렌더러 초기화
            let d2d_renderer = D2DRenderer::new()?;
            tracing::info!("Direct2D renderer initialized");

            // 설정 로드 (파일이 없으면 기본값)
            let config = Rc::new(RefCell::new(Config::load_or_default()));

            // 번역 디스패치는 프로세스 전역 싱글톤. 첫 요청 시 자동 spawn.

            let app = Rc::new(RefCell::new(App {
                hwnd,
                width: INITIAL_WINDOW_WIDTH,
                height: INITIAL_WINDOW_HEIGHT,
                buffer: None,
                config,
                tray: TrayIcon::new(),
                menu: ContextMenu::new()?,
                hotkey: None,
                clipboard: ClipboardWatcher::new(hwnd),
                taskbar_created_msg,
                settings_hwnd: None,
                translate_hwnd: None,
                backlog_hwnd: None,
                magnetic: None,
                current_text: "아네모네 시작됨 - 클립보드를 복사해보세요".to_string(),
                d2d_renderer: Some(d2d_renderer),
                pending_original_text: None,
            }));

            // 전역 인스턴스 설정
            APP.with(|cell| {
                *cell.borrow_mut() = Some(app.clone());
            });

            // 초기화
            let should_start_clipboard = {
                let mut app_ref = app.borrow_mut();

                // 더블 버퍼 초기화
                let hdc = GetDC(Some(hwnd));
                app_ref.buffer = Some(DoubleBuffer::new(hdc, INITIAL_WINDOW_WIDTH, INITIAL_WINDOW_HEIGHT)?);
                ReleaseDC(Some(hwnd), hdc);

                // 트레이 아이콘 생성
                app_ref.tray.create(hwnd, 0)?;

                // 핫키 등록
                let mut hotkey = HotkeyManager::new(hwnd);
                if let Err(e) = hotkey.register_defaults() {
                    tracing::warn!("Failed to register hotkeys: {e}");
                }
                app_ref.hotkey = Some(hotkey);

                // 초기 페인트
                app_ref.paint()?;

                // 클립보드 감시 여부 확인 (borrow_mut 블록 안에서)
                app_ref.config.borrow().clipboard_watch
            };

            // 클립보드 감시 시작 (borrow_mut 블록 밖에서)
            // SetClipboardViewer가 동기적으로 WM_DRAWCLIPBOARD를 보내므로
            // RefCell이 borrow 상태가 아닐 때 호출해야 함
            if should_start_clipboard {
                app.borrow_mut().clipboard.start();
            }

            // 윈도우 표시
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);

            // 메시지 루프
            let mut msg: MSG = zeroed();
            while GetMessageW(&mut msg, None, 0, 0).into() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            // 정리
            APP.with(|cell| {
                *cell.borrow_mut() = None;
            });

            Ok(())
        }
    }

    fn register_class(instance: HMODULE, class_name: PCWSTR, wndproc: WNDPROC) -> Result<()> {
        // SAFETY: instance is a valid module handle from GetModuleHandleW, class_name is a
        // static wide string, and wndproc is a valid function pointer. RegisterClassExW is
        // called with a properly initialized WNDCLASSEXW struct.
        unsafe {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: wndproc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance.into(),
                hIcon: LoadIconW(None, IDI_APPLICATION)?,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hbrBackground: HBRUSH(null_mut()),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: class_name,
                hIconSm: HICON::default(),
            };

            let atom = RegisterClassExW(&wc);
            if atom == 0 {
                return Err(Error::from_hresult(HRESULT::from_win32(GetLastError().0)));
            }
            Ok(())
        }
    }

    fn paint(&mut self) -> Result<()> {
        let buffer = match &mut self.buffer {
            Some(b) => b,
            None => return Ok(()),
        };

        let cfg = self.config.borrow();

        // 설정 값 복사
        let background_visible = cfg.background_visible;
        let background_color = cfg.background_color;
        let border_visible = cfg.border_visible;
        let border_width = cfg.border_width;
        let border_color = cfg.border_color;

        // 텍스트 스타일 정보 가져오기
        let text_style = &cfg.translation_style;
        let render_style = TextRenderStyle {
            font_size: text_style.size,
            font_face: text_style.font_face.clone(),
            font_style: text_style.font_style,
            color: text_style.color_primary,
            outline1_size: text_style.outline1_size,
            outline1_color: text_style.color_outline1,
            outline2_size: text_style.outline2_size,
            outline2_color: text_style.color_outline2,
            shadow_enabled: text_style.shadow_enabled,
            shadow_color: text_style.color_shadow,
            shadow_offset_x: cfg.shadow_offset_x,
            shadow_offset_y: cfg.shadow_offset_y,
        };
        let margin_x = cfg.text_margin_x;
        let margin_y = cfg.text_margin_y;

        drop(cfg);

        // D2D 렌더러로 그리기
        if let Some(ref mut renderer) = self.d2d_renderer {
            // DC 바인딩
            if let Err(e) = renderer.bind_dc(buffer.hdc(), buffer.width, buffer.height) {
                tracing::error!("D2D bind_dc failed: {e}");
                return Ok(());
            }

            // 렌더링 시작
            renderer.begin_draw();

            // 배경 클리어
            if background_visible {
                renderer.clear(background_color);
            } else {
                renderer.clear(TRANSPARENT_ALPHA);
            }

            // 테두리 그리기
            if border_visible {
                if let Err(e) = renderer.draw_border(border_width, border_color) {
                    tracing::error!("D2D draw_border failed: {e}");
                }
            }

            // 텍스트 그리기
            if !self.current_text.is_empty() {
                let max_width = (buffer.width - margin_x * 2) as f32;
                let max_height = (buffer.height - margin_y * 2) as f32;
                if let Err(e) = renderer.draw_text(
                    &self.current_text,
                    margin_x as f32,
                    margin_y as f32,
                    max_width,
                    max_height,
                    &render_style,
                ) {
                    tracing::error!("D2D draw_text failed: {e}");
                }
            }

            // 렌더링 종료 (device loss 시 render target 자동 폐기 → 다음 paint에서 재생성)
            if let Err(e) = renderer.end_draw() {
                tracing::error!("D2D end_draw failed: {e}");
            }
        }

        // 레이어드 윈도우 업데이트
        window::update_layered_window(self.hwnd, buffer)?;

        Ok(())
    }

    fn resize(&mut self, width: i32, height: i32) -> Result<()> {
        if width <= 0 || height <= 0 {
            return Ok(());
        }

        self.width = width;
        self.height = height;

        // SAFETY: self.hwnd is a valid window handle created during App initialization.
        // GetDC/ReleaseDC are called in matched pairs with the same hwnd.
        unsafe {
            let hdc = GetDC(Some(self.hwnd));
            self.buffer = Some(DoubleBuffer::new(hdc, width, height)?);
            ReleaseDC(Some(self.hwnd), hdc);
        }

        self.paint()
    }

    fn show_context_menu(&mut self, x: i32, y: i32) -> Result<()> {
        self.menu.build(&self.config.borrow())?;
        self.menu.show(self.hwnd, x, y)
    }

    fn handle_menu_command(&mut self, cmd: u16) -> Result<()> {
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
            // SAFETY: self.hwnd is a valid window handle created during App initialization.
            menu::id::EXIT => unsafe {
                if let Err(e) = DestroyWindow(self.hwnd) {
                    tracing::error!("DestroyWindow failed: {e}");
                }
            },
            _ => {}
        }
        Ok(())
    }

    /// 대화상자 열기 헬퍼
    ///
    /// 이미 열려있으면 포커스, 아니면 새로 생성
    fn open_dialog_generic<F, E>(
        hwnd_storage: &mut Option<HWND>,
        dialog_name: &str,
        create_fn: F,
    ) where
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
        Self::open_dialog_generic(
            &mut self.settings_hwnd,
            "settings",
            || SettingsDialog::show(main_hwnd, config, None),
        );
    }

    /// 번역 대화상자 열기
    fn open_translate_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let config = self.config.clone();
        Self::open_dialog_generic(
            &mut self.translate_hwnd,
            "translate",
            || TranslateDialog::show(main_hwnd, config),
        );
    }

    /// 백로그 대화상자 열기
    fn open_backlog_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let config = self.config.clone();
        Self::open_dialog_generic(
            &mut self.backlog_hwnd,
            "backlog",
            || BacklogDialog::show(main_hwnd, config),
        );
    }

    /// 설정 대화상자에서 변경된 윈도우 상태를 실제 윈도우에 반영
    fn sync_window_state(&mut self) {
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

    fn handle_hotkey(&mut self, id: i32) -> Result<()> {
        if let Some(cmd) = HotkeyManager::to_menu_command(id) {
            self.handle_menu_command(cmd)?;
        }
        Ok(())
    }

    fn handle_clipboard_change(&mut self) {
        if let Some(text) = self.clipboard.on_clipboard_update() {
            // 클립보드 텍스트 처리
            tracing::debug!("Clipboard: {}", text);

            // 자동 번역 처리 (비동기)
            self.process_clipboard_text_async(&text);
        }
    }

    /// 클립보드 텍스트 처리 (언어 감지 및 비동기 번역)
    fn process_clipboard_text_async(&mut self, text: &str) {
        let config = self.config.borrow();

        // 자동 감지가 비활성화되면 원문 표시
        if !config.translation.auto_detect {
            drop(config);
            self.current_text = text.to_string();
            if let Err(e) = self.paint() {
                tracing::warn!("paint failed after clipboard text: {e}");
            }

            // 백로그에 추가
            let entry = LogEntry::new(text.to_string());
            add_to_backlog(entry);
            return;
        }

        let source_lang = config.translation.get_source_language();
        drop(config);

        // 소스 언어가 아니면 번역하지 않음
        if !crate::translation::is_source_language(text, source_lang) {
            self.current_text = text.to_string();
            if let Err(e) = self.paint() {
                tracing::warn!("paint failed after non-source text: {e}");
            }

            // 백로그에 추가
            let entry = LogEntry::new(text.to_string());
            add_to_backlog(entry);
            return;
        }

        // 비동기 번역 요청
        self.request_translation_async(text);
    }

    /// 비동기 번역 요청
    fn request_translation_async(&mut self, text: &str) {
        use crate::translation::{get_eztrans_manager, EngineCredentials, TranslationEngine};

        let config = self.config.borrow();
        let engine = config.translation.get_engine();
        let source_lang = config.translation.get_source_language();
        let target_lang = config.translation.get_target_language();
        let credentials = match engine {
            TranslationEngine::DeepL => EngineCredentials::DeepL {
                keys: config.translation.deepl_effective_keys(),
                strategy: config.translation.deepl_strategy(),
            },
            TranslationEngine::Papago => EngineCredentials::Papago {
                client_id: config.translation.papago_client_id.clone(),
                client_secret: config.translation.papago_client_secret.clone(),
            },
            TranslationEngine::Llm => {
                EngineCredentials::Llm(config.translation.llm.to_call_params())
            }
            _ => EngineCredentials::None,
        };

        // EzTrans 초기화 (필요시)
        if engine == TranslationEngine::EzTrans {
            if !config.translation.eztrans_dll_path.is_empty() {
                let manager = get_eztrans_manager();
                if let Ok(mut mgr) = manager.lock() {
                    if let Err(e) = mgr.init(
                        &config.translation.eztrans_dll_path,
                        &config.translation.eztrans_dat_path,
                    ) {
                        tracing::warn!("EzTrans init failed: {e}");
                    }
                }
            }
        }

        drop(config);

        // 원문 저장 (번역 완료 시 백로그에 추가)
        self.pending_original_text = Some(text.to_string());

        // 번역 중 표시
        self.current_text = format!("[번역 중...]\n{}", text);
        if let Err(e) = self.paint() {
            tracing::warn!("paint failed during translation: {e}");
        }

        // 디스패치에 번역 요청 (워커는 프로세스 전역)
        request_translation(
            self.hwnd,
            text.to_string(),
            engine,
            source_lang,
            target_lang,
            credentials,
        );
    }

    /// 번역 완료 처리
    ///
    /// `WM_TRANSLATION_COMPLETE` 의 WPARAM 으로 전달된 `req_id` 에 해당하는
    /// 응답만 꺼낸다. 디스패치가 hwnd 기준으로 라우팅하므로 다른 다이얼로그의
    /// 응답이 섞일 일은 없지만, 동일 hwnd 에 누적된 응답 중에서도 정확히
    /// 매칭된 한 건만 처리한다.
    fn handle_translation_complete(&mut self, req_id: u64) {
        let Some(response) = take_response(req_id) else {
            return;
        };

        let translation = match response.result {
            Ok(translated) => {
                self.current_text = translated.clone();
                Some(translated)
            }
            Err(err) => {
                tracing::error!("Translation error: {}", err);
                if let Some(ref original) = self.pending_original_text {
                    self.current_text = original.clone();
                }
                None
            }
        };

        if let Some(original) = self.pending_original_text.take() {
            let mut entry = LogEntry::new(original);
            if let Some(trans) = translation {
                entry = entry.with_translation(trans);
            }
            add_to_backlog(entry);
        }

        if let Err(e) = self.paint() {
            tracing::warn!("paint failed after translation complete: {e}");
        }
    }

    /// 트레이 아이콘 이벤트 처리
    ///
    /// # Safety
    /// `hwnd`는 유효한 윈도우 핸들이어야 한다.
    unsafe fn handle_tray_event(&mut self, lparam: LPARAM) {
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
    unsafe fn handle_right_click(&mut self, hwnd: HWND, msg: u32, lparam: LPARAM) {
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
                    // 디스플레이 변경 시 render target 재생성 (DPI/해상도 변경 대응)
                    if let Some(ref mut renderer) = self.d2d_renderer {
                        renderer.invalidate_target();
                    }
                    if let Err(e) = self.paint() {
                        tracing::warn!("paint failed on display change: {e}");
                    }
                    Some(LRESULT(0))
                }

                WM_DPICHANGED => {
                    // Per-Monitor V2: 모니터 간 이동 또는 OS DPI 변경 시 호출된다.
                    // wParam 하위 워드 = 새 DPI. lParam = Windows 가 제안하는 RECT(논리 좌표는 아니고
                    // 새 DPI 에 맞춰 스케일된 화면 좌표). 자식 컨트롤이 없는 D2D 레이어드 윈도우이므로
                    // 권장 RECT 로 위치/크기를 갱신하고 render target 을 무효화하면 충분하다.
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
                    if let Some(ref mut renderer) = self.d2d_renderer {
                        renderer.invalidate_target();
                    }
                    if let Err(e) = self.paint() {
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
                    if lparam.0 == 1 {
                        // 설정 대화상자에서 보낸 갱신 요청 — 윈도우 상태도 동기화
                        self.sync_window_state();
                        if let Err(e) = self.paint() {
                            tracing::warn!("paint failed on WM_PAINT: {e}");
                        }
                    }
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
    unsafe extern "system" fn parent_wndproc(
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
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let app = APP.with(|cell| {
            cell.try_borrow().ok().and_then(|g| g.clone())
        });

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
                        if let Some(hit) = window::hit_test_resize_border(hwnd, x, y, RESIZE_BORDER_WIDTH) {
                            return LRESULT(hit as isize);
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
                    if let Some(result) = unsafe { app_ref.dispatch_message(hwnd, msg, wparam, lparam) } {
                        return result;
                    }
                } else {
                    unsafe {
                        let _ = PostMessageW(
                            Some(hwnd),
                            WM_DEFERRED_CLIPBOARD,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                    return LRESULT(0);
                }
            } else if let Ok(mut app_ref) = app.try_borrow_mut() {
                // SAFETY: Valid system parameters forwarded to dispatch_message.
                if let Some(result) = unsafe { app_ref.dispatch_message(hwnd, msg, wparam, lparam) } {
                    return result;
                }
            }
        }

        // SAFETY: Forwarding valid system-provided parameters to DefWindowProcW.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}
