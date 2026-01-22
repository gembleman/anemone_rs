use std::cell::RefCell;
use std::mem::zeroed;
use std::ptr::null_mut;
use std::rc::Rc;

use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
};

use crate::clipboard::ClipboardWatcher;
use crate::config::Config;
use crate::hotkey::HotkeyManager;
use crate::menu::{self, ContextMenu};
use crate::tray::{self, TrayIcon};
use crate::window::{self, DoubleBuffer};

const CLASS_NAME: PCWSTR = w!("AnemoneWindowClass");
const PARENT_CLASS_NAME: PCWSTR = w!("AnemoneParentClass");
const WINDOW_TITLE: PCWSTR = w!("아네모네");

const MIN_WINDOW_SIZE: i32 = 100;
const RESIZE_BORDER_WIDTH: i32 = 8;

pub struct App {
    hwnd: HWND,
    #[allow(dead_code)]
    hwnd_parent: HWND,
    width: i32,
    height: i32,
    buffer: Option<DoubleBuffer>,
    config: Config,
    tray: TrayIcon,
    menu: ContextMenu,
    hotkey: Option<HotkeyManager>,
    clipboard: ClipboardWatcher,
    taskbar_created_msg: u32,
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

            // App 인스턴스 생성
            let app = Rc::new(RefCell::new(App {
                hwnd,
                hwnd_parent,
                width: 400,
                height: 200,
                buffer: None,
                config: Config::default(),
                tray: TrayIcon::new(),
                menu: ContextMenu::new()?,
                hotkey: None,
                clipboard: ClipboardWatcher::new(hwnd),
                taskbar_created_msg,
            }));

            // 전역 인스턴스 설정
            APP.with(|cell| {
                *cell.borrow_mut() = Some(app.clone());
            });

            // 초기화
            {
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

                // 클립보드 감시 시작
                if app_ref.config.clipboard_watch {
                    app_ref.clipboard.start();
                }

                // 초기 페인트
                app_ref.paint()?;
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

        // 배경 클리어
        if self.config.background_visible {
            let bg = self.config.background_color;
            let a = ((bg >> 24) & 0xFF) as u8;
            let r = ((bg >> 16) & 0xFF) as u8;
            let g = ((bg >> 8) & 0xFF) as u8;
            let b = (bg & 0xFF) as u8;
            buffer.clear(r, g, b, a);
        } else {
            buffer.clear(0, 0, 0, 1); // 거의 투명
        }

        // 테두리 그리기
        if self.config.border_visible {
            buffer.draw_border(self.config.border_width, self.config.border_color);
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
        self.menu.build(&self.config)?;
        self.menu.show(self.hwnd, x, y)
    }

    fn handle_menu_command(&mut self, cmd: u16) -> Result<()> {
        match cmd {
            menu::id::WINDOW_SHOW => {
                self.config.toggle_window_visible();
                window::set_window_visible(self.hwnd, self.config.window_visible);
            }
            menu::id::CLICK_THROUGH => {
                self.config.toggle_click_through();
                window::set_click_through(self.hwnd, self.config.click_through);
            }
            menu::id::CLIPBOARD_WATCH => {
                self.config.toggle_clipboard_watch();
                if self.config.clipboard_watch {
                    self.clipboard.start();
                } else {
                    self.clipboard.stop();
                }
            }
            menu::id::BACKGROUND_TOGGLE => {
                self.config.toggle_background_visible();
                self.paint()?;
            }
            menu::id::BORDER_TOGGLE => {
                self.config.toggle_border_visible();
                self.paint()?;
            }
            menu::id::MAGNETIC_MODE => {
                self.config.toggle_magnetic_mode();
                // TODO: 자석 모드 구현
            }
            menu::id::SETTINGS => {
                // TODO: 설정 대화상자
            }
            menu::id::BACKLOG => {
                // TODO: 백로그 윈도우
            }
            menu::id::EXIT => unsafe {
                DestroyWindow(self.hwnd).ok();
            },
            _ => {}
        }
        Ok(())
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
            // TODO: 번역 처리 등
            println!("Clipboard: {}", text);
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
            // TaskbarCreated 메시지 체크
            let taskbar_msg = app.borrow().taskbar_created_msg;
            if msg == taskbar_msg {
                app.borrow_mut().tray.restore();
                return LRESULT(0);
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
                                let mut app_ref = app.borrow_mut();
                                app_ref.config.window_visible = true;
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

                    // 클립보드 메시지
                    WM_DRAWCLIPBOARD => {
                        app.borrow_mut().handle_clipboard_change();
                        return LRESULT(0);
                    }

                    WM_CHANGECBCHAIN => {
                        app.borrow_mut().clipboard.on_change_chain(wparam, lparam);
                        return LRESULT(0);
                    }

                    _ => {}
                }
            }
        }

        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}
