//! `d2d_composition` 시각 검증 예제.
//!
//! `cargo run --example d2d_composition_smoke --target i686-pc-windows-msvc`
//!
//! 움직이는 반투명 도형과 정렬별 text로 합성, alpha, resize, glyph overhang을
//! 확인한다. ESC 또는 닫기 시 COM 해제도 검증한다.

#![cfg(windows)]

// Binary 전용 API가 example에서는 dead code이므로 module 단위로 허용한다.
#[allow(dead_code)]
#[path = "../benchmark/bench.rs"]
mod bench;

mod config {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum TextAlign {
        Left,
        Center,
        Right,
    }
}
// Example이 사용하지 않는 binary용 D2D API만 허용한다.
#[allow(dead_code)]
#[path = "../src/d2d/mod.rs"]
mod d2d;

use bench::{BenchAccumulator, paint_bench_iters};
use config::TextAlign;
use d2d::{CompositionRenderer, D2DRenderer, TextBox, TextRenderStyle, WaitOutcome};
use std::cell::RefCell;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            Direct3D::*,
            Direct3D11::*,
            DirectComposition::*,
            Dxgi::{Common::*, *},
            Gdi::*,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::KeyboardAndMouse::VK_ESCAPE, WindowsAndMessaging::*},
    },
    core::*,
};

const WINDOW_CLASS: PCWSTR = w!("AnemoneDCompSmokeClass");

thread_local! {
    static RENDERER: RefCell<Option<CompositionRenderer>> = const { RefCell::new(None) };
    /// 합성 target에서 `D2DRenderer` 재사용을 시연한다.
    static D2D: RefCell<Option<D2DRenderer>> = const { RefCell::new(None) };
    static FRAME_COUNT: RefCell<u32> = const { RefCell::new(0) };
}

