use std::sync::Arc;

use windows::{
    Win32::{
        Foundation::D2DERR_RECREATE_TARGET,
        Graphics::Dxgi::{DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET},
        Graphics::Gdi::InvalidateRect,
        UI::WindowsAndMessaging::SetTimer,
    },
    core::{Error, Result},
};

use super::{App, COMPOSITION_RETRY_TIMER, state};
use crate::d2d::{CompositionRenderer, WaitOutcome};
use crate::window::TextRenderStyle;

impl App {
    const FRAME_WAIT_TIMEOUT_MS: u32 = 16;

    fn text_layout_extent(size: i32, margin: i32) -> f32 {
        size.saturating_sub(margin.saturating_mul(2)).max(1) as f32
    }

    pub(super) fn paint(&mut self) -> Result<()> {
        #[cfg(feature = "benchmark")]
        use super::bench::{PhaseField, phase_now, phase_record};

        // Recorder가 꺼져 있으면 phase hook은 사실상 no-op이다.
        #[cfg(feature = "benchmark")]
        let t = phase_now();
        // 클라이언트 크기가 생기는 첫 paint에서 합성 렌더러를 붙인다.
        if self.composition.is_none() {
            if self.composition_retry_scheduled {
                return Ok(());
            }
            // D2D factory를 공유해 모든 그리기 자원의 호환성을 보장한다.
            let Some(d2d_renderer) = self.d2d_renderer.as_ref() else {
                tracing::warn!("paint: d2d_renderer 미초기화 — 합성 렌더러 부착 보류");
                return Ok(());
            };
            match CompositionRenderer::new(self.hwnd, d2d_renderer.factory()) {
                Ok(c) => {
                    tracing::info!("DComp composition renderer initialized");
                    self.composition = Some(c);
                    self.composition_init_failures = 0;
                }
                Err(e) => {
                    let delay_ms = composition_retry_delay_ms(self.composition_init_failures);
                    self.composition_init_failures =
                        self.composition_init_failures.saturating_add(1);
                    // SAFETY: hwnd는 App이 소유한다. callback 없는 window timer는
                    // WM_TIMER를 같은 UI thread의 window procedure로 보낸다.
                    let timer = unsafe {
                        SetTimer(Some(self.hwnd), COMPOSITION_RETRY_TIMER, delay_ms, None)
                    };
                    self.composition_retry_scheduled = timer != 0;
                    tracing::error!("CompositionRenderer init failed: {e}; retry in {delay_ms} ms");
                    return Ok(());
                }
            }
        }
        #[cfg(feature = "benchmark")]
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
        if self.render_font_face.as_ref() != text_style.font_face {
            self.render_font_face = Arc::from(text_style.font_face.as_str());
        }
        let render_style = TextRenderStyle {
            font_size: text_style.size,
            font_face: Arc::clone(&self.render_font_face),
            font_style: text_style.font_style,
            text_align: cfg.text_align,
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
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::Setup, t);

        // 한 프레임만 기다린다. Timeout은 재예약하고 API 오류는 스택을 재생성한다.
        match composition.wait_for_back_buffer(Self::FRAME_WAIT_TIMEOUT_MS) {
            WaitOutcome::Ready => {}
            WaitOutcome::Timeout => {
                tracing::debug!("DComp back buffer wait timed out; retrying next paint");
                // SAFETY: hwnd는 App이 소유한 유효한 top-level window handle.
                unsafe {
                    let _ = InvalidateRect(Some(self.hwnd), None, false);
                }
                return Ok(());
            }
            WaitOutcome::Failed(e) => {
                tracing::error!("DComp back buffer wait failed: {e}; recreating stack");
                self.handle_device_lost();
                // SAFETY: 새 스택을 다음 WM_PAINT에서 lazy-init하기 위해 예약.
                unsafe {
                    let _ = InvalidateRect(Some(self.hwnd), None, false);
                }
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
        let clear_color = if background_visible {
            background_color
        } else {
            0
        };
        renderer.clear(ctx, clear_color);
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::BeginClear, t);

        let mut frame_error = false;

        // 테두리 그리기
        if border_visible
            && let Err(e) = renderer.draw_border(
                ctx,
                self.state.client_size.width,
                self.state.client_size.height,
                border_width,
                border_color,
            )
        {
            tracing::error!("D2D draw_border failed: {e}");
            frame_error = true;
        }
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::Border, t);

        // 텍스트 그리기
        if !self.state.current_text.is_empty() {
            let max_width = Self::text_layout_extent(self.state.client_size.width, margin_x);
            let max_height = Self::text_layout_extent(self.state.client_size.height, margin_y);
            if let Err(e) = renderer.draw_text(
                ctx,
                &self.state.current_text,
                crate::d2d::TextBox {
                    x: margin_x as f32,
                    y: margin_y as f32,
                    max_width,
                    max_height,
                },
                &render_style,
            ) {
                tracing::error!("D2D draw_text failed: {e}");
                frame_error = true;
            }
        }
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::Text, t);

