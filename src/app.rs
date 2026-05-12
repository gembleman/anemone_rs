use std::cell::RefCell;
use std::mem::zeroed;
use std::ptr::null_mut;
use std::rc::Rc;
use std::sync::Arc;

use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Dxgi::{DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET},
            Gdi::{ClientToScreen, HBRUSH, UpdateWindow, ValidateRect},
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::clipboard::ClipboardWatcher;
use crate::config::Config;
use crate::constants::{
    INITIAL_WINDOW_HEIGHT, INITIAL_WINDOW_WIDTH, INITIAL_WINDOW_X, INITIAL_WINDOW_Y,
    MIN_WINDOW_SIZE, RESIZE_BORDER_WIDTH,
    WM_DEFERRED_CLIPBOARD, WM_TRANSLATION_COMPLETE, WM_TRAY_ICON,
};
use crate::d2d::D2DRenderer;
use crate::d2d_composition::CompositionRenderer;
use crate::dialogs::helpers::Dialog;
use crate::dialogs::{
    BacklogDialog, FileTransDialog, HookSettingsDialog, LogEntry, SettingsDialog,
    TranslateDialog, add_to_backlog,
};
use crate::hotkey::HotkeyManager;
use crate::magnetic::MagneticManager;
use crate::menu::{self, ContextMenu};
use crate::translation::{request_translation, take_response, unregister_translation_hwnd};
use crate::tray::{self, TrayIcon};
use crate::window::{self, TextRenderStyle};

const CLASS_NAME: PCWSTR = w!("AnemoneWindowClass");
const PARENT_CLASS_NAME: PCWSTR = w!("AnemoneParentClass");
const WINDOW_TITLE: PCWSTR = w!("아네모네");

pub struct App {
    hwnd: HWND,
    width: i32,
    height: i32,
    config: Rc<RefCell<Config>>,
    tray: TrayIcon,
    menu: ContextMenu,
    hotkey: Option<HotkeyManager>,
    clipboard: ClipboardWatcher,
    taskbar_created_msg: u32,
    settings_hwnd: Option<HWND>,
    translate_hwnd: Option<HWND>,
    backlog_hwnd: Option<HWND>,
    file_trans_hwnd: Option<HWND>,
    hook_settings_hwnd: Option<HWND>,
    magnetic: Option<MagneticManager>,
    current_text: String,
    d2d_renderer: Option<D2DRenderer>,
    /// DComp 합성 렌더러. lazy init: hwnd 가 보이는 시점 (`ShowWindow` 후)
    /// 의 첫 paint 에서 만든다. client size 가 0 이면 swap chain 생성이
    /// 실패하기 때문.
    composition: Option<CompositionRenderer>,
    /// 대기 중인 번역의 원문 (번역 완료 시 백로그에 추가)
    pending_original_text: Option<String>,
    /// 텍스트가 차지하는 라인 단위 사각형 (클라이언트 좌표).
    ///
    /// 비어 있으면 `WM_NCHITTEST` 가 윈도우 사각 전체를 `HTCAPTION` 으로
    /// 잡는다 (현재 동작). 비어 있지 않으면 점이 사각형 합집합에 들면
    /// `HTCAPTION`, 아니면 `HTTRANSPARENT`. 채워지는 조건은
    /// `background_visible=false` 이고 `current_text` 가 비어있지 않을 때.
    hit_region: Vec<RECT>,
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
                app_ref.tray.create(hwnd, 0)?;

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
            // SetClipboardViewer가 동기적으로 WM_DRAWCLIPBOARD를 보내므로
            // RefCell이 borrow 상태가 아닐 때 호출해야 함
            if should_start_clipboard {
                app.borrow_mut().clipboard.start();
            }

