//! Outline/shadow 효과를 중간 bitmap에 합성하는 경로.
//!
//! [`super::text`]의 direct 경로와 동일한 layout/geometry 캐시를 공유하되,
//! 결과를 [`super::cache::OutlineBitmap`]으로 캐싱해 반복 paint의 GPU 비용을
//! 줄인다.

use windows::{
    Win32::{
        Foundation::E_UNEXPECTED,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
        },
    },
    core::*,
};
use windows_numerics::{Matrix3x2, Vector2};

use super::MeasureSlot;
use super::{
    cache::{EffectiveOutlineStyle, OutlineBitmap, OutlineBitmapKey, OutlineBitmapKeyRef},
    color::argb_to_color_f,
    renderer::D2DRenderer,
    style::TextRenderStyle,
};

#[derive(Clone, Copy, Debug, PartialEq)]
struct OutlineBitmapBounds {
    width: f32,
    height: f32,
    layout_origin_x: f32,
    layout_origin_y: f32,
}

#[allow(clippy::too_many_arguments)]
fn compute_outline_bitmap_bounds(
    content_width: f32,
    content_height: f32,
    overhang: DWRITE_OVERHANG_METRICS,
    outline_total: f32,
    shadow_dx: f32,
    shadow_dy: f32,
) -> OutlineBitmapBounds {
    let effect_left = outline_total + (-shadow_dx).max(0.0);
    let effect_top = outline_total + (-shadow_dy).max(0.0);
    let effect_right = outline_total + shadow_dx.max(0.0);
    let effect_bottom = outline_total + shadow_dy.max(0.0);
    let overhang_left = overhang.left.max(0.0);
    let overhang_top = overhang.top.max(0.0);
    let overhang_right = overhang.right.max(0.0);
    let overhang_bottom = overhang.bottom.max(0.0);
    let layout_origin_x = effect_left + overhang_left;
    let layout_origin_y = effect_top + overhang_top;

    OutlineBitmapBounds {
        width: (layout_origin_x + content_width.max(0.0) + overhang_right + effect_right)
            .ceil()
            .max(1.0),
        height: (layout_origin_y + content_height.max(0.0) + overhang_bottom + effect_bottom)
            .ceil()
            .max(1.0),
        layout_origin_x,
        layout_origin_y,
    }
}

/// [`D2DRenderer::paint_outline_bitmap_layers`] 호출에 필요한 텍스트/레이아웃 문맥.
///
/// 인자를 구조체로 묶어 그림자·외곽선1·외곽선2 세 레이어가 공유하는 값을
/// 한 번에 전달한다 (clippy::too_many_arguments 회피 목적이 아니라 세 레이어가
/// 실제로 같은 문맥을 공유한다는 사실을 드러내기 위함).
struct BitmapLayerContext<'a> {
    slot: MeasureSlot,
    text: &'a str,
    style: &'a TextRenderStyle,
    max_width: f32,
    layout: &'a IDWriteTextLayout,
    effects: EffectiveOutlineStyle,
    origin_x: f32,
    origin_y: f32,
}

