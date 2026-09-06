//! 본문·outline·shadow 텍스트 그리기 진입점.
//!
//! Bitmap 캐시 경로는 [`super::text_bitmap`], 측정은 [`super::text_measure`],
//! hit-test는 [`super::text_hit_test`]로 분리했다. 이 파일은 `draw_text`가
//! bitmap 캐시와 direct 렌더링 중 어느 경로로 갈지 고르는 로직과, direct
//! 경로 자체(그림자/외곽선/본문을 매 프레임 geometry로 그리는 폴백)를 담는다.

use windows::{
    Win32::{
        Foundation::E_UNEXPECTED,
        Graphics::Direct2D::{Common::*, *},
    },
    core::*,
};
use windows_numerics::{Matrix3x2, Vector2};

use super::MeasureSlot;
use super::{
    TextBox,
    cache::{EffectiveOutlineStyle, OutlineBitmapKeyRef},
    renderer::D2DRenderer,
    style::TextRenderStyle,
};

impl D2DRenderer {
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
            let text_layout =
                self.get_or_create_layout(slot, text, style, max_width, super::MEASURE_MAX_HEIGHT)?;
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
        let (hit, text_layout) =
            match self.ensure_outline_bitmap(slot, target, &key_ref, text, style, max_width) {
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
        let Some(cached) = self.text_cache[slot as usize].as_ref() else {
            // get_or_create_outline_geometry가 방금 populate했으므로 도달 불가능하지만,
            // 프로덕션 경로에서 panic 대신 명시적 오류로 처리한다.
            return Err(Error::new(
                E_UNEXPECTED,
                "layout cache not populated by get_or_create_outline_geometry",
            ));
        };
        let text_layout = cached.layout.clone();

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
        let Some(geometry) = self.text_cache[slot as usize]
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
}
