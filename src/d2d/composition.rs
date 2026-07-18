//! D3D11/DXGI/D2D/DirectComposition 렌더링 자원과 수명을 관리한다.
//!
//! 렌더 경로: `D3D11 → DXGI flip swap chain → D2D device context → DComp → DWM`.
//! 창은 `WS_EX_LAYERED`를 사용해 top-level `WS_EX_TRANSPARENT` hit-test가 다른
//! process 창까지 통과하도록 한다. DirectComposition target은 layered HWND도 지원한다.

use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::*,
            Direct3D::*,
            Direct3D11::*,
            DirectComposition::*,
            Dxgi::{Common::*, *},
        },
        System::Threading::WaitForSingleObjectEx,
        UI::WindowsAndMessaging::GetClientRect,
    },
    core::*,
};

/// Frame-latency 대기 결과. `Timeout`은 back-pressure, `Failed`는 API 오류다.
#[must_use]
pub enum WaitOutcome {
    Ready,
    Timeout,
    Failed(Error),
}

/// 창마다 하나씩 두는 D2D/DirectComposition 렌더러.
pub struct CompositionRenderer {
    // ── DirectX 디바이스 스택 ──────────────────────────
    /// Swap chain보다 오래 살아야 하는 GPU 자원 소유자.
    _d3d_device: ID3D11Device,
    /// D2D 디바이스 컨텍스트. 호출자에게 노출되는 그리기 인터페이스.
    d2d_context: ID2D1DeviceContext,
    /// Frame-latency API를 쓰는 HWND 비결합 합성 swap chain.
    swap_chain: IDXGISwapChain2,
    /// Back buffer를 감싼 D2D 비트맵. `ResizeBuffers` 전에 `None`으로 해제한다.
    bitmap: Option<ID2D1Bitmap1>,
    /// 다음 back buffer를 기다리는 핸들. `Drop`에서 닫는다.
    frame_latency_handle: HANDLE,

    // ── DirectComposition 트리 ────────────────────────
    /// DComp 디바이스. `Commit` 메서드 보유.
    dcomp_device: IDCompositionDevice,
    /// hwnd 와 visual 트리를 묶는 타겟. drop 시 윈도우에서 visual 분리.
    _dcomp_target: IDCompositionTarget,
    /// 루트 visual — 현재는 swap chain 1 개만 매단다.
    _dcomp_visual: IDCompositionVisual,
}

struct HandleGuard(HANDLE);

impl HandleGuard {
    fn into_inner(mut self) -> HANDLE {
        let handle = self.0;
        self.0 = HANDLE::default();
        handle
    }
}

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: Guard owns this handle unless into_inner already disarmed it.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

