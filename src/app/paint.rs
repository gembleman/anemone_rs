//! `WM_PAINT`가 실제로 그리는 한 프레임의 실행부.
//!
//! [`super::rendering`]이 config/텍스트로부터 [`RenderBlock`] 목록을 만드는
//! 순수 계산을 담당한다면, 이 모듈은 그 블록을 composition/D2D 자원에 올려
//! 실제로 화면에 그리는 Win32/D2D 부작용을 담당한다.

use windows_sys::Win32::{Graphics::Gdi::InvalidateRect, UI::WindowsAndMessaging::SetTimer};

use windows_core::Result;

use super::rendering::{
    RenderBlock, build_notice_render_blocks, build_render_blocks, composition_retry_delay_ms,
    dip_rect_to_pixels, pixels_to_dips,
};
use super::{App, COMPOSITION_RETRY_TIMER, state};
use crate::d2d::MeasureSlot;
use crate::d2d::{CompositionRenderer, WaitOutcome};

/// [`App::paint`] 한 프레임을 그리는 데 필요한, borrow 없이 들고 다닐 수 있는 계획.
///
/// composition/renderer lazy-init과 config 스냅샷 + 블록 측정을 한 번에 끝내
/// 이후 draw 단계가 `self`를 다시 빌려도 충돌하지 않게 한다.
struct FramePlan {
    render_blocks: Vec<RenderBlock>,
    margin_x: i32,
    dpi: u32,
    client_width: f32,
    client_height: f32,
    max_width: f32,
    background_visible: bool,
    background_color: u32,
    border_visible: bool,
    border_width: i32,
    border_color: u32,
}

impl App {
    const FRAME_WAIT_TIMEOUT_MS: u32 = 16;

    fn text_layout_extent(size: f32, margin: i32) -> f32 {
        (size - margin.saturating_mul(2) as f32).max(1.0)
    }

    /// 클라이언트 크기가 생기는 첫 paint에서 합성 렌더러를 붙인다.
    ///
    /// `Ok(true)`면 이번 paint를 계속 진행해도 된다는 뜻이고, `Ok(false)`면
    /// lazy-init이 아직 끝나지 않았거나 재시도를 예약했으므로 이번 프레임은
    /// 건너뛰어야 한다.
    fn ensure_composition_ready(&mut self) -> Result<bool> {
        if self.composition.is_some() {
            return Ok(true);
        }
        if self.composition_retry_scheduled {
            return Ok(false);
        }
        // D2D factory를 공유해 모든 그리기 자원의 호환성을 보장한다.
        let Some(d2d_renderer) = self.d2d_renderer.as_ref() else {
            tracing::warn!("paint: d2d_renderer 미초기화 — 합성 렌더러 부착 보류");
            return Ok(false);
        };
        match CompositionRenderer::new(
            windows::Win32::Foundation::HWND(self.hwnd),
            d2d_renderer.factory(),
        ) {
            Ok(c) => {
                tracing::info!("DComp composition renderer initialized");
                self.composition = Some(c);
                self.composition_init_failures = 0;
                Ok(true)
            }
            Err(e) => {
                let delay_ms = composition_retry_delay_ms(self.composition_init_failures);
                self.composition_init_failures = self.composition_init_failures.saturating_add(1);
                // SAFETY: hwnd는 App이 소유한다. callback 없는 window timer는
                // WM_TIMER를 같은 UI thread의 window procedure로 보낸다.
                let timer = unsafe { SetTimer(self.hwnd, COMPOSITION_RETRY_TIMER, delay_ms, None) };
                self.composition_retry_scheduled = timer != 0;
                tracing::error!("CompositionRenderer init failed: {e}; retry in {delay_ms} ms");
                Ok(false)
            }
        }
    }

