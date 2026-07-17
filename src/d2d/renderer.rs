use std::collections::HashMap;

use crate::{util::to_wide, window::TextRenderStyle};
use windows::{
    Win32::Graphics::{
        Direct2D::{Common::*, *},
        DirectWrite::*,
    },
    core::*,
};
use windows_numerics::Matrix3x2;

use super::{
    cache::*,
    color::{argb_to_color_f, font_style_to_dwrite},
    outline_text_renderer::OutlineTextRenderer,
};

/// Direct2D 기반 렌더러.
///
/// 그리기 메서드들은 `&ID2D1RenderTarget` 을 받아 호출 측이 보유한
/// render target (현재는 `CompositionRenderer` 의 `ID2D1DeviceContext`)
/// 위에 그린다. `BeginDraw`/`EndDraw`/`Present` 는 호출 측이 책임지며,
/// 본 타입은 한 프레임 시작 시 [`Self::configure_frame`] 으로 캐시
/// reset + AA 모드 설정만 수행한다.
///
/// `d2d_factory` 는 [`ID2D1Factory1`] 로 보관한다 — `CreateDevice` 가
/// 필요한 `CompositionRenderer` 가 [`Self::factory`] 로 받아 같은 factory
/// 위에 D2D Device 를 만들도록 한다. 같은 factory 트리 안에 머물러야
/// brush/geometry/text-layout 같은 본 렌더러의 객체들이 합성 경로의
/// `ID2D1DeviceContext` 위에서 거부되지 않는다 (D2DERR_WRONG_FACTORY 방지).
pub struct D2DRenderer {
    pub(super) d2d_factory: ID2D1Factory1,
    pub(super) dwrite_factory: IDWriteFactory,
    /// 프레임 단위 브러시 캐시 (ARGB 색상 → SolidColorBrush)
    pub(super) brush_cache: HashMap<u32, ID2D1SolidColorBrush>,
    /// 외곽선 스트로크 스타일 캐시 (불변이므로 한 번만 생성)
    pub(super) stroke_style: Option<ID2D1StrokeStyle>,
    /// 텍스트 layout + outline geometry 캐시 (크기 1 LRU).
    ///
    /// 현재 앱은 한 번에 한 텍스트만 표시하므로 1 entry 로 충분. 키가
    /// 일치하면 layout/geometry 재사용 → DirectWrite text shaping 과
    /// glyph outline → PathGeometry 변환 (paint 핫패스의 다수 비용)
    /// 을 건너뛴다.
    pub(super) text_cache: Option<TextLayoutCache>,
    /// outline+shadow 비트맵 캐시 (크기 1 LRU).
    ///
    /// paint 1 회의 GPU 명령을 9 개 (Clear + 3×DrawGeometry +
    /// 3×FillGeometry + 2×DrawTextLayout) 에서 3 개
    /// (Clear + DrawBitmap + DrawTextLayout) 로 줄인다. paint floor 의 ~95% 를
    /// 차지하던 outline stroke+fill 비용 제거가 핵심.
    pub(super) outline_bitmap: Option<OutlineBitmap>,
    /// 캐시 miss 비율 추적 — 폭주 시 비트맵 경로 우회.
    pub(super) miss_tracker: MissTracker,
}

impl D2DRenderer {
    /// D2DRenderer 생성
    pub fn new() -> Result<Self> {
        // SAFETY: D2D1CreateFactory and DWriteCreateFactory are COM factory functions that
        // return valid COM interface pointers on success.
        unsafe {
            let d2d_factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite_factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            // Factory1::CreateStrokeStyle 은 STROKE_STYLE_PROPERTIES1 을 받는다 —
            // 본 모듈은 base 필드만 사용하므로 부모 인터페이스 메서드를 명시 호출해
            // STROKE_STYLE_PROPERTIES (base) 그대로 통과시킨다.
            let parent_factory: &ID2D1Factory = &d2d_factory;
            let stroke_style = parent_factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 1.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                },
                None,
            )?;