        // EndDraw가 flush하므로 바로 Present한다. 이벤트 기반 paint라 vsync는 기다리지 않는다.
        // Device loss면 캐시와 합성 스택을 버리고 다음 paint에서 다시 만든다.
        if let Err(e) = composition.end_draw() {
            if Self::is_device_lost(&e) {
                tracing::warn!("DComp end_draw: device lost ({e}), recreating stack");
                self.handle_device_lost();
                return Ok(());
            }
            tracing::error!("DComp end_draw failed: {e}");
            // EndDraw가 실패한 frame은 완결되지 않았으므로 Present하지 않는다.
            return Ok(());
        }
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::EndDraw, t);

        if frame_error {
            tracing::warn!("D2D frame contained draw errors; skipping present");
            return Ok(());
        }

        if let Err(e) = composition.present(0) {
            if Self::is_device_lost(&e) {
                tracing::warn!("DComp present: device lost ({e}), recreating stack");
                self.handle_device_lost();
                return Ok(());
            }
            tracing::error!("DComp present failed: {e}");
        }
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::Present, t);

        // 렌더러 borrow가 끝난 뒤 hit region을 갱신한다. 빈 값은 창 전체를 뜻한다.
        self.hit_region.clear();
        if !background_visible && !self.state.current_text.is_empty() {
            let max_width = Self::text_layout_extent(self.state.client_size.width, margin_x);
            let max_height = Self::text_layout_extent(self.state.client_size.height, margin_y);
            // Outline과 양방향 shadow 상한을 합산하며 최솟값에서도 포화시킨다.
            let shadow_inflate = if render_style.shadow_enabled {
                render_style
                    .shadow_offset_x
                    .saturating_abs()
                    .saturating_add(render_style.shadow_offset_y.saturating_abs())
            } else {
                0
            };
            let inflate = render_style
                .outline1_size
                .saturating_add(render_style.outline2_size)
                .saturating_add(shadow_inflate)
                .saturating_add(1) as f32;
            if let Some(d2d) = self.d2d_renderer.as_mut() {
                match d2d.compute_text_line_rects(
                    &self.state.current_text,
                    &render_style,
                    crate::d2d::TextBox {
                        x: margin_x as f32,
                        y: margin_y as f32,
                        max_width,
                        max_height,
                    },
                    inflate,
                ) {
                    Ok(rects) => {
                        self.hit_region.clear();
                        self.hit_region.extend_from_slice(rects);
                    }
                    Err(e) => tracing::warn!("compute_text_line_rects failed: {e}"),
                }
            }
        }
        #[cfg(feature = "benchmark")]
        let _ = phase_record(PhaseField::HitRegion, t);

        Ok(())
    }

    /// EndDraw/Present 오류가 렌더 스택 재생성이 필요한 device loss인지 판정한다.
    fn is_device_lost(e: &Error) -> bool {
        let code = e.code();
        code == D2DERR_RECREATE_TARGET
            || code == DXGI_ERROR_DEVICE_REMOVED
            || code == DXGI_ERROR_DEVICE_RESET
    }

    /// Device loss 복구를 위해 합성 렌더러와 장치 종속 캐시를 비운다.
    fn handle_device_lost(&mut self) {
        self.composition = None;
        if let Some(d2d) = self.d2d_renderer.as_mut() {
            d2d.invalidate_device_caches();
        }
    }

    pub(super) fn resize(&mut self, width: i32, height: i32) -> Result<()> {
        // DXGI가 거부하는 최소화 상태의 0 크기는 무시한다.
        let Some(size) = state::ClientSize::drawable(width, height) else {
            return Ok(());
        };

        self.state.client_size = size;

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

fn composition_retry_delay_ms(failures: u32) -> u32 {
    const INITIAL_MS: u32 = 250;
    const MAX_SHIFT: u32 = 5;
    INITIAL_MS.saturating_mul(1u32 << failures.min(MAX_SHIFT))
}

#[cfg(test)]
#[path = "../../tests/unit/app/rendering.rs"]
mod tests;
