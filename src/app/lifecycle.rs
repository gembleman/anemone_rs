use std::cell::RefCell;
use std::mem::zeroed;
use std::ptr::null_mut;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::{GetLastError, HMODULE, HWND},
        Graphics::Gdi::{HBRUSH, UpdateWindow},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW,
            HICON, IDC_ARROW, IsWindow, LoadCursorW, LoadIconW, MSG, RegisterClassExW,
            TranslateMessage, WNDCLASSEXW, WNDPROC, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW,
            WS_EX_TOPMOST, WS_POPUP,
        },
    },
    core::{Error, HRESULT, PCWSTR, Result},
};

#[cfg(feature = "benchmark")]
use windows::Win32::UI::WindowsAndMessaging::PostQuitMessage;

use super::{APP, App, CLASS_NAME, DialogWindows, PARENT_CLASS_NAME, WINDOW_TITLE, state};
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
use crate::translation::unregister_translation_hwnd;
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
                unregister_translation_hwnd(app.hwnd);
                if let Err(error) = app.config.borrow().save() {
                    tracing::error!("설정 저장 실패: {error}");
                }
            }
        });
        crate::translation::shutdown();
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
            let mut created_windows = CreatedWindows {
                parent: hwnd_parent,
                main: None,
            };

            // 메인 윈도우 생성 (DComp 합성 경로).
            // - WS_EX_NOREDIRECTIONBITMAP: DWM 이 redirection surface 미할당 → DComp visual 노출
            // - WS_EX_LAYERED 와 상호 배타. layered 시절의 픽셀 단위 알파는 swap chain
            //   premultiplied alpha + DComp 가 동등 표현 제공.
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
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
            created_windows.main = Some(hwnd);

            // TaskbarCreated 메시지 등록
            let taskbar_created_msg = tray::register_taskbar_created_message();
            Self::set_taskbar_created_message(taskbar_created_msg);

            // D2D 렌더러 초기화
            let d2d_renderer = D2DRenderer::new()?;
            tracing::info!("Direct2D renderer initialized");

            // 설정 로드 (파일이 없으면 기본값)
            let config = Rc::new(RefCell::new(Config::load_or_default()));

            // 번역 디스패치는 프로세스 전역 싱글톤. 첫 요청 시 자동 spawn.

            let app = Rc::new(RefCell::new(App {
                hwnd,
                state: state::AppState {
                    client_size: state::ClientSize::new(
                        INITIAL_WINDOW_WIDTH,
                        INITIAL_WINDOW_HEIGHT,
                    ),
                    current_text: "아네모네 시작됨 - 클립보드를 복사해보세요".to_string(),
                    pending_translation: None,
                },
                config,
                tray: TrayIcon::new(),
                menu: ContextMenu::new()?,
                hotkey: None,
                clipboard: ClipboardWatcher::new(hwnd),
                taskbar_created_msg,
                dialogs: DialogWindows::default(),
                backlog_store: Rc::new(RefCell::new(BacklogStore::new())),
                magnetic: None,
                d2d_renderer: Some(d2d_renderer),
                composition: None,
                composition_init_failures: 0,
                composition_retry_scheduled: false,
                pending_clipboard_translation: None,
                hit_region: Vec::new(),
            }));

            // 전역 인스턴스 설정
            APP.with(|cell| {
                *cell.borrow_mut() = Some(app.clone());
            });

            // 초기화 — 트레이/핫키 등 paint 와 무관한 셋업.
            // 합성 경로는 client size > 0 (윈도우가 보인 후) 에서만 부착 가능하므로
            // 첫 paint 는 ShowWindow 이후로 미룬다. 더블버퍼/UpdateLayeredWindow
            // 의존이 사라져 ShowWindow 전에 픽셀을 채울 필요가 없다.
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
                app_ref.config.borrow().clipboard_watch
            };
            Self::drain_deferred_messages(&app);

            // AddClipboardFormatListener can synchronously re-enter wndproc while the watcher
            // is mutably borrowed through App. The common deferred-message queue captures any
            // owned app message and is drained immediately after this borrow ends.
            if should_start_clipboard {
                let mut app_ref = app.borrow_mut();
                app_ref.clipboard.start();
            }
            Self::drain_deferred_messages(&app);

            {
                let mut app_ref = app.borrow_mut();
                // Apply persisted runtime policy before the main window can become the
                // foreground target itself. Visibility is applied by sync_window_state().
                app_ref.sync_window_state();
                if let Err(e) = app_ref.paint() {
                    tracing::warn!("initial paint failed: {e}");
                }
                #[cfg(feature = "benchmark")]
                {
                    // 벤치마크 모드: 환경변수로 켜진 경우 paint() N 회 측정.
                    // detailed 모드가 켜져 있으면 그쪽이 우선 (phase 정보 더 많음).
                    let ran_bench = if let Some(iters) = bench::paint_bench_detailed_iters() {
                        app_ref.run_paint_bench_detailed(iters);
                        true
                    } else if let Some(iters) = bench::paint_bench_iters() {
                        app_ref.run_paint_bench(iters);
                        true
                    } else {
                        false
                    };

                    // 벤치 측정 후에는 메시지 루프에 진입하지 않고 즉시 종료한다.
                    // (벤치는 일회성 측정이므로 GUI 를 띄워둘 이유가 없음 — 외부에서
                    // taskkill 로 죽일 필요 없이 프로세스가 스스로 정리하고 끝난다.)
                    if ran_bench {
                        PostQuitMessage(0);
                    }
                }
            }
            Self::drain_deferred_messages(&app);
            let _ = UpdateWindow(hwnd);

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
                // 리소스 기반 모델리스 창의 Tab/Shift+Tab/기본 버튼 처리를
                // 다이얼로그 매니저에 먼저 맡긴다.
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