impl CompositionRenderer {
    /// 창에 합성 스택을 부착한다.
    ///
    /// 호출자는 같은 D2D factory와 호환되는 style, 표시된 0이 아닌 client 영역을
    /// 보장해야 한다.
    pub fn new(hwnd: HWND, d2d_factory: &ID2D1Factory1) -> Result<Self> {
        // SAFETY: 출력 포인터는 로컬이며, 호출자가 유효한 hwnd를 보장한다.
        unsafe {
            // D2D interop용 BGRA 디바이스. 하드웨어 실패 시 WARP로 대체한다.
            let d3d_device = match create_d3d_device(D3D_DRIVER_TYPE_HARDWARE) {
                Ok(device) => device,
                Err(hardware_error) => {
                    tracing::warn!(
                        "D3D11 hardware device creation failed ({hardware_error}); trying WARP"
                    );
                    match create_d3d_device(D3D_DRIVER_TYPE_WARP) {
                        Ok(device) => {
                            tracing::warn!("D3D11 WARP renderer is active");
                            device
                        }
                        Err(warp_error) => {
                            return Err(Error::new(
                                warp_error.code(),
                                format!(
                                    "D3D11 device creation failed: hardware={hardware_error}; \
                                     WARP={warp_error}"
                                ),
                            ));
                        }
                    }
                }
            };

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
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED, // 핵심: per-pixel α
                // ResizeBuffers에도 다시 지정해야 하는 frame-latency 대기 플래그.
                Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
            };

            // 4. composition swap chain 생성 (HWND 와 비결합)
            let swap_chain_v1 = dxgi_factory.CreateSwapChainForComposition(
                &dxgi_device,
                &desc,
                None, // 출력 제한 없음
            )?;
            // V1 → V2 cast (frame latency API 는 V2 에서 도입).
            let swap_chain: IDXGISwapChain2 = swap_chain_v1.cast()?;
            // 이벤트 기반 paint이므로 지연을 줄이도록 한 프레임만 큐잉한다.
            swap_chain.SetMaximumFrameLatency(1)?;
            // wait 핸들 — Drop 에서 CloseHandle 책임.
            let frame_latency_handle = swap_chain.GetFrameLatencyWaitableObject();
            if frame_latency_handle.is_invalid() {
                return Err(Error::from_hresult(E_FAIL));
            }
            let frame_latency_handle = HandleGuard(frame_latency_handle);

            // 호출자의 factory를 공유해 D2DERR_WRONG_FACTORY를 피한다.
            let d2d_device = d2d_factory.CreateDevice(&dxgi_device)?;
            let d2d_context = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;

            // 6. swap chain back buffer 를 D2D bitmap 으로 wrap → render target
            let bitmap = create_bitmap_from_swapchain(&d2d_context, &swap_chain)?;
            d2d_context.SetTarget(&bitmap);

            // 단일 visual 트리에는 V1 IDCompositionDevice면 충분하다.
            let dcomp_device_v2: IDCompositionDesktopDevice =
                DCompositionCreateDevice2(&dxgi_device)?;
            let dcomp_device: IDCompositionDevice = dcomp_device_v2.cast()?;

            let dcomp_target = dcomp_device.CreateTargetForHwnd(
                hwnd, true, // top-most: 같은 윈도우 내 다른 redirection 보다 위
            )?;

            let dcomp_visual = dcomp_device.CreateVisual()?;
            dcomp_visual.SetContent(&swap_chain)?;
            dcomp_target.SetRoot(&dcomp_visual)?;
            dcomp_device.Commit()?;

            Ok(Self {
                _d3d_device: d3d_device,
                d2d_context,
                swap_chain,
                bitmap: Some(bitmap),
                frame_latency_handle: frame_latency_handle.into_inner(),
                dcomp_device,
                _dcomp_target: dcomp_target,
                _dcomp_visual: dcomp_visual,
            })
        }
    }

    /// 다음 back buffer를 기다린다. UI 스레드에서는 짧은 timeout을 사용한다.
    pub fn wait_for_back_buffer(&self, timeout_ms: u32) -> WaitOutcome {
        // SAFETY: 생성자가 얻고 Drop에서 닫는 유효한 대기 핸들이다.
        let result = unsafe { WaitForSingleObjectEx(self.frame_latency_handle, timeout_ms, false) };
        match result {
            WAIT_OBJECT_0 => WaitOutcome::Ready,
            WAIT_TIMEOUT => WaitOutcome::Timeout,
            // WAIT_FAILED의 GetLastError를 다른 Win32 호출 전에 즉시 보존한다.
            WAIT_FAILED => WaitOutcome::Failed(Error::from_thread()),
            _ => WaitOutcome::Failed(Error::new(
                E_UNEXPECTED,
                format!("unexpected frame wait result: 0x{:08X}", result.0),
            )),
        }
    }

    /// 그리기를 시작한다. 반드시 [`Self::end_draw_and_present`]로 마친다.
    pub fn begin_draw(&self) -> &ID2D1DeviceContext {
        // SAFETY: 컨텍스트는 bitmap에 연결됐고 호출자가 EndDraw와 짝을 맞춘다.
        unsafe {
            self.d2d_context.BeginDraw();
        }
        &self.d2d_context
    }

    /// 그리기를 끝내고 swap chain을 제출한다. `sync_interval`은 DXGI Present 값이다.
    /// Device loss는 렌더 스택을 다시 만들 수 있도록 `Err`로 반환한다.
    #[allow(dead_code)]
    pub fn end_draw_and_present(&self, sync_interval: u32) -> Result<()> {
        self.end_draw()?;
        self.present(sync_interval)
    }

    /// `BeginDraw`를 닫고 D2D 명령을 flush한다. 화면 제출은 하지 않는다.
    pub fn end_draw(&self) -> Result<()> {
        // SAFETY: BeginDraw 와 짝. tag 출력은 None — 본 앱은 D2D tag 미사용.
        unsafe { self.d2d_context.EndDraw(None, None) }
    }

    /// EndDraw 이후 swap chain back buffer를 제출한다.
    pub fn present(&self, sync_interval: u32) -> Result<()> {
        // SAFETY: Present 는 GPU 에 비동기 제출 — 호출자는 EndDraw 이후 호출.
        unsafe {
            self.swap_chain
                .Present(sync_interval, DXGI_PRESENT::default())
                .ok()
        }
    }

    /// Back buffer 참조를 모두 푼 뒤 창 크기에 맞춰 swap chain을 재구성한다.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }

        // SAFETY: back buffer 참조를 끊고 resize한 뒤 새 buffer를 다시 연결한다.
        // DXGI가 플래그를 보존하지 않으므로 waitable 플래그도 다시 지정한다.
        unsafe {
            self.d2d_context.SetTarget(None);
            self.bitmap = None;
            self.swap_chain.ResizeBuffers(
                0, // BufferCount 유지
                width,
                height,
                DXGI_FORMAT_UNKNOWN, // 포맷 유지
                DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
            )?;
            let new_bitmap = create_bitmap_from_swapchain(&self.d2d_context, &self.swap_chain)?;
            self.d2d_context.SetTarget(&new_bitmap);
            self.bitmap = Some(new_bitmap);
        }
        Ok(())
    }

    /// Visual 트리가 바뀐 뒤 DComp 변경을 적용한다.
    #[allow(dead_code)]
    pub fn commit(&self) -> Result<()> {
        // SAFETY: dcomp_device 는 생성자에서 만든 유효한 COM 객체.
        unsafe { self.dcomp_device.Commit() }
    }
}

