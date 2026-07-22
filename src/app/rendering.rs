use std::sync::Arc;

use windows::{
    Win32::{Foundation::RECT, Graphics::Gdi::InvalidateRect, UI::WindowsAndMessaging::SetTimer},
    core::Result,
};

use super::{App, COMPOSITION_RETRY_TIMER, state};
use crate::config::{Config, TextStyle, TextType};
use crate::d2d::{CompositionRenderer, WaitOutcome};
use crate::window::TextRenderStyle;

struct RenderBlock {
    text: String,
    style: TextRenderStyle,
    top: f32,
    height: f32,
    gap_after: f32,
}

fn split_name(text: &str) -> Option<(&str, &str)> {
    text.split_once([':', '：'])
        .map(|(name, body)| (name.trim(), body.trim()))
        .filter(|(name, _)| !name.is_empty())
}

fn display_segments(config: &Config, original: &str, translated: &str) -> Vec<(TextType, String)> {
    let (original_name, original_body) = if config.separate_name {
        split_name(original)
            .map(|(name, body)| (Some(name), body))
            .unwrap_or((None, original))
    } else {
        (None, original)
    };
    let (translated_name, translated_body) = if config.separate_name {
        split_name(translated)
            .map(|(name, body)| (Some(name), body))
            .unwrap_or((None, translated))
    } else {
        (None, translated)
    };

    let mut segments = Vec::with_capacity(3);
    if config.show_name
        && let Some(name) = translated_name.or(original_name)
        && !name.is_empty()
    {
        segments.push((TextType::Name, name.to_string()));
    }
    if config.show_original && !original_body.is_empty() {
        segments.push((TextType::Original, original_body.to_string()));
    }
    if config.show_translation && !translated_body.is_empty() {
        segments.push((TextType::Translation, translated_body.to_string()));
    }
    segments
}

fn render_style(config: &Config, style: &TextStyle) -> TextRenderStyle {
    TextRenderStyle {
        font_size: style.size,
        font_face: Arc::from(style.font_face.as_str()),
        font_style: style.font_style,
        text_align: config.text_align,
        color: style.color_primary,
        outline1_size: style.outline1_size,
        outline1_color: style.color_outline1,
        outline2_size: style.outline2_size,
        outline2_color: style.color_outline2,
        shadow_enabled: style.shadow_enabled,
        shadow_color: style.color_shadow,
        shadow_offset_x: config.shadow_offset_x,
        shadow_offset_y: config.shadow_offset_y,
    }
}

fn build_render_blocks(config: &Config, original: &str, translated: &str) -> Vec<RenderBlock> {
    let mut top = config.text_margin_y as f32;
    display_segments(config, original, translated)
        .into_iter()
        .map(|(text_type, text)| {
            let style = render_style(config, config.get_text_style(text_type));
            let line_count = text.lines().count().max(1) as f32;
            let height = line_count * (style.font_size.max(1) as f32 * 1.35);
            let block = RenderBlock {
                text,
                style,
                top,
                height,
                gap_after: if text_type == TextType::Name {
                    config.name_margin as f32
                } else {
                    0.0
                },
            };
            top += height;
            top += block.gap_after;
            block
        })
        .collect()
}

fn build_notice_render_blocks(config: &Config, text: &str) -> Vec<RenderBlock> {
    let style = render_style(config, &config.translation_style);
    vec![RenderBlock {
        text: text.to_string(),
        top: config.text_margin_y as f32,
        height: text.lines().count().max(1) as f32 * (style.font_size.max(1) as f32 * 1.35),
        style,
        gap_after: 0.0,
    }]
}

#[inline]
fn pixels_to_dips(value: i32, dpi: u32) -> f32 {
    let dpi = if dpi == 0 { crate::dpi::BASE_DPI } else { dpi };
    value as f32 * crate::dpi::BASE_DPI as f32 / dpi as f32
}

#[inline]
fn dip_floor_to_pixels(value: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { crate::dpi::BASE_DPI } else { dpi };
    let scaled = value as i64 * dpi as i64;
    scaled.div_euclid(crate::dpi::BASE_DPI as i64) as i32
}

#[inline]
fn dip_ceil_to_pixels(value: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { crate::dpi::BASE_DPI } else { dpi };
    let scaled = value as i64 * dpi as i64;
    (-(-scaled).div_euclid(crate::dpi::BASE_DPI as i64)) as i32
}

