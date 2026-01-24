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
use crate::d2d::D2DRenderer;
use crate::dialogs::{BacklogDialog, LogEntry, SettingsDialog, TranslateDialog, add_to_backlog};
use crate::hotkey::HotkeyManager;
use crate::magnetic::MagneticManager;
use crate::menu::{self, ContextMenu};
use crate::tray::{self, TrayIcon};
use crate::window::{self, DoubleBuffer, TextRenderStyle};

const CLASS_NAME: PCWSTR = w!("AnemoneWindowClass");
const PARENT_CLASS_NAME: PCWSTR = w!("AnemoneParentClass");
const WINDOW_TITLE: PCWSTR = w!("아네모네");

const MIN_WINDOW_SIZE: i32 = 100;
const RESIZE_BORDER_WIDTH: i32 = 8;

// 지연된 클립보드 처리를 위한 사용자 정의 메시지
const WM_DEFERRED_CLIPBOARD: u32 = WM_USER + 200;

pub struct App {
    hwnd: HWND,
    #[allow(dead_code)]
    hwnd_parent: HWND,
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
}

// 전역 앱 인스턴스 (WndProc에서 접근용)
thread_local! {
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

impl App {
    pub fn run() -> Result<()> {
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
                100,
                100,
                400,
                200,
                Some(hwnd_parent),
                None,
                Some(instance.into()),
                None,
            )?;

            // TaskbarCreated 메시지 등록
            let taskbar_created_msg = tray::register_taskbar_created_message();

            // D2D 렌더러 초기화
            let d2d_renderer = D2DRenderer::new()?;
            println!("Direct2D renderer initialized");

            // App 인스턴스 생성
            let config = Rc::new(RefCell::new(Config::default()));
            let app = Rc::new(RefCell::new(App {
                hwnd,
                hwnd_parent,
                width: 400,
                height: 200,
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
                app_ref.buffer = Some(DoubleBuffer::new(hdc, 400, 200)?);
                ReleaseDC(Some(hwnd), hdc);

                // 트레이 아이콘 생성
                app_ref.tray.create(hwnd, 0)?;

                // 핫키 등록
                let mut hotkey = HotkeyManager::new(hwnd);
                if let Err(e) = hotkey.register_defaults() {
                    eprintln!("Failed to register hotkeys: {e}");
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
                eprintln!("D2D bind_dc failed: {e}");
                return Ok(());
            }

            // 렌더링 시작
            renderer.begin_draw();

            // 배경 클리어
            if background_visible {
                renderer.clear(background_color);
            } else {
                renderer.clear(0x01000000); // 거의 투명 (alpha=1)
            }

            // 테두리 그리기
            if border_visible {
                if let Err(e) = renderer.draw_border(border_width, border_color) {
                    eprintln!("D2D draw_border failed: {e}");
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
                    eprintln!("D2D draw_text failed: {e}");
                }
            }

            // 렌더링 종료
            if let Err(e) = renderer.end_draw() {
                eprintln!("D2D end_draw failed: {e}");
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
                self.config.borrow_mut().toggle_window_visible();
                let visible = self.config.borrow().window_visible;
                window::set_window_visible(self.hwnd, visible);
            }
            menu::id::CLICK_THROUGH => {
                self.config.borrow_mut().toggle_click_through();
                let click_through = self.config.borrow().click_through;
                window::set_click_through(self.hwnd, click_through);
            }
            menu::id::CLIPBOARD_WATCH => {
                self.config.borrow_mut().toggle_clipboard_watch();
                let watch = self.config.borrow().clipboard_watch;
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
            menu::id::EXIT => unsafe {
                DestroyWindow(self.hwnd).ok();
            },
            _ => {}
        }
        Ok(())
    }

    /// 설정 대화상자 열기
    fn open_settings_dialog(&mut self) {
        // 이미 열려있으면 포커스
        if let Some(hwnd) = self.settings_hwnd {
            unsafe {
                if IsWindow(Some(hwnd)).as_bool() {
                    let _ = SetForegroundWindow(hwnd);
                    return;
                }
            }
        }

        // 새 설정 대화상자 열기
        match SettingsDialog::show(self.hwnd, self.config.clone(), None) {
            Ok(hwnd) => {
                self.settings_hwnd = Some(hwnd);
            }
            Err(e) => {
                eprintln!("Failed to open settings dialog: {e}");
            }
        }
    }

    /// 번역 대화상자 열기
    fn open_translate_dialog(&mut self) {
        // 이미 열려있으면 포커스
        if let Some(hwnd) = self.translate_hwnd {
            unsafe {
                if IsWindow(Some(hwnd)).as_bool() {
                    let _ = SetForegroundWindow(hwnd);
                    return;
                }
            }
        }

        // 새 번역 대화상자 열기
        match TranslateDialog::show(self.hwnd, self.config.clone()) {
            Ok(hwnd) => {
                self.translate_hwnd = Some(hwnd);
            }
            Err(e) => {
                eprintln!("Failed to open translate dialog: {e}");
            }
        }
    }

    /// 백로그 대화상자 열기
    fn open_backlog_dialog(&mut self) {
        // 이미 열려있으면 포커스
        if let Some(hwnd) = self.backlog_hwnd {
            unsafe {
                if IsWindow(Some(hwnd)).as_bool() {
                    let _ = SetForegroundWindow(hwnd);
                    return;
                }
            }
        }

        // 새 백로그 대화상자 열기
        match BacklogDialog::show(self.hwnd, self.config.clone()) {
            Ok(hwnd) => {
                self.backlog_hwnd = Some(hwnd);
            }
            Err(e) => {
                eprintln!("Failed to open backlog dialog: {e}");
            }
        }
    }

    /// 자석 모드 토글
    fn toggle_magnetic_mode(&mut self) {
        let was_enabled = self.config.borrow().magnetic_mode;
        self.config.borrow_mut().toggle_magnetic_mode();
        let is_enabled = self.config.borrow().magnetic_mode;

        if is_enabled && !was_enabled {
            // 자석 모드 시작
            let mut magnetic = MagneticManager::new(self.hwnd, self.config.clone());
            if let Err(e) = magnetic.start() {
                eprintln!("Failed to start magnetic mode: {e}");
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
        if let Some(text) = self.clipboard.on_draw_clipboard() {
            // 클립보드 텍스트 처리
            println!("Clipboard: {}", text);

            // 자동 번역 처리
            let translated_text = self.process_clipboard_text(&text);

            // 현재 텍스트 업데이트 및 다시 그리기
            self.current_text = translated_text;
            let _ = self.paint();

            // 백로그에 추가 (원문 저장)
            let entry = LogEntry::new(text);
            add_to_backlog(entry);
        }
    }

    /// 클립보드 텍스트 처리 (언어 감지 및 자동 번역)
    fn process_clipboard_text(&self, text: &str) -> String {
        let config = self.config.borrow();

        // 자동 감지가 비활성화되면 원문 반환
        if !config.translation.auto_detect {
            return text.to_string();
        }

        let source_lang = crate::translation::Language::from_u8(config.translation.source_lang);
        drop(config);

        // 소스 언어가 아니면 번역하지 않음
        if !crate::translation::is_source_language(text, source_lang) {
            return text.to_string();
        }

        // 번역 수행
        self.translate_text(text)
    }

    /// 텍스트 번역
    fn translate_text(&self, text: &str) -> String {
        use crate::translation::{
            get_translation_manager, Language, TranslationEngine, TranslationResult,
        };

        let config = self.config.borrow();
        let manager = get_translation_manager();

        if let Ok(mut mgr) = manager.lock() {
            // 설정 동기화
            mgr.set_engine(TranslationEngine::from_u8(config.translation.engine));
            mgr.set_source_language(Language::from_u8(config.translation.source_lang));
            mgr.set_target_language(Language::from_u8(config.translation.target_lang));

            // EzTrans/DeepL 초기화 (필요시)
            if !config.translation.eztrans_dll_path.is_empty() {
                let _ = mgr.init_eztrans(
                    &config.translation.eztrans_dll_path,
                    &config.translation.eztrans_dat_path,
                );
            }
            if !config.translation.deepl_api_key.is_empty() {
                mgr.set_deepl_api_key(config.translation.deepl_api_key.clone());
            }

            drop(config);

            match mgr.translate(text) {
                TranslationResult::Success(translated) => translated,
                TranslationResult::Error(err) => {
                    eprintln!("Translation error: {}", err);
                    text.to_string()
                }
            }
        } else {
            text.to_string()
        }
    }

    unsafe extern "system" fn parent_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // 앱 인스턴스 가져오기
        let app = APP.with(|cell| cell.borrow().clone());

        if let Some(app) = app {
            // TaskbarCreated 메시지 체크 (try_borrow 사용: 재진입 방지)
            if let Ok(app_ref) = app.try_borrow() {
                let taskbar_msg = app_ref.taskbar_created_msg;
                drop(app_ref); // borrow 해제 후 borrow_mut
                if msg == taskbar_msg {
                    if let Ok(mut app_ref) = app.try_borrow_mut() {
                        app_ref.tray.restore();
                    }
                    return LRESULT(0);
                }
            }

            unsafe {
                match msg {
                    WM_DESTROY => {
                        PostQuitMessage(0);
                        return LRESULT(0);
                    }

                    WM_NCHITTEST => {
                        let x = (lparam.0 & 0xFFFF) as i16 as i32;
                        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

                        // 테두리 크기 조절 영역 체크
                        if let Some(hit) =
                            window::hit_test_resize_border(hwnd, x, y, RESIZE_BORDER_WIDTH)
                        {
                            return LRESULT(hit as isize);
                        }

                        // 클라이언트 영역 -> 드래그 가능
                        return LRESULT(HTCAPTION as isize);
                    }

                    WM_GETMINMAXINFO => {
                        let mm = &mut *(lparam.0 as *mut MINMAXINFO);
                        window::set_min_track_size(mm, MIN_WINDOW_SIZE, MIN_WINDOW_SIZE);
                        return LRESULT(0);
                    }

                    WM_SIZE => {
                        let width = (lparam.0 & 0xFFFF) as i32;
                        let height = ((lparam.0 >> 16) & 0xFFFF) as i32;
                        let _ = app.borrow_mut().resize(width, height);
                        return LRESULT(0);
                    }

                    WM_DISPLAYCHANGE => {
                        // 해상도 변경 시 다시 그리기
                        let _ = app.borrow_mut().paint();
                        return LRESULT(0);
                    }

                    WM_RBUTTONUP | WM_NCRBUTTONUP => {
                        let mut pt = POINT {
                            x: (lparam.0 & 0xFFFF) as i16 as i32,
                            y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                        };
                        if msg == WM_RBUTTONUP {
                            let _ = ClientToScreen(hwnd, &mut pt);
                        }
                        let _ = app.borrow_mut().show_context_menu(pt.x, pt.y);
                        return LRESULT(0);
                    }

                    WM_COMMAND => {
                        let cmd = (wparam.0 & 0xFFFF) as u16;
                        let _ = app.borrow_mut().handle_menu_command(cmd);
                        return LRESULT(0);
                    }

                    WM_HOTKEY => {
                        let id = wparam.0 as i32;
                        let _ = app.borrow_mut().handle_hotkey(id);
                        return LRESULT(0);
                    }

                    // 트레이 아이콘 이벤트
                    tray::WM_TRAY_ICON => {
                        match lparam.0 as u32 {
                            WM_LBUTTONUP => {
                                // 좌클릭: 윈도우 표시 토글
                                let app_ref = app.borrow_mut();
                                app_ref.config.borrow_mut().window_visible = true;
                                window::set_window_visible(app_ref.hwnd, true);
                            }
                            WM_RBUTTONUP => {
                                // 우클릭: 컨텍스트 메뉴
                                let mut pt: POINT = zeroed();
                                GetCursorPos(&mut pt).ok();
                                let _ = app.borrow_mut().show_context_menu(pt.x, pt.y);
                            }
                            _ => {}
                        }
                        return LRESULT(0);
                    }

                    // WM_PAINT - 설정 대화상자에서 변경 시 다시 그리기
                    WM_PAINT => {
                        if lparam.0 == 1 {
                            let _ = app.borrow_mut().paint();
                        }
                        return LRESULT(0);
                    }

                    // 클립보드 메시지
                    WM_DRAWCLIPBOARD => {
                        // try_borrow_mut 사용: SetClipboardViewer 호출 중에는 이미 borrow 상태일 수 있음
                        if let Ok(mut app_ref) = app.try_borrow_mut() {
                            app_ref.handle_clipboard_change();
                        } else {
                            // borrow 실패 시 메시지를 지연 처리
                            let _ = PostMessageW(
                                Some(hwnd),
                                WM_DEFERRED_CLIPBOARD,
                                WPARAM(0),
                                LPARAM(0),
                            );
                        }
                        return LRESULT(0);
                    }

                    // 지연된 클립보드 처리
                    msg if msg == WM_DEFERRED_CLIPBOARD => {
                        if let Ok(mut app_ref) = app.try_borrow_mut() {
                            app_ref.handle_clipboard_change();
                        }
                        return LRESULT(0);
                    }

                    WM_CHANGECBCHAIN => {
                        // try_borrow_mut 사용: 재진입 방지
                        if let Ok(mut app_ref) = app.try_borrow_mut() {
                            app_ref.clipboard.on_change_chain(wparam, lparam);
                        }
                        return LRESULT(0);
                    }

                    _ => {}
                }
            }
        }

        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}
