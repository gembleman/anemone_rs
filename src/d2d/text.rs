use crate::window::TextRenderStyle;
use windows::{
    Win32::{
        Foundation::RECT,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
        },
    },
    core::*,
};
use windows_numerics::{Matrix3x2, Vector2};

use super::{
    TextBox,
    cache::{EffectiveOutlineStyle, OutlineBitmap, OutlineBitmapKey, OutlineBitmapKeyRef},
    color::argb_to_color_f,
    renderer::D2DRenderer,
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
    layout_width: f32,
    layout_height: f32,
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
        width: (layout_origin_x + layout_width.max(0.0) + overhang_right + effect_right)
            .ceil()
            .max(1.0),
        height: (layout_origin_y + layout_height.max(0.0) + overhang_bottom + effect_bottom)
            .ceil()
            .max(1.0),
        layout_origin_x,
        layout_origin_y,
    }
}

impl D2DRenderer {
    /// 활성 render target에 본문과 cache된 outline/shadow를 그린다.
    /// 효과가 없으면 중간 bitmap 없이 본문만 그린다.
    pub fn draw_text(
        &mut self,
        target: &ID2D1RenderTarget,
        text: &str,
        bbox: TextBox,
        style: &TextRenderStyle,
    ) -> Result<()> {
        let TextBox {
            x,
            y,
            max_width,
            max_height,
        } = bbox;
        let effects = EffectiveOutlineStyle::from_style(style);
        let has_shadow = effects.has_shadow;
        let has_outline = effects.outline_total > 0;

        // outline / shadow 둘 다 없으면 본문 한 줄만 그린다.
        if !has_outline && !has_shadow {
            let text_layout = self.get_or_create_layout(text, style, max_width, max_height)?;
            // SAFETY: target is a valid render target between BeginDraw/EndDraw.
            unsafe {
                let text_brush = self.get_or_create_brush(target, style.color)?;
                target.DrawTextLayout(
                    Vector2::new(x, y),
                    &text_layout,
                    &text_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            return Ok(());
        }

        // 조회 key는 빌리고, miss일 때만 소유 key를 만든다.
        let key_ref = OutlineBitmapKeyRef::from_style(text, style, max_width, max_height);

        // Miss가 잦으면 bitmap 생성을 건너뛰고 geometry를 직접 그린다.
        // 직전 key가 안정되면 hit가 쌓여 자동 복귀한다.
        if self.miss_tracker.is_overloaded() {
            self.outline_bitmap = None;
            let hit = self
                .miss_tracker
                .last_key
                .as_ref()
                .is_some_and(|k| key_ref.matches(k));
            // 같은 key면 할당 없이 last_key를 유지한다.
            if !hit {
                self.miss_tracker.last_key = Some(key_ref.to_owned());
            }
            self.miss_tracker.record(!hit);
            return self.draw_text_direct(target, text, bbox, style);
        }

        // 정상 경로는 실제 bitmap key로 hit를 판정하고 layout도 함께 얻는다.
        let (hit, text_layout) =
            self.ensure_outline_bitmap(target, &key_ref, text, style, max_width, max_height)?;
        self.miss_tracker.record(!hit);
        // last_key 도 hit 여부에 따라 alloc 회피.
        let need_update_last = self
            .miss_tracker
            .last_key
            .as_ref()
            .is_none_or(|k| !key_ref.matches(k));
        if need_update_last {
            self.miss_tracker.last_key = Some(key_ref.to_owned());
        }

        // SAFETY: target is a valid render target between BeginDraw/EndDraw.
        unsafe {
            // Padding을 빼 bitmap의 layout 원점을 (x, y)에 맞춘다.
            if let Some(bm) = &self.outline_bitmap {
                let dest_left = x - bm.pad_left;
                let dest_top = y - bm.pad_top;
                let dest = D2D_RECT_F {
                    left: dest_left,
                    top: dest_top,
                    right: dest_left + bm.width,
                    bottom: dest_top + bm.height,
                };
                target.DrawBitmap(
                    &bm.bitmap,
                    Some(&dest),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
            }

            // 본문 텍스트.
            let text_brush = self.get_or_create_brush(target, style.color)?;
            target.DrawTextLayout(
                Vector2::new(x, y),
                &text_layout,
                &text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        Ok(())
    }

    /// Bitmap cache를 우회해 그림자, outline2, outline1, 본문을 직접 그린다.
    /// Layout 원점 기준 geometry와 외부 target의 brush cache는 재사용한다.
    fn draw_text_direct(
        &mut self,
        target: &ID2D1RenderTarget,
        text: &str,
        bbox: TextBox,
        style: &TextRenderStyle,
    ) -> Result<()> {
        let TextBox {
            x,
            y,
            max_width,
            max_height,
        } = bbox;
        let effects = EffectiveOutlineStyle::from_style(style);
        let outline_total = effects.outline_total;
        let has_shadow = effects.has_shadow;

        // outline geometry + layout 확보 (캐시 hit/miss 처리 포함).
        let _ = self.get_or_create_outline_geometry(text, style, max_width, max_height)?;
        let text_layout = self.get_or_create_layout(text, style, max_width, max_height)?;

        // SAFETY: target 은 caller (paint) 의 BeginDraw 안의 유효 RT.
        // SetTransform 은 각 블록 끝에서 identity 로 복구.
        unsafe {
            // 1. 그림자 (outline + 본문 모두 shadow_color 로).
            if has_shadow {
                let sx = x + effects.shadow_offset_x as f32;
                let sy = y + effects.shadow_offset_y as f32;
                let shadow_brush = self.get_or_create_brush(target, effects.shadow_color)?;
                if outline_total > 0 {
                    self.stroke_fill_outline_at(target, sx, sy, outline_total, &shadow_brush);
                }
                target.DrawTextLayout(
                    Vector2::new(sx, sy),
                    &text_layout,
                    &shadow_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            // 2. 외곽선2 (OutlineOut) — 전체 두께로 한 번.
            if effects.outline2_size > 0 && outline_total > 0 {
                let brush = self.get_or_create_brush(target, effects.outline2_color)?;
                self.stroke_fill_outline_at(target, x, y, outline_total, &brush);
            }

            // 3. 외곽선1 (OutlineIn) — outline1_size 두께.
            if effects.outline1_size > 0 {
                let brush = self.get_or_create_brush(target, effects.outline1_color)?;
                self.stroke_fill_outline_at(target, x, y, effects.outline1_size, &brush);
            }

            // 4. 본문.
            let text_brush = self.get_or_create_brush(target, style.color)?;
            target.DrawTextLayout(
                Vector2::new(x, y),
                &text_layout,
                &text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        Ok(())
    }

    /// Transform을 잠시 이동해 cache된 geometry를 stroke/fill하고 복원한다.
    fn stroke_fill_outline_at(
        &mut self,
        target: &ID2D1RenderTarget,
        x: f32,
        y: f32,
        thickness: i32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let Some(geometry) = self
            .text_cache
            .as_ref()
            .and_then(|c| c.outline.as_ref())
            .cloned()
        else {
            debug_assert!(
                false,
                "stroke_fill_outline_at: outline geometry missing — \
                 caller must call get_or_create_outline_geometry first"
            );
            return;
        };
        // SAFETY: target 은 caller 의 BeginDraw 안의 유효 RT.
        unsafe {
            target.SetTransform(&Matrix3x2::translation(x, y));
            target.DrawGeometry(
                &geometry,
                brush,
                thickness as f32 * 2.0,
                self.stroke_style.as_ref(),
            );
            target.FillGeometry(&geometry, brush, None);
            target.SetTransform(&Matrix3x2::identity());
        }
    }

    /// Outline/shadow bitmap을 조회하거나 만들고 `(hit, layout)`을 반환한다.
    fn ensure_outline_bitmap(
        &mut self,
        target: &ID2D1RenderTarget,
        key_ref: &OutlineBitmapKeyRef<'_>,
        text: &str,
        style: &TextRenderStyle,
        max_width: f32,
        max_height: f32,
    ) -> Result<(bool, IDWriteTextLayout)> {
        if let Some(c) = &self.outline_bitmap
            && key_ref.matches(&c.key)
        {
            // hit — 비트맵 재사용. layout 도 같은 키 기준 캐시 hit.
            let layout = self.get_or_create_layout(text, style, max_width, max_height)?;
            return Ok((true, layout));
        }
        // miss — 비트맵 빌드. 키는 이 시점에만 alloc.
        let owned_key = key_ref.to_owned();
        let (bm, layout) =
            self.build_outline_bitmap(target, text, style, max_width, max_height, owned_key)?;
        self.outline_bitmap = Some(bm);
        Ok((false, layout))
    }

    /// 호환 bitmap target에 효과를 그리고, 사용한 layout과 함께 반환한다.
    fn build_outline_bitmap(
        &mut self,
        target: &ID2D1RenderTarget,
        text: &str,
        style: &TextRenderStyle,
        max_width: f32,
        max_height: f32,
        key: OutlineBitmapKey,
    ) -> Result<(OutlineBitmap, IDWriteTextLayout)> {
        // 비트맵 패딩 계산. shadow 는 한 방향만 빠져나가므로 비대칭 패딩.
        let effects = EffectiveOutlineStyle::from_style(style);
        let outline_total = effects.outline_total as f32;
        let shadow_dx = effects.shadow_offset_x as f32;
        let shadow_dy = effects.shadow_offset_y as f32;
        // Layout box와 italic/fallback glyph의 양수 overhang까지 담는다.
        let layout = self.get_or_create_layout(text, style, max_width, max_height)?;
        let mut tm = DWRITE_TEXT_METRICS::default();
        // SAFETY: layout 은 위에서 막 확보. metrics는 out 파라미터.
        let overhang = unsafe {
            layout.GetMetrics(&mut tm)?;
            layout.GetOverhangMetrics()?
        };
        let bounds = compute_outline_bitmap_bounds(
            tm.layoutWidth,
            tm.layoutHeight,
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
            inner_rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            // 비트맵은 투명으로 시작 — Clear(0).
            inner_rt.Clear(Some(&argb_to_color_f(0)));

            // 비트맵 안의 layout 원점은 (pad_left, pad_top) — 본 RT 에 그리는
            // outline / shadow 도 그 원점을 기준으로 한다.
            let origin_x = pad_left;
            let origin_y = pad_top;

            // Brush는 target 종속이므로 외부 target의 cache를 쓰지 않는다.

            // 1. 그림자
            if effects.has_shadow {
                let sx = origin_x + shadow_dx;
                let sy = origin_y + shadow_dy;
                let shadow_brush =
                    inner_rt.CreateSolidColorBrush(&argb_to_color_f(effects.shadow_color), None)?;

                if outline_total > 0.0 {
                    self.draw_outline_only(
                        inner_rt,
                        text,
                        style,
                        max_width,
                        max_height,
                        sx,
                        sy,
                        outline_total as i32,
                        &shadow_brush,
                    )?;
                }
                inner_rt.DrawTextLayout(
                    Vector2::new(sx, sy),
                    &layout,
                    &shadow_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            // 2. 외곽선2 (OutlineOut)
            if effects.outline2_size > 0 && outline_total > 0.0 {
                let brush = inner_rt
                    .CreateSolidColorBrush(&argb_to_color_f(effects.outline2_color), None)?;
                self.draw_outline_only(
                    inner_rt,
                    text,
                    style,
                    max_width,
                    max_height,
                    origin_x,
                    origin_y,
                    outline_total as i32,
                    &brush,
                )?;
            }

            // 3. 외곽선1 (OutlineIn)
            if effects.outline1_size > 0 {
                let brush = inner_rt
                    .CreateSolidColorBrush(&argb_to_color_f(effects.outline1_color), None)?;
                self.draw_outline_only(
                    inner_rt,
                    text,
                    style,
                    max_width,
                    max_height,
                    origin_x,
                    origin_y,
                    effects.outline1_size,
                    &brush,
                )?;
            }

            inner_rt.EndDraw(None, None)?;

            // bm_rt 에서 비트맵 추출. 부모 인터페이스 메서드 호출.
            let bitmap: ID2D1Bitmap = bm_rt.GetBitmap()?;

            Ok((
                OutlineBitmap {
                    key,
                    bitmap,
                    width: bm_w,
                    height: bm_h,
                    pad_left,
                    pad_top,
                },
                layout,
            ))
        }
    }

    /// Bitmap target 전용 brush로 cache된 outline geometry를 stroke/fill한다.
    #[allow(clippy::too_many_arguments)]
    fn draw_outline_only(
        &mut self,
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
            self.get_or_create_outline_geometry(text, style, max_width, max_height)?;
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

    /// `WM_NCHITTEST`용 줄별 text 사각형을 client 좌표로 반환한다.
    /// `inflate`는 outline/shadow 여유이며 빈 text면 빈 배열을 반환한다.
    pub fn compute_text_line_rects(
        &mut self,
        text: &str,
        style: &TextRenderStyle,
        bbox: TextBox,
        inflate: f32,
    ) -> Result<Vec<RECT>> {
        if text.is_empty() {
            return Ok(Vec::new());
        }

        let TextBox {
            x: origin_x,
            y: origin_y,
            max_width,
            max_height,
        } = bbox;
        let layout = self.get_or_create_layout(text, style, max_width, max_height)?;
        let text_len: u32 = text.encode_utf16().count() as u32;
        if text_len == 0 {
            return Ok(Vec::new());
        }

        // SAFETY: 첫 호출로 크기를 얻고 정확한 buffer를 할당해 다시 호출한다.
        // 크기가 0이면 빈 결과와 실제 오류를 구분하도록 probe 결과를 반환한다.
        let metrics: Vec<DWRITE_HIT_TEST_METRICS> = unsafe {
            let mut needed: u32 = 0;
            let probe = layout.HitTestTextRange(0, text_len, 0.0, 0.0, None, &mut needed);
            if needed == 0 {
                return match probe {
                    Ok(()) => Ok(Vec::new()),
                    Err(e) => Err(e),
                };
            }
            let mut buf: Vec<DWRITE_HIT_TEST_METRICS> =
                vec![DWRITE_HIT_TEST_METRICS::default(); needed as usize];
            let mut actual: u32 = 0;
            layout.HitTestTextRange(0, text_len, 0.0, 0.0, Some(&mut buf), &mut actual)?;
            buf.truncate(actual as usize);
            buf
        };

        let inflate_i = inflate.ceil() as i32;
        let rects = metrics
            .into_iter()
            .map(|m| {
                let left = (origin_x + m.left).floor() as i32 - inflate_i;
                let top = (origin_y + m.top).floor() as i32 - inflate_i;
                let right = (origin_x + m.left + m.width).ceil() as i32 + inflate_i;
                let bottom = (origin_y + m.top + m.height).ceil() as i32 + inflate_i;
                RECT {
                    left,
                    top,
                    right,
                    bottom,
                }
            })
            .collect();
        Ok(rects)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/d2d/text.rs"]
mod tests;