            Ok(Self {
                d2d_factory,
                dwrite_factory,
                brush_cache: HashMap::new(),
                stroke_style: Some(stroke_style),
                text_cache: None,
                outline_bitmap: None,
                miss_tracker: MissTracker::new(),
            })
        }
    }

    /// 본 렌더러가 보유한 D2D factory 를 노출한다.
    ///
    /// `CompositionRenderer::new` 에 넘겨주면 같은 factory 위에 D2D Device 를
    /// 만들어, 본 렌더러의 brush/geometry/text-layout 이 합성 경로의
    /// device context 에서도 거부되지 않는다.
    pub fn factory(&self) -> &ID2D1Factory1 {
        &self.d2d_factory
    }

    /// ARGB 색상으로 SolidColorBrush를 가져온다 (프레임 내 캐시 활용).
    ///
    /// `target` 은 `ID2D1RenderTarget` 으로 받는다 — DCRenderTarget 과
    /// DeviceContext 둘 다 이 인터페이스를 상속하므로 Deref coercion
    /// (`&dc_target` 또는 `&device_context`) 으로 전달 가능.
    pub(super) fn get_or_create_brush(
        &mut self,
        target: &ID2D1RenderTarget,
        color: u32,
    ) -> Result<ID2D1SolidColorBrush> {
        if let Some(brush) = self.brush_cache.get(&color) {
            return Ok(brush.clone());
        }
        // SAFETY: target is a valid render target. CreateSolidColorBrush is called with
        // valid color and brush properties.
        let brush = unsafe {
            target.CreateSolidColorBrush(
                &argb_to_color_f(color),
                Some(&D2D1_BRUSH_PROPERTIES {
                    opacity: 1.0,
                    transform: Matrix3x2::identity(),
                }),
            )?
        };
        self.brush_cache.insert(color, brush.clone());
        Ok(brush)
    }

    /// 캐시 hit 면 기존 layout, miss 면 새로 만들어 캐시 교체 후 반환.
    /// 키가 바뀌면 outline geometry 도 같이 무효화된다.
    ///
    /// hit 판정은 [`LayoutKeyRef`] 로 alloc 없이 처리 — `Arc::from(&str)` 은
    /// miss 시 새 캐시 엔트리 생성 시점에만 발생한다.
    pub(super) fn get_or_create_layout(
        &mut self,
        text: &str,
        style: &TextRenderStyle,
        max_width: f32,
        max_height: f32,
    ) -> Result<IDWriteTextLayout> {
        let key_ref = LayoutKeyRef::from_style(text, style, max_width, max_height);
        if let Some(c) = &self.text_cache
            && key_ref.matches(&c.key)
        {
            return Ok(c.layout.clone());
        }
        let layout = self.create_text_layout_uncached(text, style, max_width, max_height)?;
        self.text_cache = Some(TextLayoutCache {
            key: key_ref.to_owned(),
            layout: layout.clone(),
            outline: None,
        });
        Ok(layout)
    }

    /// outline geometry 캐시 진입. 같은 키의 layout 기준으로 한 번만
    /// `text_layout.Draw(OutlineTextRenderer)` 를 돌리고 그 결과를 보관.
    /// outline 은 layout 원점 (0, 0) 기준으로 만들어 두고, 그리기 측에서
    /// `SetTransform` 으로 (x, y) 평행이동만 곱해 재사용한다.
    pub(super) fn get_or_create_outline_geometry(
        &mut self,
        text: &str,
        style: &TextRenderStyle,
        max_width: f32,
        max_height: f32,
    ) -> Result<ID2D1PathGeometry> {
        // layout 먼저 확보 (캐시 hit/miss 처리 포함).
        let _ = self.get_or_create_layout(text, style, max_width, max_height)?;

        if let Some(c) = &self.text_cache
            && let Some(g) = &c.outline
        {
            return Ok(g.clone());
        }

        // outline geometry 신규 생성.
        // SAFETY: d2d_factory 는 유효한 COM 객체. text_layout 도 위에서 확보.
        let path_geometry: ID2D1PathGeometry = unsafe {
            let geom: ID2D1PathGeometry = self.d2d_factory.CreatePathGeometry()?.into();
            let sink = geom.Open()?;
            let renderer: IDWriteTextRenderer =
                OutlineTextRenderer::new(self.d2d_factory.clone(), sink.clone()).into();
            let layout = self
                .text_cache
                .as_ref()
                .expect("text_cache populated by get_or_create_layout")
                .layout
                .clone();
            layout.Draw(None, &renderer, 0.0, 0.0)?;
            sink.Close()?;
            geom
        };

        if let Some(c) = self.text_cache.as_mut() {
            c.outline = Some(path_geometry.clone());
        }
        Ok(path_geometry)
    }

    /// TextRenderStyle에서 IDWriteTextLayout 생성 (캐시 미경유 - 내부용).
    pub(super) fn create_text_layout_uncached(
        &self,
        text: &str,
        style: &TextRenderStyle,
        max_width: f32,
        max_height: f32,
    ) -> Result<IDWriteTextLayout> {
        // SAFETY: DirectWrite factory creates valid text format and layout objects.
        // font_face_wide is a valid null-terminated UTF-16 string.
        unsafe {
            let font_face_wide = to_wide(&style.font_face);
            let (font_weight, font_style_dw) = font_style_to_dwrite(style.font_style);

            let text_format = self.dwrite_factory.CreateTextFormat(
                PCWSTR(font_face_wide.as_ptr()),
                None,
                font_weight,
                font_style_dw,
                DWRITE_FONT_STRETCH_NORMAL,
                style.font_size as f32,
                w!(""),
            )?;

            let text_wide: Vec<u16> = text.encode_utf16().collect();
            self.dwrite_factory
                .CreateTextLayout(&text_wide, &text_format, max_width, max_height)
        }
    }

    // ── 퍼블릭 렌더링 API ───────────────────────────────

    /// device-bound 캐시 (brush, text layout, outline bitmap) 를 모두 폐기.
    ///
    /// 호출 시점:
    /// - device-lost (`D2DERR_RECREATE_TARGET`) 복구 직후 — 캐시된 D2D
    ///   객체들은 옛 device 에 묶여 있어 새 RT 에서 거부됨.
    /// - render target 의 디바이스가 바뀌는 경우 (예: 합성 경로 재초기화).
    ///
    /// `App::paint` 가 `flush` / `end_draw` / `present` 의 device-lost
    /// HRESULT 를 감지하면 호출. 캐시 일괄 폐기 후 `CompositionRenderer` 를
    /// drop 하면 다음 paint 의 lazy-init 분기가 새 device 위에서 다시
    /// 만든다 (`App::handle_device_lost`).
    pub fn invalidate_device_caches(&mut self) {
        self.brush_cache.clear();
        self.text_cache = None;
        self.outline_bitmap = None;
        // miss_tracker.last_key 는 비트맵 의존이 아니므로 유지해도 무해하나,
        // 정합성 차원에서 같이 reset.
        self.miss_tracker = MissTracker::new();
    }

    /// 호출자가 매 프레임 호출하는 단일 진입점.
    ///
    /// target 의 안티앨리어싱 모드를 grayscale 로 설정한다. `BeginDraw` 자체는
    /// `CompositionRenderer::begin_draw` 가 이미 호출했다고 가정.
    ///
    /// **brush_cache 는 프레임 간 재사용한다** — 합성 경로의
    /// `ID2D1DeviceContext` 는 단일 인스턴스를 모든 paint 에서 재사용하므로
    /// brush 도 device 가 살아 있는 한 유효. device-lost 시에만
    /// [`Self::invalidate_device_caches`] 로 일괄 폐기.
    pub fn configure_frame(&mut self, target: &ID2D1RenderTarget) {
        // SAFETY: target is a valid render target between BeginDraw/EndDraw (caller's responsibility).
        unsafe {
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        }
    }

    /// 배경 클리어 (ARGB).
    ///
    /// `target` 은 BeginDraw/EndDraw 사이의 활성 render target.
    pub fn clear(&self, target: &ID2D1RenderTarget, color: u32) {
        // SAFETY: target is valid and we are between BeginDraw/EndDraw (caller's responsibility).
        unsafe {
            target.Clear(Some(&argb_to_color_f(color)));
        }
    }

    /// 테두리 그리기 (ARGB).
    ///
    /// `target` 은 BeginDraw/EndDraw 사이의 활성 render target.
    /// `width` / `height` 는 그릴 영역 크기 (px).
    pub fn draw_border(
        &mut self,
        target: &ID2D1RenderTarget,
        width: i32,
        height: i32,
        thickness: i32,
        color: u32,
    ) -> Result<()> {
        let w = width as f32;
        let h = height as f32;
        let t = thickness as f32;

        // SAFETY: target is a valid render target between BeginDraw/EndDraw.
        unsafe {
            let brush = self.get_or_create_brush(target, color)?;

            // 상단
            target.FillRectangle(
                &D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: w,
                    bottom: t,
                },
                &brush,
            );
            // 하단
            target.FillRectangle(
                &D2D_RECT_F {
                    left: 0.0,
                    top: h - t,
                    right: w,
                    bottom: h,
                },
                &brush,
            );
            // 좌측
            target.FillRectangle(
                &D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: t,
                    bottom: h,
                },
                &brush,
            );
            // 우측
            target.FillRectangle(
                &D2D_RECT_F {
                    left: w - t,
                    top: 0.0,
                    right: w,
                    bottom: h,
                },
                &brush,
            );
        }

        Ok(())
    }
}
