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

        // phase 측정 hook 의 시작 시점. recorder 비활성 시 phase_now() 는 0 반환,
        // phase_record() 는 no-op (None 체크 1 회) — 정상 paint 경로 overhead 거의 0.
        #[cfg(feature = "benchmark")]
        let t = phase_now();
        // 합성 렌더러 lazy init — 첫 paint 시 부착.
        // hwnd 가 보이는 시점 (`ShowWindow` 이후) 이어야 클라이언트 사이즈가 양수다.
        if self.composition.is_none() {
            if self.composition_retry_scheduled {
                return Ok(());
            }
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
        let render_style = TextRenderStyle {
            font_size: text_style.size,
            font_face: Arc::from(text_style.font_face.as_str()),
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

        // waitable swap chain: 다음 back buffer 가 사용 가능해질 때까지 명시
        // 대기. UI 스레드를 장시간 막지 않도록 한 프레임만 기다린다. timeout은
        // paint를 다시 예약하고, API failure는 handle을 포함한 스택을 재생성한다.
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

        // 배경 클리어. 배경 비활성 시 ARGB=0 으로 완전 투명. DComp 합성
        // 경로는 hit-testing 이 윈도우 단위라 layered 시절의 "α=1 트릭"
        // (완전 투명이면 클릭이 통과되지 않음 방지) 은 더 이상 필요/유효하지
        // 않다 — α 0 픽셀이든 1 픽셀이든 윈도우 사각 전체가 클릭을 잡는다.
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

        // EndDraw + Present. EndDraw 가 내부적으로 GPU 명령 큐를 flush 하므로
        // 별도 Flush 호출은 두지 않는다.
        // sync_interval=0: 응답성 우선 (paint 는 이벤트 기반이라 매 프레임 호출되지
        // 않으므로 GPU 큐 백프레셔 위험 낮음). baseline 의 UpdateLayeredWindow 도
        // vsync 미대기였으니 동일 정책.
        //
        // device-lost (`D2DERR_RECREATE_TARGET` / `DXGI_ERROR_DEVICE_REMOVED` /
        // `DXGI_ERROR_DEVICE_RESET`) 감지 시 즉시 종료하고 self.handle_device_lost()
        // 로 캐시·합성 렌더러를 폐기. 다음 paint 가 lazy-init 분기에서 다시
        // 만든다.
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

        // hit_region 갱신 — `&mut self.d2d_renderer` 와 충돌하지 않도록 본
        // 블록의 가변 borrow 가 풀린 뒤 별도 호출. `background_visible=true`
        // 또는 텍스트가 비어 있으면 빈 Vec → WM_NCHITTEST 가 윈도우 사각
        // 전체를 HTCAPTION 으로 잡는 기존 동작 유지.
        self.hit_region.clear();
        if !background_visible && !self.state.current_text.is_empty() {
            let max_width = Self::text_layout_extent(self.state.client_size.width, margin_x);
            let max_height = Self::text_layout_extent(self.state.client_size.height, margin_y);
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
                    Ok(rects) => self.hit_region = rects,
                    Err(e) => tracing::warn!("compute_text_line_rects failed: {e}"),
                }
            }
        }
        #[cfg(feature = "benchmark")]
        let _ = phase_record(PhaseField::HitRegion, t);

        Ok(())
    }

    /// end_draw/present 의 에러가 D2D/DXGI 디바이스 손실인지 판정.
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

    pub(super) fn resize(&mut self, width: i32, height: i32) -> Result<()> {
        // 0 사이즈 (minimize) 는 paint/resize 모두 스킵 — DXGI ResizeBuffers 가
        // 0 사이즈를 거부하며, 어차피 그릴 면적도 없다.
        let Some(size) = state::ClientSize::drawable(width, height) else {
            return Ok(());
        };

        self.state.client_size = size;

        // 합성 렌더러가 이미 부착된 상태면 swap chain 도 따라 키운다.
        // 첫 paint 전 (lazy init 직전) 의 WM_SIZE 는 self.composition 이 None 이라
        // 자연 무시된다 — 다음 paint 의 lazy init 이 새 사이즈로 swap chain 을 만든다.
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