fn main() -> Result<()> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;

        let wnd_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: WINDOW_CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wnd_class);

        // Bench는 앱 baseline과 같은 400×200 client를 사용한다.
        let bench_iters = paint_bench_iters();
        let (win_w, win_h) = if bench_iters.is_some() {
            (400, 200)
        } else {
            (640, 360)
        };

        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP, // DComp 필수: redirection surface 미할당
            WINDOW_CLASS,
            w!("DComp Smoke Test — ESC to quit"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            win_w,
            win_h,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?;

        // 윈도우가 보이는 시점 이후 (클라이언트 사이즈 > 0) 에 합성 스택 부착.
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);

        // 생성 단계를 기록해 실패한 API를 식별한다.
        diagnose_pipeline(hwnd);

        // Factory를 공유해 D2D 자원이 합성 target과 호환되게 한다.
        let d2d = match D2DRenderer::new() {
            Ok(r) => {
                eprintln!("D2DRenderer::new: OK");
                r
            }
            Err(e) => {
                eprintln!("D2DRenderer::new failed: {e}");
                return Err(e);
            }
        };

        let renderer = match CompositionRenderer::new(hwnd, d2d.factory()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("CompositionRenderer::new failed: {e}");
                return Err(e);
            }
        };
        eprintln!("CompositionRenderer::new: OK");
        RENDERER.with(|r| *r.borrow_mut() = Some(renderer));
        D2D.with(|c| *c.borrow_mut() = Some(d2d));

        if let Some(iters) = bench_iters {
            run_composition_bench(iters);
            // 벤치만 돌리고 종료 — UI 루프 진입하지 않음.
            RENDERER.with(|r| r.borrow_mut().take());
            D2D.with(|c| c.borrow_mut().take());
            let _ = DestroyWindow(hwnd);
            return Ok(());
        }

        // 첫 프레임 그리기 트리거
        let _ = InvalidateRect(Some(hwnd), None, false);

        // 0.5 초마다 다시 그려서 흔들리는 사각형 애니메이션 확인
        let _ = SetTimer(Some(hwnd), 1, 500, None);

        // 메시지 루프
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // 명시 해제: 윈도우보다 먼저 COM 객체들이 사라지도록.
        D2D.with(|c| c.borrow_mut().take());
        RENDERER.with(|r| r.borrow_mut().take());
        Ok(())
    }
}

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let _ = BeginPaint(hwnd, &mut ps);
                if let Err(e) = render_frame_vsync() {
                    eprintln!("render_frame failed: {e}");
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_TIMER => {
                // 한 번의 mutable borrow 안에서 값을 갱신한다.
                FRAME_COUNT.with(|c| {
                    let mut v = c.borrow_mut();
                    *v = v.wrapping_add(1);
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_SIZE => {
                let width = (lparam.0 & 0xFFFF) as u32;
                let height = ((lparam.0 >> 16) & 0xFFFF) as u32;
                RENDERER.with(|r| {
                    // ResizeBuffers의 message pump 재진입을 막는다.
                    if let Ok(mut borrowed) = r.try_borrow_mut()
                        && let Some(renderer) = borrowed.as_mut()
                        && let Err(e) = renderer.resize(width, height)
                    {
                        eprintln!("resize failed: {e}");
                    }
                });
                LRESULT(0)
            }
            WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn render_frame_vsync() -> Result<()> {
    render_frame(1, RenderMode::Interactive)
}

#[derive(Copy, Clone)]
enum RenderMode {
    /// 인터랙티브 — 빨간 배경 + 사각형 + 짧은 텍스트로 시각 검증.
    Interactive,
    /// 앱의 기본 `paint()` workload를 재현하는 bench.
    BenchMatchApp,
}

fn render_frame(sync_interval: u32, mode: RenderMode) -> Result<()> {
    RENDERER.with(|r| -> Result<()> {
        // 같은 thread의 paint 재진입 시 panic 대신 frame을 건너뛴다.
        let Ok(borrowed) = r.try_borrow() else {
            return Ok(());
        };
        let renderer = borrowed
            .as_ref()
            .ok_or_else(|| Error::from_hresult(E_FAIL))?;

        // 앱처럼 back buffer 대기를 EndDraw와 분리한다.
        match renderer.wait_for_back_buffer(16) {
            WaitOutcome::Ready => {}
            WaitOutcome::Timeout => return Ok(()),
            WaitOutcome::Failed(e) => return Err(e),
        }

        let ctx = renderer.begin_draw();

        match mode {
            RenderMode::Interactive => draw_interactive(ctx)?,
            RenderMode::BenchMatchApp => draw_bench_match_app(ctx)?,
        }

        renderer.end_draw_and_present(sync_interval)?;
        Ok(())
    })
}

fn draw_interactive(ctx: &ID2D1DeviceContext) -> Result<()> {
    let frame = FRAME_COUNT.with(|c| *c.borrow());
    let offset = ((frame % 8) as f32) * 6.0;

    // pre-multiplied alpha 이므로 RGB 채널에도 α 를 곱해서 넣어야 한다.
    // (R, G, B, A) 각각 [0,1].
    let red_bg = premul(1.0, 0.2, 0.2, 0.85);
    let blue_box = premul(0.2, 0.4, 1.0, 1.0);
    let white_translucent = premul(1.0, 1.0, 1.0, 0.4);

    // SAFETY: ctx 는 begin_draw 가 BeginDraw 를 부른 직후의 컨텍스트.
    unsafe {
        ctx.Clear(Some(&red_bg));

        let blue_brush = ctx.CreateSolidColorBrush(&blue_box, None)?;
        ctx.FillRectangle(
            &D2D_RECT_F {
                left: 40.0 + offset,
                top: 40.0,
                right: 240.0 + offset,
                bottom: 200.0,
            },
            &blue_brush,
        );

        let white_brush = ctx.CreateSolidColorBrush(&white_translucent, None)?;
        ctx.FillRectangle(
            &D2D_RECT_F {
                left: 120.0,
                top: 100.0,
                right: 360.0,
                bottom: 260.0,
            },
            &white_brush,
        );
    }

    // Device context는 deref coercion으로 공통 render target API에 전달된다.
    D2D.with(|c| -> Result<()> {
        let Ok(mut borrowed_d2d) = c.try_borrow_mut() else {
            return Ok(());
        };
        let Some(d2d) = borrowed_d2d.as_mut() else {
            return Ok(());
        };

        // 한 프레임 시작 — 캐시 reset + AA 모드.
        d2d.configure_frame(ctx);

        // 노란 외곽선 (4px 두께)
        let yellow = 0xFFFFD000;
        d2d.draw_border(ctx, 640.0, 360.0, 4, yellow)?;

        let base_style = TextRenderStyle {
            font_size: 24,
            font_face: "Segoe UI".into(),
            font_style: 2, // italic overhang 확인
            text_align: TextAlign::Left,
            color: 0xFFFFFFFF,
            outline1_size: 2,
            outline1_color: 0xFF000000,
            outline2_size: 4,
            outline2_color: 0xFF404040,
            shadow_enabled: true,
            shadow_color: 0xC0000000,
            shadow_offset_x: 2,
            shadow_offset_y: 2,
        };
        // 정렬 3종을 서로 다른 슬롯으로 그려 슬롯 배열 회귀를 감지한다.
        let slots = [
            d2d::MeasureSlot::Name,
            d2d::MeasureSlot::Original,
            d2d::MeasureSlot::Translation,
        ];
        for (line, align) in [TextAlign::Left, TextAlign::Center, TextAlign::Right]
            .into_iter()
            .enumerate()
        {
            let mut style = base_style.clone();
            style.text_align = align;
            d2d.draw_text(
                ctx,
                slots[line],
                "fij ÁW · 한글 · 日本語",
                TextBox {
                    x: 20.0,
                    y: 210.0 + line as f32 * 46.0,
                    max_width: 600.0,
                    max_height: 42.0,
                },
                &style,
            )?;
        }
        Ok(())
    })
}

/// 앱의 기본 style, text, 크기, margin으로 같은 workload를 그린다.
fn draw_bench_match_app(ctx: &ID2D1DeviceContext) -> Result<()> {
    // 앱의 기본 배경색을 사용한다.
    let bg = premul(0.0, 0.0, 0.0, 0.0); // background_visible 가 보통 false 라 ARGB=0
    unsafe {
        ctx.Clear(Some(&bg));
    }

    D2D.with(|c| -> Result<()> {
        let Ok(mut borrowed_d2d) = c.try_borrow_mut() else {
            return Ok(());
        };
        let Some(d2d) = borrowed_d2d.as_mut() else {
            return Ok(());
        };

        d2d.configure_frame(ctx);

        // 메인 paint 와 같은 default TextStyle (config.rs::TextStyle::default).
        let style = TextRenderStyle {
            font_size: 22,
            font_face: "맑은 고딕".into(),
            font_style: 0,
            text_align: TextAlign::Left,
            color: 0xFFFFFFFF,
            outline1_size: 2,
            outline1_color: 0xFF000000,
            outline2_size: 4,
            outline2_color: 0xFF404040,
            shadow_enabled: true,
            shadow_color: 0x80000000,
            shadow_offset_x: 2,
            shadow_offset_y: 2,
        };

        // 400×200 윈도우 / margin 10 — paint() 와 동일.
        const WIDTH: i32 = 400;
        const HEIGHT: i32 = 200;
        const MARGIN: i32 = 10;
        let max_width = (WIDTH - MARGIN * 2) as f32;
        let max_height = (HEIGHT - MARGIN * 2) as f32;

        d2d.draw_text(
            ctx,
            d2d::MeasureSlot::Translation,
            "아네모네 시작됨 - 클립보드를 복사해보세요",
            TextBox {
                x: MARGIN as f32,
                y: MARGIN as f32,
                max_width,
                max_height,
            },
            &style,
        )?;
        Ok(())
    })
}

/// Straight alpha를 swap chain용 premultiplied alpha로 바꾼다.
fn premul(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: r * a,
        g: g * a,
        b: b * a,
        a,
    }
}

/// 합성 경로 전체를 N회 측정해 `composition_paint`로 기록한다.
fn run_composition_bench(iters: usize) {
    const WARMUP: usize = 16;

    eprintln!("--- composition paint bench: warmup {WARMUP} + measure {iters} ---");

    // Paint 비용만 비교하도록 baseline처럼 vsync를 기다리지 않는다.
    for _ in 0..WARMUP {
        if let Err(e) = render_frame(0, RenderMode::BenchMatchApp) {
            eprintln!("bench warmup failed: {e}");
            return;
        }
    }

    let mut acc = BenchAccumulator::with_capacity(iters);
    for _ in 0..iters {
        let started = std::time::Instant::now();
        if let Err(e) = render_frame(0, RenderMode::BenchMatchApp) {
            eprintln!("bench paint failed: {e}");
            return;
        }
        acc.push(started.elapsed());
    }
    acc.report("composition_paint");
    eprintln!("--- bench done (see bench_paint.log next to exe) ---");
}

/// `CompositionRenderer::new`의 각 생성 단계를 stderr에 기록한다.
fn diagnose_pipeline(hwnd: HWND) {
    unsafe {
        eprintln!("--- diagnose_pipeline start ---");

        // 1. D3D11 device
        let mut d3d_device: Option<ID3D11Device> = None;
        match D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d_device),
            None,
            None,
        ) {
            Ok(()) => eprintln!("  1. D3D11CreateDevice: OK"),
            Err(e) => {
                eprintln!("  1. D3D11CreateDevice: FAIL {e}");
                return;
            }
        }
        let Some(d3d_device) = d3d_device else {
            eprintln!("  1b. d3d_device is None");
            return;
        };

        // 2. DXGI device/factory
        let dxgi_device: IDXGIDevice = match d3d_device.cast() {
            Ok(d) => {
                eprintln!("  2a. ID3D11Device -> IDXGIDevice: OK");
                d
            }
            Err(e) => {
                eprintln!("  2a. cast IDXGIDevice: FAIL {e}");
                return;
            }
        };
        let dxgi_adapter: IDXGIAdapter = match dxgi_device.GetAdapter() {
            Ok(a) => {
                eprintln!("  2b. GetAdapter: OK");
                a
            }
            Err(e) => {
                eprintln!("  2b. GetAdapter: FAIL {e}");
                return;
            }
        };
        let dxgi_factory: IDXGIFactory2 = match dxgi_adapter.GetParent() {
            Ok(f) => {
                eprintln!("  2c. GetParent(IDXGIFactory2): OK");
                f
            }
            Err(e) => {
                eprintln!("  2c. GetParent: FAIL {e}");
                return;
            }
        };

        // 3. client rect
        let mut rect = RECT::default();
        if let Err(e) = GetClientRect(hwnd, &mut rect) {
            eprintln!("  3. GetClientRect: FAIL {e}");
            return;
        }
        let width = (rect.right - rect.left).max(1) as u32;
        let height = (rect.bottom - rect.top).max(1) as u32;
        eprintln!("  3. client size = {width}x{height}");

        // 4. swap chain
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        let swap_chain = match dxgi_factory.CreateSwapChainForComposition(&dxgi_device, &desc, None)
        {
            Ok(s) => {
                eprintln!("  4. CreateSwapChainForComposition: OK");
                s
            }
            Err(e) => {
                eprintln!("  4. CreateSwapChainForComposition: FAIL {e}");
                return;
            }
        };

        // 5. D2D factory / device / context
        let d2d_factory: ID2D1Factory1 =
            match D2D1CreateFactory::<ID2D1Factory1>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) {
                Ok(f) => {
                    eprintln!("  5a. D2D1CreateFactory: OK");
                    f
                }
                Err(e) => {
                    eprintln!("  5a. D2D1CreateFactory: FAIL {e}");
                    return;
                }
            };
        let d2d_device = match d2d_factory.CreateDevice(&dxgi_device) {
            Ok(d) => {
                eprintln!("  5b. CreateDevice: OK");
                d
            }
            Err(e) => {
                eprintln!("  5b. CreateDevice: FAIL {e}");
                return;
            }
        };
        let d2d_context = match d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE) {
            Ok(c) => {
                eprintln!("  5c. CreateDeviceContext: OK");
                c
            }
            Err(e) => {
                eprintln!("  5c. CreateDeviceContext: FAIL {e}");
                return;
            }
        };

        // 6. bitmap from swap chain
        let surface: IDXGISurface = match swap_chain.GetBuffer(0) {
            Ok(s) => {
                eprintln!("  6a. swap_chain.GetBuffer(0): OK");
                s
            }
            Err(e) => {
                eprintln!("  6a. GetBuffer: FAIL {e}");
                return;
            }
        };
        let bitmap = match d2d_context.CreateBitmapFromDxgiSurface(&surface, None) {
            Ok(b) => {
                eprintln!("  6b. CreateBitmapFromDxgiSurface: OK");
                b
            }
            Err(e) => {
                eprintln!("  6b. CreateBitmapFromDxgiSurface: FAIL {e}");
                return;
            }
        };
        d2d_context.SetTarget(&bitmap);
        eprintln!("  6c. SetTarget: OK");

        // 7. DComp device / target / visual
        let dcomp_device_v2: IDCompositionDesktopDevice =
            match DCompositionCreateDevice2(&dxgi_device) {
                Ok(d) => {
                    eprintln!("  7a. DCompositionCreateDevice2: OK");
                    d
                }
                Err(e) => {
                    eprintln!("  7a. DCompositionCreateDevice2: FAIL {e}");
                    return;
                }
            };
        let dcomp_device: IDCompositionDevice = match dcomp_device_v2.cast() {
            Ok(d) => {
                eprintln!("  7b. cast to IDCompositionDevice: OK");
                d
            }
            Err(e) => {
                eprintln!("  7b. cast: FAIL {e}");
                return;
            }
        };
        let dcomp_target = match dcomp_device.CreateTargetForHwnd(hwnd, true) {
            Ok(t) => {
                eprintln!("  7c. CreateTargetForHwnd: OK");
                t
            }
            Err(e) => {
                eprintln!("  7c. CreateTargetForHwnd: FAIL {e}");
                return;
            }
        };
        let dcomp_visual = match dcomp_device.CreateVisual() {
            Ok(v) => {
                eprintln!("  7d. CreateVisual: OK");
                v
            }
            Err(e) => {
                eprintln!("  7d. CreateVisual: FAIL {e}");
                return;
            }
        };
        if let Err(e) = dcomp_visual.SetContent(&swap_chain) {
            eprintln!("  7e. SetContent(swap_chain): FAIL {e}");
            return;
        }
        eprintln!("  7e. SetContent: OK");
        if let Err(e) = dcomp_target.SetRoot(&dcomp_visual) {
            eprintln!("  7f. SetRoot: FAIL {e}");
            return;
        }
        eprintln!("  7f. SetRoot: OK");
        if let Err(e) = dcomp_device.Commit() {
            eprintln!("  7g. Commit: FAIL {e}");
            return;
        }
        eprintln!("  7g. Commit: OK");

        // 8. 그리기 한 번 — Clear 까지 가는지
        d2d_context.BeginDraw();
        let red = D2D1_COLOR_F {
            r: 1.0 * 0.85,
            g: 0.2 * 0.85,
            b: 0.2 * 0.85,
            a: 0.85,
        };
        d2d_context.Clear(Some(&red));
        match d2d_context.EndDraw(None, None) {
            Ok(()) => eprintln!("  8. EndDraw after Clear: OK"),
            Err(e) => eprintln!("  8. EndDraw after Clear: FAIL {e}"),
        }
        match swap_chain.Present(1, DXGI_PRESENT::default()).ok() {
            Ok(()) => eprintln!("  9. Present: OK"),
            Err(e) => eprintln!("  9. Present: FAIL {e}"),
        }

        eprintln!("--- diagnose_pipeline end ---");
    }
}