    /// config 스냅샷 + 표시할 블록 구성 + DPI 종속 측정까지 끝낸 [`FramePlan`]을 만든다.
    ///
    /// composition/renderer가 아직 없으면 `None` — 이번 프레임은 건너뛴다.
    fn build_frame_plan(&mut self) -> Option<FramePlan> {
        let cfg = &self.model.config;

        // 설정 값 복사
        let background_visible = cfg.background_visible;
        let background_color = cfg.background_color;
        let border_visible = cfg.border_visible;
        let border_width = cfg.border_width;
        let border_color = cfg.border_color;
        let margin_x = cfg.text_margin_x;

        let mut render_blocks = match self.model.runtime.overlay_notice {
            Some(notice) => build_notice_render_blocks(cfg, notice.text()),
            None => build_render_blocks(
                cfg,
                &self.model.runtime.original_text,
                &self.model.runtime.translated_text,
            ),
        };

        let composition = self.composition.as_ref()?;
        let dpi = crate::dpi::dpi_for_window(self.hwnd).max(1);
        let dpi_changed = composition.set_dpi(dpi as f32);
        let renderer = self.d2d_renderer.as_mut()?;
        if dpi_changed {
            // Compatible render target bitmap은 생성 당시 DPI를 따르므로 다시 만든다.
            renderer.invalidate_device_caches();
        }
        let client_width = pixels_to_dips(self.model.runtime.client_size.width, dpi);
        let client_height = pixels_to_dips(self.model.runtime.client_size.height, dpi);
        let max_width = Self::text_layout_extent(client_width, margin_x);
        if let Some(first) = render_blocks.first() {
            let mut top = first.top;
            for block in &mut render_blocks {
                block.top = top;
                match renderer.measure_text_height(block.slot, &block.text, &block.style, max_width)
                {
                    Ok(height) => block.height = height,
                    Err(error) => {
                        tracing::warn!("DirectWrite text measurement failed: {error}");
                    }
                }
                top += block.height + block.gap_after;
            }
        }

        if cfg.hook.debug_log && self.model.runtime.hook_session.is_some() {
            let blocks = render_blocks
                .iter()
                .map(|block| {
                    format!(
                        "slot={:?} top={:.1} height={:.1} text={:?}",
                        block.slot, block.top, block.height, block.text
                    )
                })
                .collect::<Vec<_>>()
                .join(" | ");
            let diagnostic = format!(
                "client_px={}x{} client_dip={:.1}x{:.1} dpi={} max_width={:.1} blocks=[{}]",
                self.model.runtime.client_size.width,
                self.model.runtime.client_size.height,
                client_width,
                client_height,
                dpi,
                max_width,
                blocks
            );
            if self.last_render_diagnostic.as_deref() != Some(&diagnostic) {
                tracing::info!(
                    target: crate::logging::LUNAHOOK_TARGET,
                    "[overlay-layout]{diagnostic}"
                );
                self.last_render_diagnostic = Some(diagnostic);
            }
        }

        Some(FramePlan {
            render_blocks,
            margin_x,
            dpi,
            client_width,
            client_height,
            max_width,
            background_visible,
            background_color,
            border_visible,
            border_width,
            border_color,
        })
    }

    /// hit-test에 쓰는 client 좌표 사각형(`hit_region`)과 `full_hit_region`을 갱신한다.
    fn update_hit_region(&mut self, plan: &FramePlan) {
        // 렌더러 borrow가 끝난 뒤 hit region을 갱신한다. 배경/테두리가 없고
        // text 사각형도 없으면 완전히 보이지 않는 frame이므로 입력도 받지 않는다.
        self.hit_region.clear();
        self.full_hit_region = plan.background_visible || plan.border_visible;
        if !self.full_hit_region
            && !plan.render_blocks.is_empty()
            && let Some(d2d) = self.d2d_renderer.as_mut()
        {
            for block in &plan.render_blocks {
                let shadow_inflate = if block.style.shadow_enabled {
                    block
                        .style
                        .shadow_offset_x
                        .saturating_abs()
                        .saturating_add(block.style.shadow_offset_y.saturating_abs())
                } else {
                    0
                };
                let inflate = block
                    .style
                    .outline1_size
                    .saturating_add(block.style.outline2_size)
                    .saturating_add(shadow_inflate)
                    .saturating_add(1) as f32;
                match d2d.compute_text_line_rects(
                    block.slot,
                    &block.text,
                    &block.style,
                    crate::d2d::TextBox {
                        x: plan.margin_x as f32,
                        y: block.top,
                        max_width: plan.max_width,
                        max_height: plan.client_height,
                    },
                    inflate,
                ) {
                    Ok(rects) => self
                        .hit_region
                        .extend(rects.iter().map(|rect| dip_rect_to_pixels(rect, plan.dpi))),
                    Err(e) => tracing::warn!("compute_text_line_rects failed: {e}"),
                }
            }
        }
    }

