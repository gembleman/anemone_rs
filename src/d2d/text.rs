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
    cache::{
        EffectiveOutlineStyle, HitTestCache, HitTestKeyRef, MeasureKeyRef, OutlineBitmap,
        OutlineBitmapKey, OutlineBitmapKeyRef,
    },
    color::argb_to_color_f,
    renderer::D2DRenderer,
    style::TextRenderStyle,
};
use super::MeasureSlot;

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

impl D2DRenderer {
    /// 주어진 폭에서 DirectWrite가 계산한 실제 시각적 줄 높이를 반환한다.
    /// 명시적 개행 수가 아니라 layout metrics를 사용하므로 자동 줄바꿈도 포함한다.
    ///
    /// `slot`은 결과 캐시의 direct-mapped 슬롯이다. 텍스트 유형(Name/Original/
    /// Translation/Notice)별로 1블록만 측정되므로, "이름 고정 + 대사만 변경"
    /// 패턴에서 고정 블록의 측정 결과가 유지된다. 같은 슬롯이라도 키가 다르면
    /// miss로 보고 재측정한다. 캐시는 `invalidate_device_caches`(장치 손실·DPI
    /// 변경)에서 폐기된다.
    pub fn measure_text_height(
        &mut self,
        slot: MeasureSlot,
        text: &str,
        style: &TextRenderStyle,
        max_width: f32,
    ) -> Result<f32> {
        let key_ref = MeasureKeyRef::from_style(text, style, max_width);
        if let Some(height) = self.measure_cache.get(slot, &key_ref) {
            return Ok(height);
        }
        // 만든 layout은 draw/hit-test 경로와 공유한다 — `max_height`를
        // [`MEASURE_MAX_HEIGHT`]로 통일해 `get_or_create_layout`이 같은 키로
        // hit하게 한다 (텍스트 변경당 layout 생성 2→1회).
        let layout = self.get_or_create_layout(
            slot,
            text,
            style,
            max_width,
            super::MEASURE_MAX_HEIGHT,
        )?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe {
            layout.GetMetrics(&mut metrics)?;
        }
        let height = metrics.height.max(style.font_size.max(1) as f32);
        self.measure_cache.insert(slot, key_ref.to_owned(), height);
        Ok(height)
    }

    /// 활성 render target에 본문과 cache된 outline/shadow를 그린다.
    /// 효과가 없으면 중간 bitmap 없이 본문만 그린다.
    /// `slot`은 layout/outline/bitmap 캐시의 유형별 슬롯이다.
    ///
    /// `bbox.max_height`는 outline bitmap의 **높이 상한**(창 밖으로 나가는 텍스트의
    /// bitmap 생성을 막는 용도)으로만 쓰인다. layout 자체는 measure와 캐시를
    /// 공유하도록 [`MEASURE_MAX_HEIGHT`](super::MEASURE_MAX_HEIGHT)로 항상 만든다.
    pub fn draw_text(
        &mut self,
        target: &ID2D1RenderTarget,
        slot: MeasureSlot,
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
            let text_layout = self.get_or_create_layout(
                slot,
                text,
                style,
                max_width,
                super::MEASURE_MAX_HEIGHT,
            )?;
            // SAFETY: target is a valid render target between BeginDraw/EndDraw.
            unsafe {
                let text_brush = self.get_or_create_brush(target, style.color)?;
                target.DrawTextLayout(
                    Vector2::new(x, y),
                    &text_layout,
                    &text_brush,
                    D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                );
            }
            return Ok(());
        }

        // 조회 key는 빌리고, miss일 때만 소유 key를 만든다.
        let key_ref = OutlineBitmapKeyRef::from_style(text, style, max_width, max_height);

        // Miss가 잦으면 bitmap 생성을 건너뛰고 geometry를 직접 그린다.
        // 직전 key가 안정되면 hit가 쌓여 자동 복귀한다.
        if self.miss_tracker.is_overloaded(slot) {
            self.outline_bitmap[slot as usize] = None;
            let hit = self
                .miss_tracker
                .last_key(slot)
                .is_some_and(|k| key_ref.matches(k));
            self.miss_tracker.record(slot, !hit);
            let result = self.draw_text_direct(target, slot, text, bbox, style);
            // direct 경로가 만든 layout key의 문자열 소유권을 공유한다.
            if !hit
                && result.is_ok()
                && let Some(layout) = self.text_cache[slot as usize].as_ref()
            {
                self.miss_tracker
                    .set_last_key(slot, Some(key_ref.to_owned_reusing_layout(&layout.key)));
            }
            return result;
        }

