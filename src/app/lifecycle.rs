use std::cell::RefCell;
use std::collections::VecDeque;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::rc::Rc;

use windows_core::{Error, HRESULT};
use windows_sys::Win32::{
    Foundation::{GetLastError, HMODULE, HWND, POINT, RECT},
    Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
        MonitorFromPoint, MonitorFromRect, UpdateWindow,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DestroyWindow, DispatchMessageW, GetClientRect,
        GetMessageW, IDC_ARROW, IsWindow, LWA_ALPHA, LoadCursorW, LoadIconW, MSG, PostQuitMessage,
        RegisterClassExW, SWP_NOACTIVATE, SWP_NOZORDER, SetLayeredWindowAttributes, SetWindowPos,
        TranslateMessage, WNDCLASSEXW, WNDPROC, WS_EX_LAYERED, WS_EX_NOREDIRECTIONBITMAP,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    },
};
use windows_sys::core::PCWSTR;
type Result<T> = windows_core::Result<T>;

use super::{
    APP, App, CLASS_NAME, PARENT_CLASS_NAME, WINDOW_TITLE, backlog::BacklogStore,
    services::AppServices, state, window_proc::MIN_WINDOW_SIZE,
};
use crate::clipboard::ClipboardWatcher;
use crate::config::{Config, DEFAULT_WINDOW_HEIGHT, DEFAULT_WINDOW_WIDTH};
use crate::d2d::D2DRenderer;
use crate::dialogs::helpers::dispatch_resource_dialog_message;
use crate::hotkey::HotkeyManager;
use crate::menu::ContextMenu;
use crate::tray::{self, TrayIcon};