fn dip_rect_to_pixels(rect: &RECT, dpi: u32) -> RECT {
    RECT {
        left: dip_floor_to_pixels(rect.left, dpi),
        top: dip_floor_to_pixels(rect.top, dpi),
        right: dip_ceil_to_pixels(rect.right, dpi),
        bottom: dip_ceil_to_pixels(rect.bottom, dpi),
    }
}

impl App {
    const FRAME_WAIT_TIMEOUT_MS: u32 = 16;

    fn text_layout_extent(size: f32, margin: i32) -> f32 {
        (size - margin.saturating_mul(2) as f32).max(1.0)
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

        let cfg = &self.model.config;

        // 설정 값 복사
        let background_visible = cfg.background_visible;
        let background_color = cfg.background_color;
        let border_visible = cfg.border_visible;
        let border_width = cfg.border_width;
        let border_color = cfg.border_color;

        let mut render_blocks = match self.model.runtime.overlay_notice {
            Some(notice) => build_notice_render_blocks(cfg, notice.text()),
            None => build_render_blocks(
                cfg,
                &self.model.runtime.original_text,
                &self.model.runtime.translated_text,
            ),
        };
        let margin_x = cfg.text_margin_x;
        let margin_y = cfg.text_margin_y;

        let composition = match self.composition.as_ref() {
            Some(c) => c,
            None => return Ok(()),
        };
        let dpi = crate::dpi::dpi_for_window(self.hwnd).max(1);
        let dpi_changed = composition.set_dpi(dpi as f32);
        let renderer = match self.d2d_renderer.as_mut() {
            Some(r) => r,
            None => return Ok(()),
        };
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
                match renderer.measure_text_height(&block.text, &block.style, max_width) {
                    Ok(height) => block.height = height,
                    Err(error) => {
                        tracing::warn!("DirectWrite text measurement failed: {error}");
                    }
                }
                top += block.height + block.gap_after;
            }
        }
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
            && let Err(e) =
                renderer.draw_border(ctx, client_width, client_height, border_width, border_color)
        {
            tracing::error!("D2D draw_border failed: {e}");
            frame_error = true;
        }
        #[cfg(feature = "benchmark")]
        let t = phase_record(PhaseField::Border, t);

        // 텍스트 그리기
        for block in &render_blocks {
            let max_height = (client_height - block.top - margin_y as f32)
                .max(1.0)
                .min(block.height.max(1.0));
            if let Err(e) = renderer.draw_text(
                ctx,
                &block.text,
                crate::d2d::TextBox {
                    x: margin_x as f32,
                    y: block.top,
                    max_width,
                    max_height,
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

        // 렌더러 borrow가 끝난 뒤 hit region을 갱신한다. 배경/테두리가 없고
        // text 사각형도 없으면 완전히 보이지 않는 frame이므로 입력도 받지 않는다.
        self.hit_region.clear();
        self.full_hit_region = background_visible || border_visible;
        if !self.full_hit_region
            && !render_blocks.is_empty()
            && let Some(d2d) = self.d2d_renderer.as_mut()
        {
            for block in &render_blocks {
                let max_height = (client_height - block.top - margin_y as f32)
                    .max(1.0)
                    .min(block.height.max(1.0));
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
                    &block.text,
                    &block.style,
                    crate::d2d::TextBox {
                        x: margin_x as f32,
                        y: block.top,
                        max_width,
                        max_height,
                    },
                    inflate,
                ) {
                    Ok(rects) => self
                        .hit_region
                        .extend(rects.iter().map(|rect| dip_rect_to_pixels(rect, dpi))),
                    Err(e) => tracing::warn!("compute_text_line_rects failed: {e}"),
                }
            }
        }
        #[cfg(feature = "benchmark")]
        let _ = phase_record(PhaseField::HitRegion, t);

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
            let _ = InvalidateRect(Some(self.hwnd), None, false);
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

fn composition_retry_delay_ms(failures: u32) -> u32 {
    const INITIAL_MS: u32 = 250;
    const MAX_SHIFT: u32 = 5;
    INITIAL_MS.saturating_mul(1u32 << failures.min(MAX_SHIFT))
}

#[cfg(test)]
#[path = "../../tests/unit/app/rendering.rs"]
mod tests;
