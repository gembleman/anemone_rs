//! Direct2D + DirectComposition 합성 경로 프로토타입 (경로 B)
//!
//! 현재 렌더 경로 (`d2d.rs` + `window.rs::DoubleBuffer`/`update_layered_window`)
//! 를 다음 단계에서 GPU 합성 경로로 교체하기 위한 **스케치**.
//!
//! # 배경
//!
//! 기존 경로:
//! `D2D ID2D1DCRenderTarget` → DIB 메모리 비트맵 → `UpdateLayeredWindow`
//! 의 3 단 구조. paint 1 회당 시스템 메모리 ↔ GPU 왕복이 발생해 baseline
//! avg ~2.66 ms (`docs/next_steps.md` 항목 1 측정 참조).
//!
//! 신규 경로 (본 모듈이 시연):
//! `D3D11 → DXGI flip swap chain (CreateSwapChainForComposition) →
//!  ID2D1DeviceContext + ID2D1Bitmap1 → IDCompositionVisual → DWM`.
//! GPU 안에서 합성이 끝나므로 메모리 왕복이 사라진다.
//!
//! # 윈도우 측 요건 (본 모듈에서는 강제하지 않음)
//!
//! `CreateWindowEx` 시 `WS_EX_NOREDIRECTIONBITMAP` 확장 스타일이 **필수**.
//! 그래야 DWM 이 redirection surface 를 만들지 않아 DComp visual 이 윈도우
//! 클라이언트 영역에 그대로 노출된다. `WS_EX_LAYERED` 와 동시에 사용하지
//! 않는다 (DComp 와 layered window 는 상호 배타).
//!
//! # 알파/히트테스팅
//!
//! pre-multiplied alpha 픽셀 단위 표현은 그대로 유지 (그림자/외곽선 디자인
//! 손실 없음). 단 히트테스팅은 **윈도우 단위로만** 동작하므로 투명 영역
//! 클릭 통과를 픽셀 단위로 의존하던 로직이 있다면 별도 보완 필요
//! (예: `WM_NCHITTEST` 에서 자체 판정).
//!
//! # 본 모듈의 역할
//!
//! 이 파일은 컴파일/링크가 통과하는 최소 동작 스켈레톤이다:
//! - `CompositionRenderer::new(hwnd)` — 전체 스택 부트스트랩
//! - `begin_draw()` → `&ID2D1DeviceContext` 반환 → 호출자가 D2D 명령 발행
//! - `present()` — swap chain 제출 + DComp commit (commit 은 트리 변경 시만)
//! - `resize(w, h)` — `ResizeBuffers` + bitmap 재바인딩
//!
//! 본 작업 (경로 교체) 시:
//! 1. `d2d.rs::D2DRenderer` 의 그리기 메서드들을 `&ID2D1DeviceContext`
//!    대상으로 일반화 (현재는 `ID2D1DCRenderTarget`).
//! 2. `app.rs::paint` 에서 `DoubleBuffer` + `update_layered_window` 경로
//!    제거하고 `CompositionRenderer` 사용.
//! 3. 윈도우 클래스 등록 시 `WS_EX_NOREDIRECTIONBITMAP` 추가, `WS_EX_LAYERED`
//!    제거.
//! 4. `WM_SIZE` 처리에서 `CompositionRenderer::resize` 호출.

use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            Direct3D::*,
            Direct3D11::*,
            DirectComposition::*,
            Dxgi::{Common::*, *},
        },
        UI::WindowsAndMessaging::GetClientRect,
    },
    core::*,
};

/// D2D + DirectComposition 합성 렌더러
///
/// 한 윈도우 (HWND) 당 하나 보유한다. `Drop` 시 COM 객체들이 역순으로 해제.
pub struct CompositionRenderer {
    // ── DirectX 디바이스 스택 ──────────────────────────
    /// D3D11 디바이스 (GPU 자원 소유자). 현재는 보관만 — DXGI device 확보 후
    /// 직접 사용처는 없지만 swap chain 보다 오래 살아야 한다.
    _d3d_device: ID3D11Device,
    /// D2D 디바이스 컨텍스트. 호출자에게 노출되는 그리기 인터페이스.
    d2d_context: ID2D1DeviceContext,
    /// composition 용 swap chain (no-hwnd, flip + premultiplied alpha)
    swap_chain: IDXGISwapChain1,
    /// swap chain back buffer 를 wrap 한 D2D 비트맵. resize 시 재생성.
    bitmap: ID2D1Bitmap1,

