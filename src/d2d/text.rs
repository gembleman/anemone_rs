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
    /// 텍스트 그리기 (외곽선, 그림자 포함).
    ///
    /// outline+shadow 결과를 중간 비트맵에 한 번 그려두고 매 paint 는
    /// `DrawBitmap` 1 회 + 본문 `DrawTextLayout` 1 회로 끝낸다. 캐시 miss
    /// 시에만 비트맵 재생성 — 본 앱은 텍스트가 자주 바뀌지 않으므로 hit
    /// 율이 매우 높다. paint 1 회의 GPU 명령은 9 개 (Clear + 3×Draw/Fill
    /// Geometry + 2×DrawTextLayout) 에서 3 개 (Clear + DrawBitmap +
    /// DrawTextLayout) 로 감소.
    ///
    /// outline/shadow 가 모두 없는 케이스는 비트맵을 만들지 않고 본문만
    /// 1 회 DrawTextLayout — 비트맵 비용보다 본문 1 회가 더 싸다.
    ///
    /// `target` 은 BeginDraw/EndDraw 사이의 활성 render target.
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

        // 키 비교용 ref view — alloc 없음. hit 판정 후 miss 일 때만
        // `to_owned()` 로 실제 키를 만든다. 폭주 시 매 paint 의 alloc 누적
        // 비용 제거.
        let key_ref = OutlineBitmapKeyRef::from_style(text, style, max_width, max_height);

        // 폭주 모드 (miss rate ≥ 5/8) 이면 비트맵 빌드 자체를 건너뛰고
        // outline geometry 를 paint 핫패스에서 직접 stroke+fill. paint 1 회
        // GPU 명령은 9 개로 늘지만 "비트맵 RT 생성+해제+합성" 의 ~1.4 ms
        // 비용을 회피해 baseline 수준 (~1 ms) 으로 회복한다.
        //
        // 폭주 모드의 hit 판정은 `last_key` (직전 paint 의 key) 와의 비교 —
        // 비트맵을 만들지 않더라도 텍스트가 안정화되면 ring 이 hit 으로
        // 채워져 자동 복귀한다.
        //
        // 폭주 모드 진입 시 `outline_bitmap = None` — stale 한
        // 비트맵 GPU 리소스를 즉시 회수하고, 폭주 종료 후 정상 경로 복귀 시
        // `ensure_outline_bitmap` 이 새 키로 빌드한다 (이 1 회는 자연 miss).
        if self.miss_tracker.is_overloaded() {
            self.outline_bitmap = None;
            let hit = self
                .miss_tracker
                .last_key
                .as_ref()
                .is_some_and(|k| key_ref.matches(k));
            // last_key 갱신은 동일 키 hit 일 때 alloc 을 건너뛰는 게 본질.
            // miss 인 경우에만 새 키를 만든다.
            if !hit {
                self.miss_tracker.last_key = Some(key_ref.to_owned());
            }
            self.miss_tracker.record(!hit);
            return self.draw_text_direct(target, text, bbox, style);
        }

        // 정상 경로 — hit/miss 판정은 outline_bitmap 의 실체 키 기준.
        // last_key 는 텍스트 안정 여부 판정용이라 실제 캐시 상태와 어긋날 수
        // 있어 (예: 폭주 모드 진입 직후), 정상 경로에서는 캐시 자체의 키와
        // 비교하는 게 정확하다.
        //
        // `ensure_outline_bitmap` 는 hit 여부와 layout 핸들을 같이 돌려줘
        // 동일 키 기준 layout 캐시도 한 번의 호출로 확보 — 별도
        // `get_or_create_layout` 재호출 없음.
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
            // outline 비트맵 합성. layout 원점이 비트맵의 (pad_left, pad_top)
            // 에 있으므로 dest 사각형은 (x - pad_left, y - pad_top) 부터.
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

    /// 폭주 모드 폴백 경로 — 비트맵 캐시를 건너뛰고 outline geometry
    /// 를 paint 핫패스에서 직접 stroke+fill.
    ///
    /// 그리기 순서는 비트맵 빌드 (`build_outline_bitmap`) 와 동일:
    /// 그림자 → outline2 → outline1 → 본문. `draw_outline_only` 는 비트맵
    /// RT 전용 (brush 캐시 우회) 이라 본 경로는 외부 RT 의 `brush_cache`
    /// 를 활용하는 별도 시퀀스로 구성. outline geometry 캐시 (항목 8)
    /// 는 그대로 공유 — `get_or_create_outline_geometry` 가 layout 원점
    /// (0, 0) 기준 PathGeometry 한 번만 만들어 두면 색/두께가 달라도
    /// `SetTransform` + 다른 brush 로 같은 geometry 를 재사용한다.
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

    /// 폭주 모드 폴백용 outline stroke+fill — caller (외부 paint RT) 에
    /// `SetTransform(translation(x, y))` 적용 → `DrawGeometry(thickness*2)`
    /// → `FillGeometry` → `SetTransform(identity)`. brush 는 외부 RT 의
    /// `brush_cache` 에서 받은 것을 그대로 사용.
    ///
    /// 호출 전에 `draw_text_direct` 가 `get_or_create_outline_geometry`
    /// 를 부르므로 `text_cache.outline` 은 항상 채워져 있다 — 도달 불가
    /// 분기는 `debug_assert!` 로 잡고 release 빌드에선 조용히 무시한다.
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

    /// outline+shadow 비트맵 캐시 진입.
    ///
    /// 키 일치 시 no-op, 불일치 시 새 비트맵을 만들어 캐시 교체. 새
    /// 비트맵 생성은 호출자의 render target 으로부터 `CreateCompatibleRender
    /// Target` 으로 보조 비트맵 render target 을 만들어, 거기에 outline /
    /// shadow 를 모두 한 번 그려 둔다. 본 비트맵 RT 의 BeginDraw/EndDraw 는
    /// 캐시 miss 시에만 발생하므로 paint 1 회 한정으로는 비용이 늘지만
    /// 정상 운용 (hit) 에서는 0.
    ///
    /// 반환: `(hit, text_layout)`. hit 은 caller (`draw_text`) 가
    /// miss-tracker ring 에 정확한 결과를 기록할 수 있도록 캐시 실체 기준
    /// (= 키 일치 + 비트맵 빌드 스킵) 으로 판정한다. text_layout 은 build
    /// 과정에서 어차피 확보하는 핸들을 그대로 돌려줘, 호출자가
    /// `get_or_create_layout` 을 다시 부르는 중복 호출을 없앤다.
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

    /// 보조 비트맵 RT 를 만들어 outline+shadow 를 모두 그린 뒤, 그 비트맵
    /// 과 본문 layout 핸들을 함께 돌려준다. layout 은 본 함수 내부에서
    /// metrics 산정에 어차피 확보하므로, 호출자가 재차
    /// `get_or_create_layout` 을 부를 필요가 없도록 그대로 넘긴다.
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
        // 정렬은 layout box 전체를 기준으로 glyph를 이동시킨다. 따라서
        // 중간 bitmap도 전체 layout box를 담고, italic/fallback glyph가
        // box 밖으로 돌출되는 양수 overhang만큼 각 변을 추가로 넓힌다.
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

        // SAFETY: target 은 caller 의 BeginDraw 안의 유효 render target.
        // CreateCompatibleRenderTarget 은 그 target 의 디바이스 위에 새 RT 를
        // 만들고, 같은 픽셀 포맷 + premultiplied alpha 를 자동 적용한다.
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
            // bm_rt 의 부모 render target view — 본 렌더러의 그리기 메서드들이
            // 받는 인터페이스. Deref 로 cast.
            let inner_rt: &ID2D1RenderTarget = &bm_rt;

            inner_rt.BeginDraw();
            inner_rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            // 비트맵은 투명으로 시작 — Clear(0).
            inner_rt.Clear(Some(&argb_to_color_f(0)));

            // 비트맵 안의 layout 원점은 (pad_left, pad_top) — 본 RT 에 그리는
            // outline / shadow 도 그 원점을 기준으로 한다.
            let origin_x = pad_left;
            let origin_y = pad_top;

            // 비트맵 RT 는 본 렌더러의 brush_cache (외부 RT 용) 와 분리돼야
            // 하므로 캐시 우회 — 직접 CreateSolidColorBrush 호출.
            // shadow 와 outline 색이 다르면 brush 2~3 개 생성되지만 캐시
            // miss 시에만 발생.

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

    /// 비트맵 RT 용 outline stroke+fill. brush 캐시를 우회하고 caller 가
    /// 미리 만든 brush 를 받는다 (비트맵 RT 에 속한 brush 와 본 렌더러의
    /// `brush_cache` (외부 RT 에 속함) 가 섞이지 않도록).
    ///
    /// outline geometry 자체는 layout 원점 (0, 0) 기준 캐시본을 그대로
    /// 재사용하고 SetTransform 으로 (x, y) 평행이동만 적용 — `stroke_fill_
    /// outline_at` 과 동일 패턴 (그쪽은 외부 RT + brush_cache 용).
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

    /// 텍스트가 차지하는 라인 단위 사각형을 클라이언트 좌표계로 돌려준다.
    ///
    /// DComp 합성 경로에서는 hit-testing 이 윈도우 사각 단위라 투명 배경
    /// 영역도 클릭/드래그를 가로채는 회귀가 있다. `WM_NCHITTEST` 에서
    /// 본 사각형 합집합 vs 점 검사로 그 영역만 `HTCAPTION` 로 잡고
    /// 나머지를 `HTTRANSPARENT` 반환하기 위함.
    ///
    /// `origin_x` / `origin_y` 는 텍스트 그리기 원점 (= margin), `inflate`
    /// 는 outline/shadow 두께를 흡수하기 위한 사각형 확장 (px). 빈 텍스트
    /// 면 빈 Vec.
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

        // SAFETY: layout 은 위에서 막 만든 유효 객체. HitTestTextRange 는
        // 먼저 None / 0 으로 호출해 필요한 metrics 개수를 받고, 그 크기로
        // 버퍼를 잡아 두 번째 호출에서 채운다 (E_NOT_SUFFICIENT_BUFFER
        // 는 정상 흐름이라 probe 의 Err 여부는 무시 가능).
        //
        // probe 결과 처리는 `needed` 값으로 분기:
        // - `needed == 0` → 빈 줄 (Ok) 이거나 진짜 에러 (Err). 어느 쪽이든
        //   두 번째 호출이 의미 없으므로 probe 의 Ok/Err 를 그대로 반환.
        // - `needed > 0` → 버퍼 부족이 정상 흐름. 그 크기로 재호출.
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
