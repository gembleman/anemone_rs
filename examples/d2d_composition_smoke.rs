//! `d2d_composition` 모듈 동작 검증용 최소 예제
//!
//! `cargo run --example d2d_composition_smoke --target i686-pc-windows-msvc`
//!
//! 빨간 배경 + 파란 사각형 + 반투명 흰 사각형을 합성 경로로 그린 후, 1 초마다
//! 사각형 위치를 흔든다. 윈도우는 `WS_EX_NOREDIRECTIONBITMAP` 으로 만들어
//! `IDCompositionTarget` 이 클라이언트 영역에 visual 을 직접 합성한다.
//!
//! 확인 포인트:
//! - 윈도우가 검정/회색이 아니라 빨간색으로 칠해진다 → swap chain + DComp 연결 OK
//! - 알파 채널이 적용된 영역이 데스크톱 배경과 섞여 보인다 → premultiplied α OK
//! - 리사이즈해도 그림이 깨지지 않는다 → `resize()` 경로 OK
//! - ESC 또는 X 로 종료 시 크래시 없음 → COM 해제 순서 OK
//!
//! 메인 앱의 paint 경로 (`d2d.rs` + layered window) 는 **건드리지 않는다**.
//! 본 예제는 `src/d2d_composition.rs` 만 `#[path]` 로 직접 포함해 binary crate
//! 와 격리된다.

#![cfg(windows)]

// d2d_composition 의 일부 API (flush 등) 는 메인 binary 만 사용하므로
// example 격리 컴파일 시 dead_code 로 잡힌다 — 모듈 단위로 봉합.
#[allow(dead_code)]
#[path = "../src/d2d_composition.rs"]
mod d2d_composition;
// bench 모듈에는 메인 binary 만 사용하는 phase 측정 인프라가 들어있다.
// example 격리 컴파일이라 일부 항목이 dead_code 로 잡혀 모듈 단위로 봉합.
#[allow(dead_code)]
#[path = "../benchmark/bench.rs"]
mod bench;

// d2d.rs 는 `crate::util::to_wide` 와 `crate::window::TextRenderStyle` 를
// 참조하므로, 같은 이름의 stub 모듈을 example crate 루트에 둔다. binary
// crate 와 격리된 채로 d2d.rs 를 그대로 컴파일 시키기 위한 최소 의존만 노출.
mod util {
    pub fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
}
mod window {
    #[derive(Clone, Debug)]
    pub struct TextRenderStyle {
        pub font_size: i32,
        pub font_face: String,
        pub font_style: u8,
        pub color: u32,
        pub outline1_size: i32,
        pub outline1_color: u32,
        pub outline2_size: i32,
        pub outline2_color: u32,
        pub shadow_enabled: bool,
        pub shadow_color: u32,
        pub shadow_offset_x: i32,
        pub shadow_offset_y: i32,
    }
}
// example 은 d2d.rs 의 일부 API (configure_frame / draw_border / draw_text)
// 만 사용한다. 메인 binary crate 에서는 모두 사용되지만 격리 컴파일이라
// 사용 안 한 메서드/필드가 dead_code 경고로 잡힌다 — example 측 모듈에만
// 한정해 봉합.
#[allow(dead_code)]
#[path = "../src/d2d.rs"]
mod d2d;

use bench::{BenchAccumulator, paint_bench_iters};
use d2d::{D2DRenderer, TextBox};
use d2d_composition::CompositionRenderer;
use std::cell::RefCell;
use window::TextRenderStyle;
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
    /// 합성 경로에서 D2DRenderer (DC 경로용으로 만든 일반화된 그리기 헬퍼) 를
    /// 그대로 재사용할 수 있는지 시연용. 인터랙티브 모드에서만 사용.
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

        // 벤치 모드면 baseline (`benchmark/app.rs::run_paint_bench`) 과 동일한
        // 400×200 클라이언트 영역으로 윈도우를 만들어 측정 조건을 맞춘다.
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

        // 단계별 진단 — `CompositionRenderer::new` 가 어느 호출에서 실패하는지
        // 식별. 성공해도 출력은 남겨서 어디까지 도달했는지 알 수 있다.
        diagnose_pipeline(hwnd);

        // D2DRenderer 를 먼저 만들고 그 factory 를 CompositionRenderer 에 주입한다.
        // factory 통일이 핵심 — brush/geometry/text-layout 이 합성 경로
        // device context 위에서 거부되지 않도록 한다 (D2DERR_WRONG_FACTORY 회피).
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
                // 한 borrow 안에서 읽고 쓴다 — `*c.borrow_mut() = c.borrow().wrapping_add(1)`
                // 처럼 양쪽 borrow 를 동시에 잡으면 RefCell 이 panic 한다.
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
                    // 재진입 안전성 — DXGI ResizeBuffers 가 내부에서 메시지를
                    // 펌프해 WM_SIZE/WM_PAINT 가 다시 들어올 수 있다.
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
    /// 벤치 — `app.rs::paint` 와 같은 워크로드 (default TextStyle:
    /// 맑은 고딕 22 / outline1=2 / outline2=4 / shadow_enabled=true,
    /// 초기 텍스트, 400×200 윈도우, margin 10) 를 재현.
    BenchMatchApp,
}