        // 정상 경로는 실제 bitmap key로 hit를 판정하고 layout도 함께 얻는다.
        let (hit, text_layout) = match self
            .ensure_outline_bitmap(slot, target, &key_ref, text, style, max_width)
        {
            Ok(bitmap) => bitmap,
            Err(error) => {
                // 큰 overlay나 GPU 압박으로 compatible bitmap 생성이 실패해도 본문까지
                // 멈추지 않는다. 반복 실패는 miss tracker가 direct 경로로 전환한다.
                tracing::warn!(
                    "outline bitmap build failed ({error}); falling back to direct rendering"
                );
                self.outline_bitmap[slot as usize] = None;
                self.miss_tracker.record(slot, true);
                let result = self.draw_text_direct(target, slot, text, bbox, style);
                if result.is_ok()
                    && let Some(layout) = self.text_cache[slot as usize].as_ref()
                {
                    self.miss_tracker
                        .set_last_key(slot, Some(key_ref.to_owned_reusing_layout(&layout.key)));
                }
                return result;
            }
        };
        self.miss_tracker.record(slot, !hit);
        // last_key 도 hit 여부에 따라 alloc 회피.
        let need_update_last = self
            .miss_tracker
            .last_key(slot)
            .is_none_or(|k| !key_ref.matches(k));
        if need_update_last {
            self.miss_tracker.set_last_key(
                slot,
                self.outline_bitmap[slot as usize]
                    .as_ref()
                    .map(|bitmap| bitmap.key.clone()),
            );
        }

        // SAFETY: target is a valid render target between BeginDraw/EndDraw.
        unsafe {
            // Padding을 빼 bitmap의 layout 원점을 (x, y)에 맞춘다.
            if let Some(bm) = &self.outline_bitmap[slot as usize] {
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
                D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
            );
        }