const APP_ICON_ID: u32 = 1;

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
                // 이전 비동기 저장이 최종 설정보다 늦게 파일을 덮지 않게 한다.
                app.services.config_save.shutdown();
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
                && IsWindow(hwnd) != 0
            {
                let _ = DestroyWindow(hwnd);
            }
            if IsWindow(self.parent) != 0 {
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
            let instance = GetModuleHandleW(null());
            if instance.is_null() {
                return Err(last_win_error());
            }

            // 윈도우 클래스 등록
            Self::register_class(instance, PARENT_CLASS_NAME, Some(Self::parent_wndproc))?;
            Self::register_class(instance, CLASS_NAME, Some(Self::wndproc))?;

            // 부모 윈도우 생성 (숨김)
            let mut created_windows = Self::create_parent_window(instance)?;

            // 설정 로드 (파일이 없으면 기본값). 저장된 오버레이 위치를 첫
            // CreateWindowExW부터 반영해야 그 모니터의 DPI로 크기를 정할 수 있다.
            let config = Config::load_or_default();
            let (hwnd, initial_client_width, initial_client_height, taskbar_created_msg) =
                Self::create_main_window(instance, created_windows.parent, &config)?;
            created_windows.main = Some(hwnd);

            let app = Self::build_app(
                hwnd,
                taskbar_created_msg,
                config,
                initial_client_width,
                initial_client_height,
            )?;

            // 전역 인스턴스 설정
            APP.with(|cell| {
                *cell.borrow_mut() = Some(app.clone());
            });

            Self::start_runtime(&app, hwnd)?;

            Self::run_message_loop()
        }
    }

    /// 부모(숨김) 윈도우를 만들어 [`CreatedWindows`] guard로 감싼다.
    unsafe fn create_parent_window(instance: HMODULE) -> Result<CreatedWindows> {
        // SAFETY: 유효한 class/instance로 숨김 부모 창을 만든다.
        unsafe {
            let hwnd_parent = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                PARENT_CLASS_NAME,
                WINDOW_TITLE,
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd_parent.is_null() {
                return Err(last_win_error());
            }
            Ok(CreatedWindows {
                parent: hwnd_parent,
                main: None,
            })
        }
    }

    /// 주 오버레이 창을 저장된 위치/크기(또는 DPI 스케일된 기본값)로 만들고
    /// 배치까지 끝낸다. `(hwnd, initial client width, initial client height,
    /// taskbar-created message id)`를 반환한다.
    unsafe fn create_main_window(
        instance: HMODULE,
        hwnd_parent: HWND,
        config: &Config,
    ) -> Result<(HWND, i32, i32, u32)> {
        // SAFETY: 유효한 class/instance/부모 창으로 주 창을 만들고 배치한다.
        unsafe {
            let saved_position = config.window_x.zip(config.window_y);
            let saved_size = config
                .window_width
                .zip(config.window_height)
                .filter(|&(width, height)| width > 0 && height > 0);

            // Redirection surface 없이 DComp premultiplied-alpha visual을 노출한다.
            let (create_width, create_height) =
                saved_size.unwrap_or((DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT));
            let default_monitor = primary_monitor_rect();
            let (create_x, create_y) = saved_position.unwrap_or_else(|| {
                default_monitor
                    .map(|rect| centered_position(rect, create_width, create_height))
                    .unwrap_or((0, 0))
            });
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                WINDOW_TITLE,
                WS_POPUP,
                create_x,
                create_y,
                create_width,
                create_height,
                hwnd_parent,
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                return Err(last_win_error());
            }

            // CreateWindowEx의 좌표는 PMv2 프로세스에서 물리 pixel이다. 96-DPI 기준
            // 초기 overlay 위치와 크기를 대상 모니터 DPI로 확장해 논리 값을 일정하게
            // 둔다. 저장된 위치와 크기는 이미 물리 pixel이므로 그대로 쓴다.
            let initial_dpi = crate::dpi::dpi_for_window(hwnd);
            // SetWindowPos는 WM_GETMINMAXINFO를 거치지 않으므로, 손으로 고친
            // 설정이 최소 크기 아래로 내려가지 않게 여기서 직접 올린다.
            let min_size = crate::dpi::scale(MIN_WINDOW_SIZE, initial_dpi).max(1);
            let (initial_width, initial_height) = saved_size.map_or_else(
                || {
                    (
                        crate::dpi::scale(DEFAULT_WINDOW_WIDTH, initial_dpi).max(1),
                        crate::dpi::scale(DEFAULT_WINDOW_HEIGHT, initial_dpi).max(1),
                    )
                },
                |(width, height)| (width.max(min_size), height.max(min_size)),
            );
            // 저장된 위치가 지금의 모니터 구성에서 완전히 화면 밖이면(모니터를
            // 뗐거나 해상도를 바꾼 경우) 기본 위치로 되돌린다.
            let (initial_x, initial_y) = saved_position
                .filter(|&(x, y)| rect_intersects_monitor(x, y, initial_width, initial_height))
                .unwrap_or_else(|| {
                    default_monitor
                        .map(|rect| centered_position(rect, initial_width, initial_height))
                        .unwrap_or((0, 0))
                });
            if SetWindowPos(
                hwnd,
                null_mut(),
                initial_x,
                initial_y,
                initial_width,
                initial_height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            ) == 0
            {
                return Err(last_win_error());
            }
            let mut client_rect = RECT::default();
            if GetClientRect(hwnd, &mut client_rect) == 0 {
                return Err(last_win_error());
            }
            let initial_client_width = (client_rect.right - client_rect.left).max(1);
            let initial_client_height = (client_rect.bottom - client_rect.top).max(1);

            // Top-level WS_EX_TRANSPARENT의 hit-test 통과는 layered window와 결합해야
            // 다른 process 창까지 보장된다. 불투명도는 DComp의 per-pixel alpha가 담당한다.
            if SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA) == 0 {
                return Err(last_win_error());
            }

            // TaskbarCreated 메시지 등록
            let taskbar_created_msg = tray::register_taskbar_created_message();
            Self::set_taskbar_created_message(taskbar_created_msg);

            Ok((
                hwnd,
                initial_client_width,
                initial_client_height,
                taskbar_created_msg,
            ))
        }
    }

    /// D2D/서비스 초기화 후 `App` 인스턴스를 만들어 `Rc<RefCell<_>>`로 감싼다.
    fn build_app(
        hwnd: HWND,
        taskbar_created_msg: u32,
        config: Config,
        initial_client_width: i32,
        initial_client_height: i32,
    ) -> Result<Rc<RefCell<App>>> {
        // D2D 렌더러 초기화
        let d2d_renderer = D2DRenderer::new()?;
        tracing::info!("Direct2D renderer initialized");

        let services = AppServices::new(hwnd);
        let action_queue = Rc::new(RefCell::new(VecDeque::new()));
        // config는 AppModel로 이동하므로 필요한 값은 미리 꺼낸다.
        let hook_merge_ms = super::config_hook_merge_ms(&config);

        Ok(Rc::new(RefCell::new(App {
            hwnd,
            model: state::AppModel {
                config,
                backlog: BacklogStore::new(),
                runtime: state::AppState {
                    client_size: state::ClientSize::new(
                        initial_client_width,
                        initial_client_height,
                    ),
                    resizing: false,
                    pending_resize: None,
                    original_text: String::new(),
                    translated_text: "아네모네 시작됨 - 클립보드를 복사해보세요".to_string(),
                    overlay_notice: None,
                    pending_translation: None,
                    clipboard_debounce: state::ClipboardDebounce::default(),
                    hook_session: None,
                    hook_merger: crate::hook::text_bridge::TextMerger::new(hook_merge_ms),
                    hook_text_queue: std::collections::VecDeque::new(),
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
            settings_dialog_active: false,
            context_menu_active: false,
            next_clipboard_pause_session: 1,
            taskbar_created_msg,
            magnetic: None,
            d2d_renderer: Some(d2d_renderer),
            composition: None,
            composition_init_failures: 0,
            composition_retry_scheduled: false,
            hook_merge_timer_active: false,
            last_render_diagnostic: None,
            hit_region: Vec::new(),
            full_hit_region: true,
            pending_update: None,
            update_operation_in_progress: false,
        })))
    }

    /// Tray/hotkey/clipboard를 켜고 첫 paint와 자동 업데이트 확인까지 진행한다.
    unsafe fn start_runtime(app: &Rc<RefCell<App>>, hwnd: HWND) -> Result<()> {
        // SAFETY: hwnd와 app은 방금 만든 유효한 창/인스턴스다.
        unsafe {
            // Tray와 hotkey를 준비하고 합성 renderer는 표시 후 첫 paint에서 붙인다.
            let should_start_clipboard = {
                let mut app_ref = app.borrow_mut();

                // 트레이 아이콘 생성
                app_ref.tray.create(hwnd, APP_ICON_ID)?;

                // 핫키 등록 (config에 저장된 사용자 지정 단축키 사용)
                let mut hotkey = HotkeyManager::new(hwnd);
                if let Err(e) = hotkey.register_from_config(&app_ref.model.config.hotkeys) {
                    tracing::warn!("Failed to register hotkeys: {e}");
                }
                app_ref.hotkey = Some(hotkey);

                // 클립보드 감시 여부 확인 (borrow_mut 블록 안에서)
                app_ref.model.config.clipboard_watch
            };
            Self::drain_deferred_messages(app);

            // Listener 등록 중 재진입한 message는 mutable borrow가 끝난 뒤 처리한다.
            if should_start_clipboard {
                let mut app_ref = app.borrow_mut();
                if let Err(error) = app_ref.clipboard.start() {
                    app_ref.model.config.clipboard_watch = false;
                    tracing::error!("Failed to start clipboard listener: {error}");
                }
            }
            Self::drain_deferred_messages(app);

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
            Self::drain_deferred_messages(app);

            // 조건을 통과하면 확인 요청만 보내고 즉시 반환한다 — 시작을 지연시키지 않는다.
            {
                let app_ref = app.borrow();
                app_ref.maybe_start_auto_update_check();
            }

            let _ = UpdateWindow(hwnd);

            // Release smoke test는 GUI 초기화 뒤 자동 종료한다.
            if std::env::var_os("ANEMONE_SMOKE_EXIT").as_deref() == Some(std::ffi::OsStr::new("1"))
            {
                PostQuitMessage(0);
            }

            Ok(())
        }
    }

    /// 표준 Win32 message loop. modeless dialog의 키보드 탐색을 먼저 위임한다.
    unsafe fn run_message_loop() -> Result<()> {
        // SAFETY: 유효한 UI thread에서 표준 message loop를 돈다.
        unsafe {
            let mut msg: MSG = zeroed();
            loop {
                let status = GetMessageW(&mut msg, null_mut(), 0, 0);
                if status == 0 {
                    break;
                }
                if status == -1 {
                    let error = Error::from_hresult(HRESULT::from_win32(GetLastError()));
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
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: wndproc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: LoadIconW(instance, APP_ICON_ID as usize as *const u16),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hbrBackground: null_mut(),
                lpszMenuName: null(),
                lpszClassName: class_name,
                hIconSm: null_mut(),
            };

            if wc.hIcon.is_null() || wc.hCursor.is_null() {
                return Err(last_win_error());
            }

            let atom = RegisterClassExW(&wc);
            if atom == 0 {
                return Err(Error::from_hresult(HRESULT::from_win32(GetLastError())));
            }
            Ok(())
        }
    }
}

/// 주어진 창 사각형이 지금 연결된 모니터 중 하나와 겹치는지 확인한다.
fn rect_intersects_monitor(x: i32, y: i32, width: i32, height: i32) -> bool {
    let rect = RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    };
    // SAFETY: 스택에 있는 유효한 RECT 하나만 넘기며, 반환 handle은 조회만 한다.
    !unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) }.is_null()
}

/// 주 모니터의 전체 영역을 물리 pixel 좌표로 반환한다.
fn primary_monitor_rect() -> Option<RECT> {
    // SAFETY: (0, 0)을 기준으로 주 모니터 handle을 얻고, 유효한 MONITORINFO에 쓴다.
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        (GetMonitorInfoW(monitor, &mut info) != 0).then_some(info.rcMonitor)
    }
}

/// 창의 중심을 모니터 전체 영역의 중심에 맞춘 좌표를 계산한다.
fn centered_position(monitor: RECT, width: i32, height: i32) -> (i32, i32) {
    (
        monitor.left + (monitor.right - monitor.left - width) / 2,
        monitor.top + (monitor.bottom - monitor.top - height) / 2,
    )
}

fn last_win_error() -> Error {
    Error::from_hresult(HRESULT::from_win32(unsafe { GetLastError() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_position_handles_monitor_origin() {
        let monitor = RECT {
            left: -1920,
            top: -100,
            right: 0,
            bottom: 980,
        };
        assert_eq!(centered_position(monitor, 400, 200), (-1160, 340));
    }
}
