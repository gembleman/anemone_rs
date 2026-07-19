use std::cell::RefCell;
use std::collections::VecDeque;
use std::mem::zeroed;
use std::ptr::null_mut;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::{COLORREF, GetLastError, HMODULE, HWND},
        Graphics::Gdi::{HBRUSH, UpdateWindow},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW,
            HICON, IDC_ARROW, IsWindow, LWA_ALPHA, LoadCursorW, LoadIconW, MSG, PostQuitMessage,
            RegisterClassExW, SetLayeredWindowAttributes, TranslateMessage, WNDCLASSEXW, WNDPROC,
            WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
    core::{Error, HRESULT, PCWSTR, Result},
};

use super::{APP, App, CLASS_NAME, PARENT_CLASS_NAME, WINDOW_TITLE, state};
use crate::clipboard::ClipboardWatcher;
use crate::config::Config;
use crate::constants::{
    APP_ICON_ID, INITIAL_WINDOW_HEIGHT, INITIAL_WINDOW_WIDTH, INITIAL_WINDOW_X, INITIAL_WINDOW_Y,
};
use crate::d2d::D2DRenderer;
use crate::dialogs::BacklogStore;
use crate::dialogs::helpers::dispatch_resource_dialog_message;
use crate::hotkey::HotkeyManager;
use crate::menu::ContextMenu;
use crate::services::AppServices;
use crate::tray::{self, TrayIcon};

#[cfg(feature = "benchmark")]
use super::bench;

struct AppCleanupGuard;

impl Drop for AppCleanupGuard {
    fn drop(&mut self) {
        APP.with(|cell| {
            let Ok(mut slot) = cell.try_borrow_mut() else {
                tracing::error!("APP remained borrowed during cleanup");
                return;
            };
            if let Some(app) = slot.take()
                && let Ok(app) = app.try_borrow()
            {
                app.services.translation_ui.unregister(app.hwnd);
                if let Err(error) = app.model.config.save() {
                    tracing::error!("설정 저장 실패: {error}");
                }
                app.services.shutdown();
            }
        });
    }
}

struct CreatedWindows {
    parent: HWND,
    main: Option<HWND>,
}

impl Drop for CreatedWindows {
    fn drop(&mut self) {
        unsafe {
            if let Some(hwnd) = self.main
                && IsWindow(Some(hwnd)).as_bool()
            {
                let _ = DestroyWindow(hwnd);
            }
            if IsWindow(Some(self.parent)).as_bool() {
                let _ = DestroyWindow(self.parent);
            }
        }
    }
}

impl App {
    pub fn run() -> Result<()> {
        let _cleanup = AppCleanupGuard;
        unsafe { Self::run_inner() }
    }

