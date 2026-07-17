use super::*;

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
                config,
                tray: TrayIcon::new(),
                menu: ContextMenu::new()?,
                hotkey: None,
                clipboard: ClipboardWatcher::new(hwnd),
                taskbar_created_msg,
                settings_hwnd: None,
                translate_hwnd: None,
                backlog_hwnd: None,
                file_trans_hwnd: None,
                hook_settings_hwnd: None,
                magnetic: None,
                current_text: "아네모네 시작됨 - 클립보드를 복사해보세요".to_string(),
                d2d_renderer: Some(d2d_renderer),
                composition: None,
                pending_original_text: None,
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

            // 클립보드 감시 시작 (borrow_mut 블록 밖에서)
            // AddClipboardFormatListener 가 내부적으로 Win32 동기 메시지
            // (SendMessageTimeoutW)를 보낼 수 있으므로, start() 호출 시점에
            // RefMut 이 살아있으면 wndproc 의 borrow_mut 과 충돌해 패닉한다.
            //
            // `app.borrow_mut().clipboard.start()` 는 임시 RefMut 의 수명이
            // statement 끝까지 유지되어 start() 실행 중에도 borrow 가 걸려 있다.
            // 명시적 변수 + 별도 statement 로 분리해 RefMut 을 start() 전에 drop.
            if should_start_clipboard {
                let mut app_ref = app.borrow_mut();
                app_ref.clipboard.start();
                // app_ref (RefMut) 은 이 블록 끝에서 drop — start() 완료 후.
                // start() 내부의 AddClipboardFormatListener 가 동기 메시지를 보내
                // wndproc 이 재진입하더라도, wndproc 은 try_borrow_mut() 를 쓰므로
                // 이미 borrow 된 상태에서 조용히 skip 한다.
                // (이전에 별도 statement 로 쪼갰던 이유는 착각이었음 — start() 자체가
                // RefMut 홀딩 상태에서 동기 메시지를 유발하는 게 문제이며, wndproc 이
                // try_borrow_mut 을 쓰는 이상 패닉하지 않는다.)
            }

            // 윈도우 표시 → 첫 paint (CompositionRenderer lazy init 트리거).
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);

            {
                let mut app_ref = app.borrow_mut();
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

            // 메시지 루프
            let mut msg: MSG = zeroed();
            while GetMessageW(&mut msg, None, 0, 0).into() {
                // 리소스 기반 모델리스 창의 Tab/Shift+Tab/기본 버튼 처리를
                // 다이얼로그 매니저에 먼저 맡긴다.
                if dispatch_resource_dialog_message(&msg) {
                    continue;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            // 정리 순서:
            // 1) App drop → tray/hotkey/clipboard/composition 등 RAII 해제
            // 2) 번역 워커 스레드 + tokio runtime 명시 종료
            //    (detach 채로 두면 main 리턴 후 CRT cleanup 단계에서 hang 위험)
            APP.with(|cell| {
                *cell.borrow_mut() = None;
            });
            crate::translation::shutdown();

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