        Ok(())
    }

    /// Bitmap cache를 우회해 그림자, outline2, outline1, 본문을 직접 그린다.
    /// Layout 원점 기준 geometry와 외부 target의 brush cache는 재사용한다.
    fn draw_text_direct(
        &mut self,
        target: &ID2D1RenderTarget,
        slot: MeasureSlot,
        text: &str,
        bbox: TextBox,
        style: &TextRenderStyle,
    ) -> Result<()> {
        let TextBox {
            x,
            y,
            max_width,
            max_height: _,
        } = bbox;
        let effects = EffectiveOutlineStyle::from_style(style);
        let outline_total = effects.outline_total;
        let has_shadow = effects.has_shadow;

        // outline geometry + layout 확보 (캐시 hit/miss 처리 포함).
        // layout/geometry는 measure·bitmap 경로와 공유하도록 1M 박스로 만든다.
        let _ = self.get_or_create_outline_geometry(
            slot,
            text,
            style,
            max_width,
            super::MEASURE_MAX_HEIGHT,
        )?;
        // 위에서 방금 캐시에 넣은 layout을 재조회(str 비교) 없이 쓴다.
        let text_layout = self
            .text_cache[slot as usize]
            .as_ref()
            .expect("layout cache populated by get_or_create_outline_geometry")
            .layout
            .clone();

        // SAFETY: target 은 caller (paint) 의 BeginDraw 안의 유효 RT.
        // SetTransform 은 각 블록 끝에서 identity 로 복구.
        unsafe {
            // 1. 그림자 (outline + 본문 모두 shadow_color 로).
            if has_shadow {
                let sx = x + effects.shadow_offset_x as f32;
                let sy = y + effects.shadow_offset_y as f32;
                let shadow_brush = self.get_or_create_brush(target, effects.shadow_color)?;
                if outline_total > 0 {
                    self.stroke_fill_outline_at(slot, target, sx, sy, outline_total, &shadow_brush);
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
                self.stroke_fill_outline_at(slot, target, x, y, outline_total, &brush);
            }

            // 3. 외곽선1 (OutlineIn) — outline1_size 두께.
            if effects.outline1_size > 0 {
                let brush = self.get_or_create_brush(target, effects.outline1_color)?;
                self.stroke_fill_outline_at(slot, target, x, y, effects.outline1_size, &brush);
            }

            // 4. 본문.
            let text_brush = self.get_or_create_brush(target, style.color)?;
            target.DrawTextLayout(
                Vector2::new(x, y),
                &text_layout,
                &text_brush,
                D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
            );
        }

        Ok(())
    }

    /// Transform을 잠시 이동해 cache된 geometry를 stroke/fill하고 복원한다.
    fn stroke_fill_outline_at(
        &mut self,
        slot: MeasureSlot,
        target: &ID2D1RenderTarget,
        x: f32,
        y: f32,
        thickness: i32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let Some(geometry) = self
            .text_cache[slot as usize]
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
    ///
    /// `key_ref`의 `max_height`는 bitmap 높이 상한이다 — layout은 measure와
    /// 캐시를 공유하도록 [`MEASURE_MAX_HEIGHT`](super::MEASURE_MAX_HEIGHT)로 만든다.
    #[allow(clippy::too_many_arguments)]
    fn ensure_outline_bitmap(
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
        let layout = self.get_or_create_layout(
            slot,
            text,
            style,
            max_width,
            super::MEASURE_MAX_HEIGHT,
        )?;
        let owned_key = key_ref.to_owned_reusing_layout(
            &self
                .text_cache[slot as usize]
                .as_ref()
                .expect("layout cache populated by get_or_create_layout")
                .key,
        );
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
            tm.height.max(tm.layoutHeight + overhang.bottom).min(max_height),
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
            let draw_result = (|| -> Result<()> {
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
                    let shadow_brush = inner_rt
                        .CreateSolidColorBrush(&argb_to_color_f(effects.shadow_color), None)?;

                    if outline_total > 0.0 {
                        self.draw_outline_only(
                            slot,
                            inner_rt,
                            text,
                            style,
                            max_width,
                            super::MEASURE_MAX_HEIGHT,
                            sx,
                            sy,
                            outline_total as i32,
                            &shadow_brush,
                        )?;
                    }
                    inner_rt.DrawTextLayout(
                        Vector2::new(sx, sy),
                        layout,
                        &shadow_brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }

                // 2. 외곽선2 (OutlineOut)
                if effects.outline2_size > 0 && outline_total > 0.0 {
                    let brush = inner_rt
                        .CreateSolidColorBrush(&argb_to_color_f(effects.outline2_color), None)?;
                    self.draw_outline_only(
                        slot,
                        inner_rt,
                        text,
                        style,
                        max_width,
                        super::MEASURE_MAX_HEIGHT,
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
                        slot,
                        inner_rt,
                        text,
                        style,
                        max_width,
                        super::MEASURE_MAX_HEIGHT,
                        origin_x,
                        origin_y,
                        effects.outline1_size,
                        &brush,
                    )?;
                }
                Ok(())
            })();
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

    /// Bitmap target 전용 brush로 cache된 outline geometry를 stroke/fill한다.
    #[allow(clippy::too_many_arguments)]
    fn draw_outline_only(
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

    /// `WM_NCHITTEST`용 줄별 text 사각형을 client 좌표로 반환한다.
    /// `inflate`는 outline/shadow 여유이며 빈 text면 빈 배열을 반환한다.
    /// `slot`은 layout 캐시의 유형별 슬롯이다.
    pub fn compute_text_line_rects(
        &mut self,
        slot: MeasureSlot,
        text: &str,
        style: &TextRenderStyle,
        bbox: TextBox,
        inflate: f32,
    ) -> Result<&[RECT]> {
        if text.is_empty() {
            self.hit_test_cache[slot as usize] = None;
            return Ok(&[]);
        }

        let TextBox {
            x: origin_x,
            y: origin_y,
            max_width,
            max_height: _,
        } = bbox;
        // layout은 measure와 공유하는 1M 박스로 만들어지므로 키도 같은 값을 쓴다.
        // bbox.max_height(= bitmap 높이 상한)를 넣으면 저장 키(1M)와 어긋나
        // 영구 miss가 된다 — hit-test 경로는 bbox.max_height를 쓰지 않는다.
        let key_ref = HitTestKeyRef::from_style(
            text,
            style,
            origin_x,
            origin_y,
            max_width,
            super::MEASURE_MAX_HEIGHT,
            inflate,
        );
        if self
            .hit_test_cache[slot as usize]
            .as_ref()
            .is_some_and(|cache| key_ref.matches(&cache.key))
        {
            return Ok(&self
                .hit_test_cache[slot as usize]
                .as_ref()
                .expect("hit-test cache checked above")
                .rects);
        }

        // layout은 measure/draw와 공유한다 — 캐시 키에 맞춰 1M 박스로 조회.
        let layout = self.get_or_create_layout(
            slot,
            text,
            style,
            max_width,
            super::MEASURE_MAX_HEIGHT,
        )?;
        // 함수 진입부의 `text.is_empty()` 가드 때문에 여기 텍스트는 항상
        // 비어있지 않다 — UTF-16 단위 수도 0이 될 수 없다.
        let text_len: u32 = text.encode_utf16().count() as u32;

        // SAFETY: 첫 호출로 크기를 얻고 정확한 buffer를 할당해 다시 호출한다.
        // 크기가 0이면 빈 결과와 실제 오류를 구분하도록 probe 결과를 반환한다.
        let metrics: Vec<DWRITE_HIT_TEST_METRICS> = unsafe {
            let mut needed: u32 = 0;
            let probe = layout.HitTestTextRange(0, text_len, 0.0, 0.0, None, &mut needed);
            if needed == 0 {
                return match probe {
                    Ok(()) => {
                        self.hit_test_cache[slot as usize] = None;
                        Ok(&[])
                    }
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
        let rects: Vec<RECT> = metrics
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
        let key = key_ref.to_owned_reusing_layout(
            &self
                .text_cache[slot as usize]
                .as_ref()
                .expect("layout cache populated by get_or_create_layout")
                .key,
        );
        self.hit_test_cache[slot as usize] = Some(HitTestCache { key, rects });
        Ok(&self
            .hit_test_cache[slot as usize]
            .as_ref()
            .expect("hit-test cache stored above")
            .rects)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/d2d/text.rs"]
mod tests;