            // 윈도우 표시 → 첫 paint (CompositionRenderer lazy init 트리거).
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);

            {
                let mut app_ref = app.borrow_mut();
                if let Err(e) = app_ref.paint() {
                    tracing::warn!("initial paint failed: {e}");
                }
                // 벤치마크 모드: 환경변수로 켜진 경우 paint() N 회 측정.
                // detailed 모드가 켜져 있으면 그쪽이 우선 (phase 정보 더 많음).
                if let Some(iters) = crate::bench::paint_bench_detailed_iters() {
                    app_ref.run_paint_bench_detailed(iters);
                } else if let Some(iters) = crate::bench::paint_bench_iters() {
                    app_ref.run_paint_bench(iters);
                }
            }

            // 메시지 루프
            let mut msg: MSG = zeroed();
            while GetMessageW(&mut msg, None, 0, 0).into() {
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
        use crate::bench::{phase_now, phase_record, PhaseField};

        // phase 측정 hook 의 시작 시점. recorder 비활성 시 phase_now() 는 0 반환,
        // phase_record() 는 no-op (None 체크 1 회) — 정상 paint 경로 overhead 거의 0.
        let t = phase_now();
        // 합성 렌더러 lazy init — 첫 paint 시 부착.
        // hwnd 가 보이는 시점 (`ShowWindow` 이후) 이어야 클라이언트 사이즈가 양수다.
        if self.composition.is_none() {
            // D2DRenderer 의 factory 를 공유해 합성 경로의 device 를 같은 factory
            // 위에서 만든다 → brush/geometry/text-layout 의 factory 일치 보장.
            let Some(d2d_renderer) = self.d2d_renderer.as_ref() else {
                tracing::warn!("paint: d2d_renderer 미초기화 — 합성 렌더러 부착 보류");
                return Ok(());
            };
            match CompositionRenderer::new(self.hwnd, d2d_renderer.factory()) {
                Ok(c) => {
                    tracing::info!("DComp composition renderer initialized");
                    self.composition = Some(c);
                }
                Err(e) => {
                    tracing::error!("CompositionRenderer init failed: {e}");
                    return Ok(());
                }
            }
        }
        let t = phase_record(PhaseField::LazyInit, t);

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
            font_face: Arc::from(text_style.font_face.as_str()),
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

        let composition = match self.composition.as_ref() {
            Some(c) => c,
            None => return Ok(()),
        };
        let renderer = match self.d2d_renderer.as_mut() {
            Some(r) => r,
            None => return Ok(()),
        };
        let t = phase_record(PhaseField::Setup, t);

        // waitable swap chain: 다음 back buffer 가 사용 가능해질 때까지 명시
        // 대기. 이 wait 가 없으면 DXGI 가 EndDraw 내부에서 동일 대기를 수행해
        // ~1 ms 스톨을 만든다. 1000 ms 는 GPU TDR 등 비정상 상태 안전망.
        composition.wait_for_back_buffer(1000);
        let t = phase_record(PhaseField::SwapChainWait, t);

        // 합성 경로: BeginDraw 는 CompositionRenderer 가 책임진다.
        let ctx = composition.begin_draw();

        // 한 프레임 시작 — brush 캐시 reset + AA 모드.
        renderer.configure_frame(ctx);

        // 배경 클리어. 배경 비활성 시 ARGB=0 으로 완전 투명. DComp 합성
        // 경로는 hit-testing 이 윈도우 단위라 layered 시절의 "α=1 트릭"
        // (완전 투명이면 클릭이 통과되지 않음 방지) 은 더 이상 필요/유효하지
        // 않다 — α 0 픽셀이든 1 픽셀이든 윈도우 사각 전체가 클릭을 잡는다.
        let clear_color = if background_visible { background_color } else { 0 };
        renderer.clear(ctx, clear_color);
        let t = phase_record(PhaseField::BeginClear, t);

        // 테두리 그리기
        if border_visible
            && let Err(e) = renderer.draw_border(ctx, self.width, self.height, border_width, border_color)
        {
            tracing::error!("D2D draw_border failed: {e}");
        }
        let t = phase_record(PhaseField::Border, t);

        // 텍스트 그리기
        if !self.current_text.is_empty() {
            let max_width = (self.width - margin_x * 2) as f32;
            let max_height = (self.height - margin_y * 2) as f32;
            if let Err(e) = renderer.draw_text(
                ctx,
                &self.current_text,
                crate::d2d::TextBox {
                    x: margin_x as f32,
                    y: margin_y as f32,
                    max_width,
                    max_height,
                },
                &render_style,
            ) {
                tracing::error!("D2D draw_text failed: {e}");
            }
        }
        let t = phase_record(PhaseField::Text, t);

        // Flush + EndDraw + Present 를 분리 호출 — phase 측정용. 정상 동작은
        // `end_draw_and_present` 와 동일하다 (Flush 는 EndDraw 내부에서도
        // 수행되므로 중복이지만 비용 분리 목적).
        // sync_interval=0: 응답성 우선 (paint 는 이벤트 기반이라 매 프레임 호출되지
        // 않으므로 GPU 큐 백프레셔 위험 낮음). baseline 의 UpdateLayeredWindow 도
        // vsync 미대기였으니 동일 정책.
        //
        // device-lost (`D2DERR_RECREATE_TARGET` / `DXGI_ERROR_DEVICE_REMOVED` /
        // `DXGI_ERROR_DEVICE_RESET`) 감지 시 즉시 종료하고 self.handle_device_lost()
        // 로 캐시·합성 렌더러를 폐기. 다음 paint 가 lazy-init 분기에서 다시
        // 만든다.
        if let Err(e) = composition.flush() {
            if Self::is_device_lost(&e) {
                tracing::warn!("DComp flush: device lost ({e}), recreating stack");
                self.handle_device_lost();
                return Ok(());
            }
            tracing::error!("DComp flush failed: {e}");
        }
        let t = phase_record(PhaseField::Flush, t);

        if let Err(e) = composition.end_draw() {
            if Self::is_device_lost(&e) {
                tracing::warn!("DComp end_draw: device lost ({e}), recreating stack");
                self.handle_device_lost();
                return Ok(());
            }
            tracing::error!("DComp end_draw failed: {e}");
        }
        let t = phase_record(PhaseField::EndDraw, t);

        if let Err(e) = composition.present(0) {
            if Self::is_device_lost(&e) {
                tracing::warn!("DComp present: device lost ({e}), recreating stack");
                self.handle_device_lost();
                return Ok(());
            }
            tracing::error!("DComp present failed: {e}");
        }
        let t = phase_record(PhaseField::Present, t);

        // hit_region 갱신 — `&mut self.d2d_renderer` 와 충돌하지 않도록 본
        // 블록의 가변 borrow 가 풀린 뒤 별도 호출. `background_visible=true`
        // 또는 텍스트가 비어 있으면 빈 Vec → WM_NCHITTEST 가 윈도우 사각
        // 전체를 HTCAPTION 으로 잡는 기존 동작 유지.
        self.hit_region.clear();
        if !background_visible && !self.current_text.is_empty() {
            let max_width = (self.width - margin_x * 2) as f32;
            let max_height = (self.height - margin_y * 2) as f32;
            // shadow 가 그림자 방향으로만 확장되므로 양방향 inflate 의 보수적
            // 상한으로 abs 합. outline 은 텍스트 주변 전 방향이라 그대로 합산.
            // i32::MIN 에 가까운 값이 들어오면 unsigned_abs() as i32 가
            // 음수로 뒤집히므로 saturating_abs 로 안전 변환.
            let shadow_inflate = if render_style.shadow_enabled {
                render_style
                    .shadow_offset_x
                    .saturating_abs()
                    .saturating_add(render_style.shadow_offset_y.saturating_abs())
            } else {
                0
            };
            let inflate = (render_style.outline1_size
                + render_style.outline2_size
                + shadow_inflate
                + 1) as f32;
            if let Some(d2d) = self.d2d_renderer.as_mut() {
                match d2d.compute_text_line_rects(
                    &self.current_text,
                    &render_style,
                    crate::d2d::TextBox {
                        x: margin_x as f32,
                        y: margin_y as f32,
                        max_width,
                        max_height,
                    },
                    inflate,
                ) {
                    Ok(rects) => self.hit_region = rects,
                    Err(e) => tracing::warn!("compute_text_line_rects failed: {e}"),
                }
            }
        }
        let _ = phase_record(PhaseField::HitRegion, t);

        Ok(())
    }

    /// flush/end_draw/present 의 에러가 D2D/DXGI 디바이스 손실인지 판정.
    ///
    /// 손실 시 D2D context 와 swap chain 의 모든 GPU 객체가 무효 — 같은
    /// device 위에서 재시도해 봐야 같은 에러가 반복된다. 새 device 와
    /// swap chain 으로 스택을 통째로 다시 만들어야 한다.
    fn is_device_lost(e: &Error) -> bool {
        let code = e.code();
        code == D2DERR_RECREATE_TARGET
            || code == DXGI_ERROR_DEVICE_REMOVED
            || code == DXGI_ERROR_DEVICE_RESET
    }

    /// device-lost 복구: 합성 렌더러를 폐기하고 D2D 캐시(brush/text/outline)
    /// 를 비운다. 다음 paint 의 lazy-init 분기가 새 device 위에서
    /// `CompositionRenderer` 를 다시 만들고, D2DRenderer 는 새 RT 에
    /// 맞춰 캐시를 재구축한다.
    fn handle_device_lost(&mut self) {
        self.composition = None;
        if let Some(d2d) = self.d2d_renderer.as_mut() {
            d2d.invalidate_device_caches();
        }
    }

    /// paint() 1 회 비용을 N 회 반복 측정해 통계를 로그로 출력.
    ///
    /// `ANEMONE_BENCH_PAINT=<N>` 환경변수가 설정된 경우 초기 paint 직후 1 회
    /// 호출된다. D2D 합성 경로 (DCRenderTarget → HwndRT/DComp) 재작성 결정의
    /// baseline 측정용.
    fn run_paint_bench(&mut self, iters: usize) {
        // 워밍업 (캐시 / 셰이더 컴파일 등의 1 회성 비용 제거)
        const WARMUP: usize = 16;
        for _ in 0..WARMUP {
            if let Err(e) = self.paint() {
                tracing::warn!("bench warmup paint failed: {e}");
                return;
            }
        }

        let mut acc = crate::bench::BenchAccumulator::with_capacity(iters);
        for _ in 0..iters {
            let t0 = acc.timer().now();
            if let Err(e) = self.paint() {
                tracing::warn!("bench paint failed: {e}");
                return;
            }
            let t1 = acc.timer().now();
            acc.push(t1 - t0);
        }
        acc.report("paint");
    }

    /// paint() 의 phase 별 비용을 분리 측정. `ANEMONE_BENCH_PAINT_DETAILED=<N>`
    /// 환경변수가 설정된 경우 초기 paint 직후 1 회 호출된다.
    ///
    /// 결과 라벨: `paint_detailed_<phase>` — `setup`, `begin_clear`, `border`,
    /// `text`, `end_draw`, `present`, `hit_region`, `lazy_init`, `total`.
    /// Present 경로 최적화 작업의 ROI 판단 (어느 phase 가 floor 를 만드는가)
    /// 용도. paint() 내부 hook 의 thread_local borrow overhead 가 있어 total
    /// 자체는 일반 `paint` 벤치보다 약간 더 느릴 수 있다.
    fn run_paint_bench_detailed(&mut self, iters: usize) {
        const WARMUP: usize = 16;

        // `ANEMONE_BENCH_PAINT_NO_OUTLINE=1` 토글 — Flush 비용의 출처가
        // outline/shadow geometry 인지 텍스트 본문인지 분리 측정. config 를
        // 임시로 수정하고 측정 후 원복한다 (production paint 경로는 무변경).
        let no_outline = crate::bench::bench_disable_outline();
        let saved_style = if no_outline {
            let mut cfg = self.config.borrow_mut();
            let original = cfg.translation_style.clone();
            cfg.translation_style.outline1_size = 0;
            cfg.translation_style.outline2_size = 0;
            cfg.translation_style.shadow_enabled = false;
            Some(original)
        } else {
            None
        };

        // `ANEMONE_BENCH_PAINT_CACHE_MISS=1` 토글 — 매 iteration 마다
        // current_text 끝에 카운터를 붙여 outline 비트맵 / layout /
        // geometry 캐시를 강제 miss 시킨다. 항목 10 의 "miss 폭주" 위험 실측용.
        let force_cache_miss = crate::bench::bench_force_cache_miss();
        let saved_text = if force_cache_miss {
            Some(self.current_text.clone())
        } else {
            None
        };

        for _ in 0..WARMUP {
            if let Err(e) = self.paint() {
                tracing::warn!("bench detailed warmup paint failed: {e}");
                if let Some(s) = saved_style {
                    self.config.borrow_mut().translation_style = s;
                }
                if let Some(t) = saved_text {
                    self.current_text = t;
                }
                return;
            }
        }

        let mut phased = crate::bench::PhasedBenchAccumulator::with_capacity(iters);
        let outer_timer = crate::bench::QpcTimer::new();
        for i in 0..iters {
            // 매 iteration 마다 텍스트 변경 → 캐시 miss 강제.
            // 카운터는 텍스트 끝 ("…#0", "#1", …) 에 붙여 layout box 크기
            // 변동을 최소화 (자릿수 1 → 2 → 3 자리 전환점에서만 폭 변화).
            if let Some(orig) = saved_text.as_ref() {
                self.current_text = format!("{}#{}", orig, i);
            }
            crate::bench::phase_begin();
            let t0 = outer_timer.now();
            if let Err(e) = self.paint() {
                tracing::warn!("bench detailed paint failed: {e}");
                crate::bench::phase_end(); // 슬롯 비워서 다음 측정 안전
                if let Some(s) = saved_style {
                    self.config.borrow_mut().translation_style = s;
                }
                if let Some(t) = saved_text {
                    self.current_text = t;
                }
                return;
            }
            let t1 = outer_timer.now();
            if let Some(rec) = crate::bench::phase_end() {
                phased.push(&rec, t1 - t0);
            }
        }
        phased.report();

        if let Some(s) = saved_style {
            self.config.borrow_mut().translation_style = s;
        }
        if let Some(t) = saved_text {
            self.current_text = t;
        }
    }

    fn resize(&mut self, width: i32, height: i32) -> Result<()> {
        // 0 사이즈 (minimize) 는 paint/resize 모두 스킵 — DXGI ResizeBuffers 가
        // 0 사이즈를 거부하며, 어차피 그릴 면적도 없다.
        if width <= 0 || height <= 0 {
            return Ok(());
        }

        self.width = width;
        self.height = height;

        // 합성 렌더러가 이미 부착된 상태면 swap chain 도 따라 키운다.
        // 첫 paint 전 (lazy init 직전) 의 WM_SIZE 는 self.composition 이 None 이라
        // 자연 무시된다 — 다음 paint 의 lazy init 이 새 사이즈로 swap chain 을 만든다.
        if let Some(composition) = self.composition.as_mut()
            && let Err(e) = composition.resize(width as u32, height as u32)
        {
            tracing::error!("CompositionRenderer.resize failed: {e}");
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

    /// 파일 번역 대화상자 열기
    fn open_file_trans_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let config = self.config.clone();
        Self::open_dialog_generic(
            &mut self.file_trans_hwnd,
            "file_trans",
            || FileTransDialog::show(main_hwnd, config),
        );
    }

    /// 후크 설정 대화상자 열기
    fn open_hook_settings_dialog(&mut self) {
        let main_hwnd = self.hwnd;
        let config = self.config.clone();
        Self::open_dialog_generic(
            &mut self.hook_settings_hwnd,
            "hook_settings",
            || HookSettingsDialog::show(main_hwnd, config),
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
        if engine == TranslationEngine::EzTrans
            && !config.translation.eztrans_dll_path.is_empty()
        {
            let manager = get_eztrans_manager();
            if let Ok(mut mgr) = manager.lock()
                && let Err(e) = mgr.init(
                    &config.translation.eztrans_dll_path,
                    &config.translation.eztrans_dat_path,
                )
            {
                tracing::warn!("EzTrans init failed: {e}");
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
                    if lparam.0 == 1 {
                        // 설정 대화상자에서 보낸 갱신 요청 — 윈도우 상태도 동기화
                        self.sync_window_state();
                        if let Err(e) = self.paint() {
                            tracing::warn!("paint failed on WM_PAINT: {e}");
                        }
                    }
                    // 시스템이 보낸 WM_PAINT 든 사용자 정의 갱신이든, invalid
                    // region 을 비워야 메시지 큐가 같은 WM_PAINT 를 재발행해
                    // 폭주하는 것을 막는다. NOREDIRECTIONBITMAP 윈도우에선
                    // 시스템 invalidate 빈도가 낮지만 디스플레이 변경 등
                    // 엣지케이스에 대비.
                    let _ = ValidateRect(Some(hwnd), None);
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