    // ── DirectComposition 트리 ────────────────────────
    /// DComp 디바이스. `Commit` 메서드 보유.
    dcomp_device: IDCompositionDevice,
    /// hwnd 와 visual 트리를 묶는 타겟. drop 시 윈도우에서 visual 분리.
    _dcomp_target: IDCompositionTarget,
    /// 루트 visual — 현재는 swap chain 1 개만 매단다.
    _dcomp_visual: IDCompositionVisual,
}

impl CompositionRenderer {
    /// hwnd 에 합성 스택을 부착한다.
    ///
    /// 호출자는 hwnd 가 `WS_EX_NOREDIRECTIONBITMAP` 으로 만들어졌고
    /// `WS_EX_LAYERED` 가 아님을 보장해야 한다. 클라이언트 사이즈가
    /// 0 이면 swap chain 생성이 실패하므로 윈도우가 보이는 시점 이후에
    /// 호출.
    pub fn new(hwnd: HWND) -> Result<Self> {
        // SAFETY: 모든 Direct3D/DXGI/D2D/DComp create 함수는 표준 COM
        // 부트스트랩이며, out-pointer 는 로컬 변수다. hwnd 는 호출자가
        // 보장한 유효 핸들.
        unsafe {
            // 1. D3D11 디바이스 생성 (BGRA = D2D interop)
            let mut d3d_device: Option<ID3D11Device> = None;
            D3D11CreateDevice(
                None,                  // 기본 어댑터
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),    // 소프트웨어 모듈 없음
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,                  // 가능한 가장 높은 feature level 자동 선택
                D3D11_SDK_VERSION,
                Some(&mut d3d_device),
                None,                  // 실제 feature level 알 필요 없음
                None,                  // immediate context 도 안 받음
            )?;
            let d3d_device = d3d_device.ok_or_else(|| Error::from_hresult(E_FAIL))?;

            // 2. DXGI 디바이스/팩토리 확보
            let dxgi_device: IDXGIDevice = d3d_device.cast()?;
            let dxgi_adapter: IDXGIAdapter = dxgi_device.GetAdapter()?;
            let dxgi_factory: IDXGIFactory2 = dxgi_adapter.GetParent()?;

            // 3. composition swap chain 기술 — 클라이언트 사이즈 조회
            let mut rect = RECT::default();
            GetClientRect(hwnd, &mut rect)?;
            let width = (rect.right - rect.left).max(1) as u32;
            let height = (rect.bottom - rect.top).max(1) as u32;

            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED, // 핵심: per-pixel α
                Flags: 0,
            };

            // 4. composition swap chain 생성 (HWND 와 비결합)
            let swap_chain = dxgi_factory.CreateSwapChainForComposition(
                &dxgi_device,
                &desc,
                None, // 출력 제한 없음
            )?;

            // 5. D2D factory + device + context
            let d2d_factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = d2d_factory.CreateDevice(&dxgi_device)?;
            let d2d_context =
                d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;

            // 6. swap chain back buffer 를 D2D bitmap 으로 wrap → render target
            let bitmap = create_bitmap_from_swapchain(&d2d_context, &swap_chain)?;
            d2d_context.SetTarget(&bitmap);

            // 7. DComp 디바이스 / 타겟 / 비주얼
            //
            // V2 API (`DCompositionCreateDevice2`) 가 권장이지만 우리 트리는
            // visual 1 개로 단순하므로 V1 인터페이스 (`IDCompositionDevice`)
            // 만 있으면 충분하다. V2 device 를 만들고 V1 인터페이스로
            // QueryInterface 하는 형태가 표준 (`Why use DirectComposition`
            // MS Learn 참조).
            let dcomp_device_v2: IDCompositionDesktopDevice =
                DCompositionCreateDevice2(&dxgi_device)?;
            let dcomp_device: IDCompositionDevice = dcomp_device_v2.cast()?;

            let dcomp_target = dcomp_device.CreateTargetForHwnd(
                hwnd,
                true, // top-most: 같은 윈도우 내 다른 redirection 보다 위
            )?;