    /// 이번 프레임에서 사용하지 않은 슬롯의 bitmap/layout/hit-test 캐시를 정리한다.
    ///
    /// 표시가 꺼진 유형의 장치 종속 자원이 상주하지 않게 한다. notice 표시
    /// 중에는 본문 슬롯이 notice 종료 직후에 필요하므로 전 슬롯을 사용 중으로
    /// 보고해 폐기를 막는다 (캐시는 어차피 4슬롯 바운드 — 회수 손실이 없다.
    /// notice가 끝난 뒤 paint가 이어지면 유예 카운터가 올라간다).
    fn prune_render_slots(&mut self, render_blocks: &[RenderBlock]) {
        let mut used_slots = [true; MeasureSlot::COUNT];
        if self.model.runtime.overlay_notice.is_none() {
            used_slots = [false; MeasureSlot::COUNT];
            for block in render_blocks {
                used_slots[block.slot as usize] = true;
            }
        }
        if let Some(renderer) = self.d2d_renderer.as_mut() {
            renderer.prune_unused_slots(used_slots);
        }
    }

    pub(super) fn paint(&mut self) -> Result<()> {
        #[cfg(feature = "benchmark")]
        use super::bench::{PhaseField, phase_now, phase_record};

        // Recorder가 꺼져 있으면 phase hook은 사실상 no-op이다.
        #[cfg(feature = "benchmark")]
        let t = phase_now();

        if !self.ensure_composition_ready()? {
            return Ok(());
        }
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::LazyInit, t);