fn render_frame(sync_interval: u32, mode: RenderMode) -> Result<()> {
    RENDERER.with(|r| -> Result<()> {
        // `try_borrow` 로 재진입 (WM_SIZE 처리 중 BeginPaint → WM_PAINT 가
        // 같은 스레드에서 즉시 재호출되는 경로) 시 panic 대신 그리기 스킵.
        let Ok(borrowed) = r.try_borrow() else {
            return Ok(());
        };
        let renderer = borrowed
            .as_ref()
            .ok_or_else(|| Error::from_hresult(E_FAIL))?;

        // waitable swap chain — 메인 paint 와 동일하게 다음 back buffer 가
        // 사용 가능해질 때까지 명시 wait. 안 부르면 EndDraw 내부에서 같은
        // 대기가 발생해 phase 별 비용이 EndDraw 로 합쳐진다.
        renderer.wait_for_back_buffer(1000);

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

    // ── D2DRenderer 결합 시연 ─────────────────────────────────
    //
    // CompositionRenderer 가 돌려준 `&ID2D1DeviceContext` 를 그대로 — cast
    // 없이 deref coercion 만으로 — D2DRenderer 의 일반화된 그리기 메서드에
    // 전달한다. `&ID2D1DeviceContext` → `&ID2D1RenderTarget` 으로 자동 변환
    // 되는 것이 1 단계 일반화의 핵심.
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
        d2d.draw_border(ctx, 640, 360, 4, yellow)?;

        let style = TextRenderStyle {
            font_size: 28,
            font_face: "Segoe UI".to_string(),
            font_style: 1, // bold
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
        d2d.draw_text(
            ctx,
            "D2DRenderer ▸ DComp 결합 OK",
            TextBox {
                x: 20.0,
                y: 300.0,
                max_width: 600.0,
                max_height: 40.0,
            },
            &style,
        )?;
        Ok(())
    })
}

/// 메인 `app.rs::paint` 와 동일한 워크로드 — default TextStyle / 초기
/// 텍스트 / 400×200 윈도우 / margin 10. paint() 가 default config 에서
/// 무엇을 그리는지 정확히 재현한다.
fn draw_bench_match_app(ctx: &ID2D1DeviceContext) -> Result<()> {
    // 메인 paint 의 배경 처리: background_visible=true 일 때 default
    // background_color. config::Config default 와 동일한 값 사용.
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
            font_face: "맑은 고딕".to_string(),
            font_style: 0,
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

/// straight α → premultiplied α 변환.
/// swap chain 이 `DXGI_ALPHA_MODE_PREMULTIPLIED` 라 RGB 에 α 를 곱해 넣지
/// 않으면 흰 빛이 도는 합성 결과가 나온다.
fn premul(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: r * a,
        g: g * a,
        b: b * a,
        a,
    }
}

/// 합성 경로 paint 1 회 비용을 N 회 반복 측정. baseline (기존 paint 경로,
/// `benchmark/app.rs::run_paint_bench`) 과 동일한 구조 — WARMUP 16 + iters 회 측정 →
/// `bench_paint.log` 에 라벨 `composition_paint` 로 append.
///
/// 측정 범위는 begin_draw → D2D 명령 → end_draw_and_present 까지. baseline 의
/// paint() 가 D2D 그리기 + UpdateLayeredWindow 를 포함하는 것과 대응된다.
fn run_composition_bench(iters: usize) {
    const WARMUP: usize = 16;

    eprintln!("--- composition paint bench: warmup {WARMUP} + measure {iters} ---");

    // 벤치는 vsync 비대기 (`SyncInterval=0`) 로 측정. vsync 대기를 포함하면
    // 매 샘플이 정확히 1 모니터 주기 (~16.67ms @ 60Hz) 로 묶여 paint 자체
    // 비용이 가려진다. baseline 의 `UpdateLayeredWindow` 도 vsync 대기를
    // 하지 않으므로 비교 조건을 맞추는 것과도 부합.
    for _ in 0..WARMUP {
        if let Err(e) = render_frame(0, RenderMode::BenchMatchApp) {
            eprintln!("bench warmup failed: {e}");
            return;
        }
    }

    let mut acc = BenchAccumulator::with_capacity(iters);
    for _ in 0..iters {
        let t0 = acc.timer().now();
        if let Err(e) = render_frame(0, RenderMode::BenchMatchApp) {
            eprintln!("bench paint failed: {e}");
            return;
        }
        let t1 = acc.timer().now();
        acc.push(t1 - t0);
    }
    acc.report("composition_paint");
    eprintln!("--- bench done (see bench_paint.log next to exe) ---");
}

/// `CompositionRenderer::new` 의 6 단계를 풀어서 어디서 실패하는지 stderr 에
/// 단계별로 찍는다. 결과 자체는 버린다 — 진단 목적.
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
