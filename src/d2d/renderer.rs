use std::collections::HashMap;

use crate::window::TextRenderStyle;
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
    color::{argb_to_color_f, font_style_to_dwrite, text_align_to_dwrite},
    outline_text_renderer::OutlineTextRenderer,
};

/// 호출자가 관리하는 render target에 그리는 Direct2D 렌더러.
///
/// `BeginDraw`/`EndDraw`/`Present`는 호출자가 담당한다. Factory를 합성 렌더러와
/// 공유하며 장치 종속 cache는 device loss 전까지 재사용한다.
pub struct D2DRenderer {
    pub(super) d2d_factory: ID2D1Factory1,
    pub(super) dwrite_factory: IDWriteFactory,
    /// device 수명 단위 브러시 캐시 (ARGB 색상 → SolidColorBrush)
    pub(super) brush_cache: HashMap<u32, ID2D1SolidColorBrush>,
    /// 외곽선 스트로크 스타일 캐시 (불변이므로 한 번만 생성)
    pub(super) stroke_style: Option<ID2D1StrokeStyle>,
    /// 현재 표시 중인 text layout과 outline geometry cache.
    pub(super) text_cache: Option<TextLayoutCache>,
    /// 현재 outline과 shadow를 합성한 bitmap cache.
    pub(super) outline_bitmap: Option<OutlineBitmap>,
    /// 투명 배경의 `WM_NCHITTEST`용 줄별 사각형 cache.
    pub(super) hit_test_cache: Option<HitTestCache>,
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

            // Base 속성을 받는 부모 interface의 CreateStrokeStyle을 호출한다.
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
                hit_test_cache: None,
                miss_tracker: MissTracker::new(),
            })
        }
    }

    /// 합성 렌더러와 공유할 D2D factory를 반환한다.
    pub fn factory(&self) -> &ID2D1Factory1 {
        &self.d2d_factory
    }

    /// ARGB 색상의 brush를 장치 수명 cache에서 가져온다.
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

    /// Layout cache를 조회하고 miss면 outline geometry와 함께 교체한다.
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

    /// Layout 원점 기준 outline geometry를 만들거나 cache에서 가져온다.
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
        debug_assert!(style.font_size > 0, "font_size must be positive");
        debug_assert!(
            style.outline1_size >= 0,
            "outline1_size must be non-negative"
        );
        debug_assert!(
            style.outline2_size >= 0,
            "outline2_size must be non-negative"
        );
        debug_assert!(max_width.is_finite() && max_width > 0.0);
        debug_assert!(max_height.is_finite() && max_height > 0.0);
        // SAFETY: DirectWrite factory creates valid text format and layout objects.
        unsafe {
            let (font_weight, font_style_dw) = font_style_to_dwrite(style.font_style);
            let font_face: &str = style.font_face.as_ref();

            let text_format = self.dwrite_factory.CreateTextFormat(
                &HSTRING::from(font_face),
                None,
                font_weight,
                font_style_dw,
                DWRITE_FONT_STRETCH_NORMAL,
                style.font_size as f32,
                w!(""),
            )?;
            text_format.SetTextAlignment(text_align_to_dwrite(style.text_align))?;

            let text_wide: Vec<u16> = text.encode_utf16().collect();
            self.dwrite_factory
                .CreateTextLayout(&text_wide, &text_format, max_width, max_height)
        }
    }

    // ── 퍼블릭 렌더링 API ───────────────────────────────

    /// Device loss나 render target 교체 후 모든 장치 종속 cache를 비운다.
    pub fn invalidate_device_caches(&mut self) {
        self.brush_cache.clear();
        self.text_cache = None;
        self.outline_bitmap = None;
        self.hit_test_cache = None;
        // 추적 상태도 함께 초기화한다.
        self.miss_tracker = MissTracker::new();
    }

    /// 활성 frame의 antialiasing을 설정한다. Brush cache는 frame 사이에도 유지한다.
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