    unsafe fn run_inner() -> Result<()> {
        // SAFETY: 유효한 class/instance로 창을 만들고 주 thread에서 message loop를 돈다.
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
            let mut created_windows = CreatedWindows {
                parent: hwnd_parent,
                main: None,
            };

            // Redirection surface 없이 DComp premultiplied-alpha visual을 노출한다.
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
            // Top-level WS_EX_TRANSPARENT의 hit-test 통과는 layered window와 결합해야
            // 다른 process 창까지 보장된다. 불투명도는 DComp의 per-pixel alpha가 담당한다.
            SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
            created_windows.main = Some(hwnd);

            // TaskbarCreated 메시지 등록
            let taskbar_created_msg = tray::register_taskbar_created_message();
            Self::set_taskbar_created_message(taskbar_created_msg);

            // D2D 렌더러 초기화
            let d2d_renderer = D2DRenderer::new()?;
            tracing::info!("Direct2D renderer initialized");

            // 설정 로드 (파일이 없으면 기본값)
            let config = Config::load_or_default();
            let services = AppServices::new();
            let action_queue = Rc::new(RefCell::new(VecDeque::new()));

            let app = Rc::new(RefCell::new(App {
                hwnd,
                model: state::AppModel {
                    config,
                    backlog: BacklogStore::new(),
                    runtime: state::AppState {
                        client_size: state::ClientSize::new(
                            INITIAL_WINDOW_WIDTH,
                            INITIAL_WINDOW_HEIGHT,
                        ),
                        original_text: String::new(),
                        translated_text: "아네모네 시작됨 - 클립보드를 복사해보세요".to_string(),
                        pending_translation: None,
                        clipboard_debounce: state::ClipboardDebounce::default(),
                    },
                },
                action_queue: action_queue.clone(),
                services,
                tray: TrayIcon::new(),
                menu: ContextMenu::new()?,
                hotkey: None,
                clipboard: ClipboardWatcher::new(hwnd),
                translate_dialog_session: None,
                file_trans_dialog_session: None,
                next_clipboard_pause_session: 1,
                taskbar_created_msg,
                magnetic: None,
                d2d_renderer: Some(d2d_renderer),
                composition: None,
                composition_init_failures: 0,
                composition_retry_scheduled: false,
                hit_region: Vec::new(),
                full_hit_region: true,
            }));

            // 전역 인스턴스 설정
            APP.with(|cell| {
                *cell.borrow_mut() = Some(app.clone());
            });

            // Tray와 hotkey를 준비하고 합성 renderer는 표시 후 첫 paint에서 붙인다.
            let should_start_clipboard = {
                let mut app_ref = app.borrow_mut();

                // 트레이 아이콘 생성
                app_ref.tray.create(hwnd, APP_ICON_ID)?;

                // 핫키 등록
                let mut hotkey = HotkeyManager::new(hwnd);
                if let Err(e) = hotkey.register_defaults() {
                    tracing::warn!("Failed to register hotkeys: {e}");
                }
                app_ref.hotkey = Some(hotkey);

                // 클립보드 감시 여부 확인 (borrow_mut 블록 안에서)
                app_ref.model.config.clipboard_watch
            };
            Self::drain_deferred_messages(&app);

            // Listener 등록 중 재진입한 message는 mutable borrow가 끝난 뒤 처리한다.
            if should_start_clipboard {
                let mut app_ref = app.borrow_mut();
                if let Err(error) = app_ref.clipboard.start() {
                    app_ref.model.config.clipboard_watch = false;
                    tracing::error!("Failed to start clipboard listener: {error}");
                }
            }
            Self::drain_deferred_messages(&app);

            {
                let mut app_ref = app.borrow_mut();
                // 주 창이 foreground가 되기 전에 저장된 runtime 정책을 적용한다.
                app_ref.sync_window_state();
                if let Err(e) = app_ref.paint() {
                    tracing::warn!("initial paint failed: {e}");
                }
                #[cfg(feature = "benchmark")]
                {
                    // 환경 변수로 켠 benchmark는 상세 측정을 우선한다.
                    let ran_bench = if let Some(iters) = bench::paint_bench_detailed_iters() {
                        app_ref.run_paint_bench_detailed(iters);
                        true
                    } else if let Some(iters) = bench::paint_bench_iters() {
                        app_ref.run_paint_bench(iters);
                        true
                    } else {
                        false
                    };

                    // Benchmark 뒤에는 message loop 없이 종료한다.
                    if ran_bench {
                        PostQuitMessage(0);
                    }
                }
            }
            Self::drain_deferred_messages(&app);
            let _ = UpdateWindow(hwnd);

            // Release smoke test는 GUI 초기화 뒤 자동 종료한다.
            if std::env::var_os("ANEMONE_SMOKE_EXIT").as_deref() == Some(std::ffi::OsStr::new("1"))
            {
                PostQuitMessage(0);
            }

            // 메시지 루프
            let mut msg: MSG = zeroed();
            loop {
                let status = GetMessageW(&mut msg, None, 0, 0).0;
                if status == 0 {
                    break;
                }
                if status == -1 {
                    let error = Error::from_hresult(HRESULT::from_win32(GetLastError().0));
                    tracing::error!("GetMessageW failed: {error}");
                    return Err(error);
                }
                // Modeless dialog가 keyboard navigation을 먼저 처리한다.
                if dispatch_resource_dialog_message(&msg) {
                    continue;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            Ok(())
        }
    }

    fn register_class(instance: HMODULE, class_name: PCWSTR, wndproc: WNDPROC) -> Result<()> {
        // SAFETY: module, static class name, wndproc와 WNDCLASSEXW가 유효하다.
        unsafe {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: wndproc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance.into(),
                hIcon: LoadIconW(
                    Some(instance.into()),
                    PCWSTR(APP_ICON_ID as usize as *const u16),
                )?,
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
}