impl D2DRenderer {
    /// Outline/shadow bitmap을 조회하거나 만들고 `(hit, layout)`을 반환한다.
    ///
    /// `key_ref`의 `max_height`는 bitmap 높이 상한이다 — layout은 measure와
    /// 캐시를 공유하도록 [`MEASURE_MAX_HEIGHT`](super::MEASURE_MAX_HEIGHT)로 만든다.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ensure_outline_bitmap(
        &mut self,
        slot: MeasureSlot,
        target: &ID2D1RenderTarget,
        key_ref: &OutlineBitmapKeyRef<'_>,
        text: &str,
        style: &TextRenderStyle,
        max_width: f32,
    ) -> Result<(bool, IDWriteTextLayout)> {
        if let Some(c) = &self.outline_bitmap[slot as usize]
            && key_ref.matches(&c.key)
        {
            // hit — 비트맵 재사용. bitmap 키가 layout 키를 포함하므로 layout
            // 일치가 보장된다 — layout 캐시 재검증(문자열 비교) 없이 보관된
            // layout을 그대로 쓴다.
            return Ok((true, c.layout.clone()));
        }
        // miss — 비트맵 빌드. 키는 이 시점에만 alloc.
        let layout =
            self.get_or_create_layout(slot, text, style, max_width, super::MEASURE_MAX_HEIGHT)?;
        // get_or_create_layout이 방금 populate했으므로 도달 불가능하지만,
        // 프로덕션 경로에서 panic 대신 명시적 오류로 처리한다.
        let Some(cached) = self.text_cache[slot as usize].as_ref() else {
            return Err(Error::new(
                E_UNEXPECTED,
                "layout cache not populated by get_or_create_layout",
            ));
        };
        let owned_key = key_ref.to_owned_reusing_layout(&cached.key);
        let bm = self.build_outline_bitmap(slot, target, style, owned_key, &layout)?;
        self.outline_bitmap[slot as usize] = Some(bm);
        Ok((false, layout))
    }

    /// 호환 bitmap target에 효과를 그리고, 사용한 layout과 함께 반환한다.
    ///
    /// `key.max_height_bits`는 bitmap 높이 상한(창 밖 텍스트의 래스터화 방지)
    /// 이다. outline geometry는 measure와 공유하는 1M layout으로 만들어
    /// direct 경로와 같은 캐시 항목을 쓴다.
    fn build_outline_bitmap(
        &mut self,
        slot: MeasureSlot,
        target: &ID2D1RenderTarget,
        style: &TextRenderStyle,
        key: OutlineBitmapKey,
        layout: &IDWriteTextLayout,
    ) -> Result<OutlineBitmap> {
        let text = key.text.as_ref();
        let max_width = f32::from_bits(key.max_width_bits);
        let max_height = f32::from_bits(key.max_height_bits);
        // 비트맵 패딩 계산. shadow 는 한 방향만 빠져나가므로 비대칭 패딩.
        let effects = EffectiveOutlineStyle::from_style(style);
        let outline_total = effects.outline_total as f32;
        let shadow_dx = effects.shadow_offset_x as f32;
        let shadow_dy = effects.shadow_offset_y as f32;
        // Layout box와 italic/fallback glyph의 양수 overhang까지 담는다.
        // `tm.width`/`tm.height`는 콘텐츠 크기이고 `layoutWidth`/`layoutHeight`는
        // 박스 크기다 — measure와 공유하는 1M 박스로는 비트맵이 터지므로
        // 콘텐츠 크기를 쓰고, 창 밖으로 나가는 부분은 `max_height`로 자른다.
        let mut tm = DWRITE_TEXT_METRICS::default();
        // SAFETY: layout 은 위에서 막 확보. metrics는 out 파라미터.
        let overhang = unsafe {
            layout.GetMetrics(&mut tm)?;
            layout.GetOverhangMetrics()?
        };
        let bounds = compute_outline_bitmap_bounds(
            // 이탤릭처럼 잉크가 콘텐츠 폭을 넘는 경우를 담는다 — overhang.right는
            // 박스 기준 값이므로 잉크 우측 끝(layoutWidth + overhang.right)과
            // 콘텐츠 폭 중 큰 쪽을 비트맵 폭으로 쓴다.
            tm.width.max(tm.layoutWidth + overhang.right),
            // overhang.bottom도 박스 기준 값이라 1M 박스에서는 죽는다 — 잉크 하단
            // 끝(layoutHeight + overhang.bottom)과 콘텐츠 높이 중 큰 쪽을 쓰고,
            // 창 높이 상한(max_height)으로 자른다.
            tm.height
                .max(tm.layoutHeight + overhang.bottom)
                .min(max_height),
            overhang,
            outline_total,
            shadow_dx,
            shadow_dy,
        );
        let bm_w = bounds.width;
        let bm_h = bounds.height;
        let pad_left = bounds.layout_origin_x;
        let pad_top = bounds.layout_origin_y;

        // SAFETY: target은 BeginDraw 중이며 호환 target이 같은 device/format을 쓴다.
        unsafe {
            let size = D2D_SIZE_F {
                width: bm_w,
                height: bm_h,
            };
            let bm_rt = target.CreateCompatibleRenderTarget(
                Some(&size),
                None,
                None,
                D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE,
            )?;
            // 공통 그리기 API가 받는 부모 render target view다.
            let inner_rt: &ID2D1RenderTarget = &bm_rt;

            inner_rt.BeginDraw();
            // 중간 resource 생성이 실패하더라도 BeginDraw/EndDraw 짝은 반드시 닫는다.
            // (아래 블록은 `?`를 쓰지 않으므로 closure 없이 값으로 평가한다.)
            inner_rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            // 비트맵은 투명으로 시작 — Clear(0).
            inner_rt.Clear(Some(&argb_to_color_f(0)));

            // 비트맵 안의 layout 원점은 (pad_left, pad_top) — 본 RT 에 그리는
            // outline / shadow 도 그 원점을 기준으로 한다.
            let draw_result = self.paint_outline_bitmap_layers(
                inner_rt,
                BitmapLayerContext {
                    slot,
                    text,
                    style,
                    max_width,
                    layout,
                    effects,
                    origin_x: pad_left,
                    origin_y: pad_top,
                },
            );
            let end_result = inner_rt.EndDraw(None, None);
            draw_result?;
            end_result?;

            // bm_rt 에서 비트맵 추출. 부모 인터페이스 메서드 호출.
            let bitmap: ID2D1Bitmap = bm_rt.GetBitmap()?;

            Ok(OutlineBitmap {
                key,
                bitmap,
                layout: layout.clone(),
                width: bm_w,
                height: bm_h,
                pad_left,
                pad_top,
            })
        }
    }

    /// 비트맵 안에 그림자 → 외곽선2 → 외곽선1 순서로 효과 레이어를 그린다.
    ///
    /// [`Self::build_outline_bitmap`]의 `BeginDraw`/`EndDraw` 클로저에서만 호출되며,
    /// 그리기 순서와 조건은 원래 `build_outline_bitmap` 본문과 동일하다.
    fn paint_outline_bitmap_layers(
        &mut self,
        inner_rt: &ID2D1RenderTarget,
        ctx: BitmapLayerContext<'_>,
    ) -> Result<()> {
        let effects = ctx.effects;
        let outline_total = effects.outline_total as f32;
        let shadow_dx = effects.shadow_offset_x as f32;
        let shadow_dy = effects.shadow_offset_y as f32;

        // SAFETY: inner_rt 는 caller(build_outline_bitmap) 가 BeginDraw 한 유효
        // 비트맵 RT.
        unsafe {
            // 1. 그림자
            if effects.has_shadow {
                let sx = ctx.origin_x + shadow_dx;
                let sy = ctx.origin_y + shadow_dy;
                let shadow_brush =
                    inner_rt.CreateSolidColorBrush(&argb_to_color_f(effects.shadow_color), None)?;

                if outline_total > 0.0 {
                    self.draw_outline_only(
                        ctx.slot,
                        inner_rt,
                        ctx.text,
                        ctx.style,
                        ctx.max_width,
                        super::MEASURE_MAX_HEIGHT,
                        sx,
                        sy,
                        outline_total as i32,
                        &shadow_brush,
                    )?;
                }
                inner_rt.DrawTextLayout(
                    Vector2::new(sx, sy),
                    ctx.layout,
                    &shadow_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            // 2. 외곽선2 (OutlineOut)
            if effects.outline2_size > 0 && outline_total > 0.0 {
                let brush = inner_rt
                    .CreateSolidColorBrush(&argb_to_color_f(effects.outline2_color), None)?;
                self.draw_outline_only(
                    ctx.slot,
                    inner_rt,
                    ctx.text,
                    ctx.style,
                    ctx.max_width,
                    super::MEASURE_MAX_HEIGHT,
                    ctx.origin_x,
                    ctx.origin_y,
                    outline_total as i32,
                    &brush,
                )?;
            }

            // 3. 외곽선1 (OutlineIn)
            if effects.outline1_size > 0 {
                let brush = inner_rt
                    .CreateSolidColorBrush(&argb_to_color_f(effects.outline1_color), None)?;
                self.draw_outline_only(
                    ctx.slot,
                    inner_rt,
                    ctx.text,
                    ctx.style,
                    ctx.max_width,
                    super::MEASURE_MAX_HEIGHT,
                    ctx.origin_x,
                    ctx.origin_y,
                    effects.outline1_size,
                    &brush,
                )?;
            }
            Ok(())
        }
    }

    /// Bitmap target 전용 brush로 cache된 outline geometry를 stroke/fill한다.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_outline_only(
        &mut self,
        slot: MeasureSlot,
        target: &ID2D1RenderTarget,
        text: &str,
        style: &TextRenderStyle,
        max_width: f32,
        max_height: f32,
        x: f32,
        y: f32,
        thickness: i32,
        brush: &ID2D1SolidColorBrush,
    ) -> Result<()> {
        let path_geometry =
            self.get_or_create_outline_geometry(slot, text, style, max_width, max_height)?;
        // SAFETY: target 은 caller (build_outline_bitmap) 가 BeginDraw 한 유효
        // 비트맵 RT. transform 은 함수 끝에서 identity 로 복구.
        unsafe {
            target.SetTransform(&Matrix3x2::translation(x, y));
            target.DrawGeometry(
                &path_geometry,
                brush,
                thickness as f32 * 2.0,
                self.stroke_style.as_ref(),
            );
            target.FillGeometry(&path_geometry, brush, None);
            target.SetTransform(&Matrix3x2::identity());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/unit/d2d/text_bitmap.rs"]
mod tests;