        let Some(plan) = self.build_frame_plan() else {
            return Ok(());
        };
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::Setup, t);

        let Some(composition) = self.composition.as_ref() else {
            return Ok(());
        };
        let Some(renderer) = self.d2d_renderer.as_mut() else {
            return Ok(());
        };

        // 한 프레임만 기다린다. Timeout은 재예약하고 API 오류는 스택을 재생성한다.
        match composition.wait_for_back_buffer(Self::FRAME_WAIT_TIMEOUT_MS) {
            WaitOutcome::Ready => {}
            WaitOutcome::Timeout => {
                tracing::debug!("DComp back buffer wait timed out; retrying next paint");
                // SAFETY: hwnd는 App이 소유한 유효한 top-level window handle.
                unsafe {
                    let _ = InvalidateRect(self.hwnd, std::ptr::null(), 0);
                }
                return Ok(());
            }
            WaitOutcome::Failed(e) => {
                tracing::error!("DComp back buffer wait failed: {e}; recreating stack");
                self.recover_render_stack();
                return Ok(());
            }
        }
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::SwapChainWait, t);

        // 합성 경로: BeginDraw 는 CompositionRenderer 가 책임진다.
        let ctx = composition.begin_draw();

        // 한 프레임 시작 — brush 캐시 reset + AA 모드.
        renderer.configure_frame(ctx);

        // 배경 비활성 시 완전 투명. DComp 히트 테스트는 알파와 무관하게 창 단위다.
        let clear_color = if plan.background_visible {
            plan.background_color
        } else {
            0
        };
        renderer.clear(ctx, clear_color);
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::BeginClear, t);

        let mut frame_error = false;

        // 테두리 그리기
        if plan.border_visible
            && let Err(e) = renderer.draw_border(
                ctx,
                plan.client_width,
                plan.client_height,
                plan.border_width,
                plan.border_color,
            )
        {
            tracing::error!("D2D draw_border failed: {e}");
            frame_error = true;
        }
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::Border, t);

        // 텍스트 그리기
        for block in &plan.render_blocks {
            // bbox.max_height는 outline bitmap의 높이 상한(창 밖 래스터화 방지)이다.
            // layout 자체는 measure와 공유되는 1M 박스로 만들어진다.
            if let Err(e) = renderer.draw_text(
                ctx,
                block.slot,
                &block.text,
                crate::d2d::TextBox {
                    x: plan.margin_x as f32,
                    y: block.top,
                    max_width: plan.max_width,
                    max_height: plan.client_height,
                },
                &block.style,
            ) {
                tracing::error!("D2D draw_text failed: {e}");
                frame_error = true;
            }
        }
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::Text, t);

        // EndDraw가 flush하므로 바로 Present한다. 이벤트 기반 paint라 vsync는 기다리지 않는다.
        // 이 단계의 실패는 render target 상태를 신뢰할 수 없으므로 종류와 무관하게 재생성한다.
        if let Err(e) = composition.end_draw() {
            tracing::warn!("DComp end_draw failed ({e}); recreating stack");
            self.recover_render_stack();
            return Ok(());
        }
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::EndDraw, t);

        if frame_error {
            tracing::warn!("D2D frame contained draw errors; recreating stack");
            self.recover_render_stack();
            return Ok(());
        }

        if let Err(e) = composition.present(0) {
            // 일부 드라이버는 D3DDDIERR_DEVICEREMOVED처럼 DXGI와 다른 HRESULT를
            // 반환한다. Present 실패는 모두 동일한 안전한 복구 경로로 보낸다.
            tracing::warn!("DComp present failed ({e}); recreating stack");
            self.recover_render_stack();
            return Ok(());
        }
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::Present, t);

        self.update_hit_region(&plan);
        #[cfg(feature = "benchmark")]
        let _ = phase_record(PhaseField::HitRegion, t);

        self.prune_render_slots(&plan.render_blocks);

        Ok(())
    }

    /// Device loss 복구를 위해 합성 렌더러와 장치 종속 캐시를 비운다.
    fn handle_device_lost(&mut self) {
        self.composition = None;
        if let Some(d2d) = self.d2d_renderer.as_mut() {
            d2d.invalidate_device_caches();
        }
    }

    /// 렌더 스택을 비우고 다음 WM_PAINT에서 즉시 lazy-init하도록 예약한다.
    fn recover_render_stack(&mut self) {
        self.handle_device_lost();
        // SAFETY: hwnd는 App이 소유한 유효한 top-level window handle이다.
        unsafe {
            let _ = InvalidateRect(self.hwnd, std::ptr::null(), 0);
        }
    }

    pub(super) fn resize(&mut self, width: i32, height: i32) -> Result<()> {
        // DXGI가 거부하는 최소화 상태의 0 크기는 무시한다.
        let Some(size) = state::ClientSize::drawable(width, height) else {
            return Ok(());
        };

        // WM_DPICHANGED의 SetWindowPos가 WM_SIZE를 동기 발생시킨 뒤 client-size
        // 동기화가 이어질 수 있다. 같은 크기라면 ResizeBuffers와 중복 paint를 피한다.
        if self.model.runtime.client_size == size {
            return Ok(());
        }

        // interactive resize 중(WM_ENTERSIZEMOVE ~ WM_EXITSIZEMOVE)에는 재구축
        // 비용이 큰 작업을 모두 끝으로 미룬다. WM_SIZE는 드래그 내내 연속으로
        // 오는데, 매번 ResizeBuffers + layout/bitmap/hit-test 전 캐시 재빌드 +
        // 최대 16ms wait_for_back_buffer를 반복하면 리사이즈가 버벅인다. 최종
        // 목표 크기만 기록해 두고 WM_EXITSIZEMOVE에서 1회 적용한다.
        if self.model.runtime.resizing {
            self.model.runtime.pending_resize = Some(size);
            return Ok(());
        }

        self.model.runtime.client_size = size;

        // 첫 paint 전이면 lazy init이 현재 크기로 만들고, 이후에는 swap chain을 조정한다.
        let resize_failed = if let Some(composition) = self.composition.as_mut() {
            match composition.resize(width as u32, height as u32) {
                Ok(()) => false,
                Err(e) => {
                    tracing::error!("CompositionRenderer.resize failed: {e}");
                    true
                }
            }
        } else {
            false
        };
        if resize_failed {
            self.handle_device_lost();
        }

        self.paint()
    }
}