/// 하드웨어와 WARP가 공유하는 D3D11 생성 경계.
unsafe fn create_d3d_device(driver_type: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut device = None;
    // SAFETY: 출력 포인터는 로컬이고 WARP/하드웨어의 software module은 None이다.
    unsafe {
        D3D11CreateDevice(
            None,
            driver_type,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
    }
    device.ok_or_else(|| Error::from_hresult(E_FAIL))
}

impl Drop for CompositionRenderer {
    fn drop(&mut self) {
        // GetFrameLatencyWaitableObject 가 새 핸들을 반환하므로 해제 필요.
        if !self.frame_latency_handle.is_invalid() {
            // SAFETY: 생성자에서 받은 핸들. drop 시 1회만 호출.
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.frame_latency_handle);
            }
        }
    }
}

/// Swap chain의 첫 back buffer를 D2D bitmap으로 감싼다.
unsafe fn create_bitmap_from_swapchain(
    dc: &ID2D1DeviceContext,
    swap_chain: &IDXGISwapChain1,
) -> Result<ID2D1Bitmap1> {
    // SAFETY: swap chain은 렌더 타깃용 buffer 두 개를 가지므로 인덱스 0이 유효하다.
    unsafe {
        let surface: IDXGISurface = swap_chain.GetBuffer(0)?;
        // 속성을 생략해 D2D가 surface의 BGRA8/premultiplied 메타데이터를 사용하게 한다.
        dc.CreateBitmapFromDxgiSurface(&surface, None)
    }
}