            let dcomp_visual = dcomp_device.CreateVisual()?;
            dcomp_visual.SetContent(&swap_chain)?;
            dcomp_target.SetRoot(&dcomp_visual)?;
            dcomp_device.Commit()?;

            Ok(Self {
                _d3d_device: d3d_device,
                d2d_context,
                swap_chain,
                bitmap,
                dcomp_device,
                _dcomp_target: dcomp_target,
                _dcomp_visual: dcomp_visual,
            })
        }
    }

    /// 그리기 시작. 호출자는 반환된 컨텍스트로 D2D 명령을 발행한다.
    ///
    /// 매 호출 후 반드시 [`Self::end_draw_and_present`] 로 마쳐야 한다.
    pub fn begin_draw(&self) -> &ID2D1DeviceContext {
        // SAFETY: d2d_context 는 생성자에서 SetTarget 으로 bitmap 에 묶여 있다.
        // BeginDraw 는 EndDraw 와 짝이어야 하며 이는 end_draw_and_present 가
        // 보장한다.
        unsafe {
            self.d2d_context.BeginDraw();
        }
        &self.d2d_context
    }

    /// 그리기 종료 + swap chain 제출.
    ///
    /// device-lost (`D2DERR_RECREATE_TARGET`) 시 `Err` 를 반환한다. 호출자는
    /// 스택을 새로 만들어야 한다 (현재 PoC 에서는 재생성 로직 미포함).
    pub fn end_draw_and_present(&self) -> Result<()> {
        // SAFETY: BeginDraw 와 짝. Present 는 매 프레임 호출.
        unsafe {
            self.d2d_context.EndDraw(None, None)?;
            self.swap_chain.Present(1, DXGI_PRESENT::default()).ok()?;
        }
        Ok(())
    }

    /// 윈도우 리사이즈 처리.
    ///
    /// `ResizeBuffers` 호출 전에 D2D 가 잡고 있는 back buffer 참조를 모두
    /// 풀어야 하므로 `SetTarget(None)` + bitmap drop 을 먼저 한다.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }

        // SAFETY: SetTarget(None) 으로 bitmap 의 back buffer 참조를 끊은 뒤
        // ResizeBuffers 호출 → 새 back buffer 를 다시 wrap.
        unsafe {
            self.d2d_context.SetTarget(None);
            // bitmap 자체도 swap chain back buffer 를 잡고 있으므로 교체
            // 가능하도록 일단 dummy 1x1 로 대체하지 않고 ResizeBuffers 에 의존.
            // (Rust 소유권상 bitmap 필드는 즉시 새 값으로 덮어쓴다.)
            self.swap_chain.ResizeBuffers(
                0, // BufferCount 유지
                width,
                height,
                DXGI_FORMAT_UNKNOWN, // 포맷 유지
                DXGI_SWAP_CHAIN_FLAG::default(),
            )?;
            self.bitmap = create_bitmap_from_swapchain(&self.d2d_context, &self.swap_chain)?;
            self.d2d_context.SetTarget(&self.bitmap);
        }
        Ok(())
    }

    /// DComp 트리 변경 시 호출. visual 트리가 정적이면 생성자에서 1 회면
    /// 충분 — 본 메서드는 추후 visual 추가/제거 작업용 훅.
    #[allow(dead_code)]
    pub fn commit(&self) -> Result<()> {
        // SAFETY: dcomp_device 는 생성자에서 만든 유효한 COM 객체.
        unsafe { self.dcomp_device.Commit() }
    }
}

/// swap chain back buffer 0 번을 D2D bitmap 으로 wrap.
///
/// pre-multiplied alpha + B8G8R8A8 — swap chain 디스크립션과 일치해야 한다.
unsafe fn create_bitmap_from_swapchain(
    dc: &ID2D1DeviceContext,
    swap_chain: &IDXGISwapChain1,
) -> Result<ID2D1Bitmap1> {
    // SAFETY: swap_chain 은 BufferCount=2, BufferUsage 에 RENDER_TARGET_OUTPUT
    // 이 포함된 유효한 객체. 인덱스 0 은 항상 존재.
    unsafe {
        let surface: IDXGISurface = swap_chain.GetBuffer(0)?;
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))
    }
}
