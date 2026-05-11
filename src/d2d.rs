//! Direct2D 기반 렌더러
//!
//! 그리기 메서드들은 모두 `&ID2D1RenderTarget` 을 받도록 일반화돼
//! `CompositionRenderer` (DComp 합성 경로) 의 `ID2D1DeviceContext` 와
//! 미래에 추가될 다른 render target 모두에서 그대로 사용된다.

use std::collections::HashMap;

use crate::util::to_wide;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
        },
    },
    core::*,
};
use windows_numerics::{Matrix3x2, Vector2};

use crate::window::TextRenderStyle;

// ── ARGB 컬러 변환 헬퍼 ─────────────────────────────────

/// ARGB u32를 D2D1_COLOR_F로 변환
#[inline]
fn argb_to_color_f(color: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        a: ((color >> 24) & 0xFF) as f32 / 255.0,
        r: ((color >> 16) & 0xFF) as f32 / 255.0,
        g: ((color >> 8) & 0xFF) as f32 / 255.0,
        b: (color & 0xFF) as f32 / 255.0,
    }
}

// ── font_style 비트 → DirectWrite 변환 ──────────────────

/// font_style 비트플래그(0: normal, 1: bold, 2: italic, 3: bold+italic)를
/// DirectWrite의 weight/style 쌍으로 변환
#[inline]
fn font_style_to_dwrite(bits: u8) -> (DWRITE_FONT_WEIGHT, DWRITE_FONT_STYLE) {
    let weight = if bits & 1 != 0 {
        DWRITE_FONT_WEIGHT_BOLD
    } else {
        DWRITE_FONT_WEIGHT_NORMAL
    };
    let style = if bits & 2 != 0 {
        DWRITE_FONT_STYLE_ITALIC
    } else {
        DWRITE_FONT_STYLE_NORMAL
    };
    (weight, style)
}

// ── OutlineTextRenderer ─────────────────────────────────

/// IDWriteTextRenderer 구현체 - 글리프를 ID2D1GeometrySink로 출력
#[windows::core::implement(IDWriteTextRenderer, IDWritePixelSnapping)]
struct OutlineTextRenderer {
    d2d_factory: ID2D1Factory1,
    sink: ID2D1GeometrySink,
}

impl OutlineTextRenderer {
    fn new(d2d_factory: ID2D1Factory1, sink: ID2D1GeometrySink) -> Self {
        Self { d2d_factory, sink }
    }
}

impl IDWritePixelSnapping_Impl for OutlineTextRenderer_Impl {
    fn IsPixelSnappingDisabled(
        &self,
        _clientdrawingcontext: *const std::ffi::c_void,
    ) -> windows::core::Result<BOOL> {
        Ok(FALSE)
    }

    fn GetCurrentTransform(
        &self,
        _clientdrawingcontext: *const std::ffi::c_void,
        transform: *mut DWRITE_MATRIX,
    ) -> windows::core::Result<()> {
        // SAFETY: transform is a valid out-pointer provided by DirectWrite. Writing an
        // identity matrix is the expected behavior for pixel snapping.
        unsafe {
            *transform = DWRITE_MATRIX {
                m11: 1.0,
                m12: 0.0,
                m21: 0.0,
                m22: 1.0,
                dx: 0.0,
                dy: 0.0,
            };
        }
        Ok(())
    }

    fn GetPixelsPerDip(
        &self,
        _clientdrawingcontext: *const std::ffi::c_void,
    ) -> windows::core::Result<f32> {
        Ok(1.0)
    }
}

impl IDWriteTextRenderer_Impl for OutlineTextRenderer_Impl {
    fn DrawGlyphRun(
        &self,
        _clientdrawingcontext: *const std::ffi::c_void,
        baselineoriginx: f32,
        baselineoriginy: f32,
        _measuringmode: DWRITE_MEASURING_MODE,
        glyphrun: *const DWRITE_GLYPH_RUN,
        _glyphrundescription: *const DWRITE_GLYPH_RUN_DESCRIPTION,
        _clientdrawingeffect: windows::core::Ref<'_, windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        // SAFETY: glyphrun is a valid pointer provided by DirectWrite's text layout engine.
        // The glyph data (indices, advances, offsets) are valid for the duration of this call.
        // D2D factory operations create valid COM objects.
        unsafe {
            let glyph_run = &*glyphrun;

            // FontFace에서 글리프 아웃라인을 geometry sink로 출력
            if let Some(font_face) = &*glyph_run.fontFace {
                // 임시 PathGeometry를 만들어서 글리프 아웃라인 추출.
                // Factory1::CreatePathGeometry 는 PathGeometry1 을 반환 — 부모
                // 인터페이스로 다운캐스트해 받는다 (이후 Open/Close 만 사용).
                let temp_geometry: ID2D1PathGeometry =
                    self.d2d_factory.CreatePathGeometry()?.into();
                let temp_sink = temp_geometry.Open()?;

                font_face.GetGlyphRunOutline(
                    glyph_run.fontEmSize,
                    glyph_run.glyphIndices,
                    Some(glyph_run.glyphAdvances),
                    Some(glyph_run.glyphOffsets),
                    glyph_run.glyphCount,
                    glyph_run.isSideways.into(),
                    glyph_run.bidiLevel & 1 != 0,
                    &temp_sink,
                )?;

                temp_sink.Close()?;

                // baseline 위치를 적용한 TransformedGeometry 생성
                let transform = Matrix3x2::translation(baselineoriginx, baselineoriginy);
                let transformed: ID2D1TransformedGeometry = self
                    .d2d_factory
                    .CreateTransformedGeometry(&temp_geometry, &transform)?;

                // TransformedGeometry를 최종 sink로 출력 (Simplify 사용)
                let geometry: ID2D1Geometry = transformed.cast()?;
                geometry.Simplify(
                    D2D1_GEOMETRY_SIMPLIFICATION_OPTION_CUBICS_AND_LINES,
                    None,
                    D2D1_DEFAULT_FLATTENING_TOLERANCE,
                    &self.sink,
                )?;
            }
        }
        Ok(())
    }

    fn DrawUnderline(
        &self,
        _clientdrawingcontext: *const std::ffi::c_void,
        _baselineoriginx: f32,
        _baselineoriginy: f32,
        _underline: *const DWRITE_UNDERLINE,
        _clientdrawingeffect: windows::core::Ref<'_, windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        // 외곽선에서는 밑줄 무시
        Ok(())
    }

    fn DrawStrikethrough(
        &self,
        _clientdrawingcontext: *const std::ffi::c_void,
        _baselineoriginx: f32,
        _baselineoriginy: f32,
        _strikethrough: *const DWRITE_STRIKETHROUGH,
        _clientdrawingeffect: windows::core::Ref<'_, windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        // 외곽선에서는 취소선 무시
        Ok(())
    }

    fn DrawInlineObject(
        &self,
        _clientdrawingcontext: *const std::ffi::c_void,
        _originx: f32,
        _originy: f32,
        _inlineobject: windows::core::Ref<'_, IDWriteInlineObject>,
        _issideways: BOOL,
        _isrighttoleft: BOOL,
        _clientdrawingeffect: windows::core::Ref<'_, windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        // 인라인 객체 무시
        Ok(())
    }
}

// ── D2DRenderer ─────────────────────────────────────────

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
/// 텍스트 layout / outline geometry 캐시 키.
///
/// layout/geometry 모양을 결정하는 입력만 포함 — 색상, 그림자 오프셋,
/// outline 두께 등 "칠하기 단계" 인자는 무관. f32 max_width/height 는
/// `to_bits()` 로 정확 비교 (NaN 가 들어올 일이 없는 사이즈 값들).
#[derive(Clone, Eq, PartialEq, Hash)]
struct TextLayoutKey {
    text: String,
    font_face: String,
    font_size: i32,
    font_style: u8,
    max_width_bits: u32,
    max_height_bits: u32,
}

/// 캐시 엔트리. outline geometry 는 첫 외곽선 그리기 호출 시 lazy 생성.
struct TextLayoutCache {
    key: TextLayoutKey,
    layout: IDWriteTextLayout,
    outline: Option<ID2D1PathGeometry>,
}

pub struct D2DRenderer {
    d2d_factory: ID2D1Factory1,
    dwrite_factory: IDWriteFactory,
    /// 프레임 단위 브러시 캐시 (ARGB 색상 → SolidColorBrush)
    brush_cache: HashMap<u32, ID2D1SolidColorBrush>,
    /// 외곽선 스트로크 스타일 캐시 (불변이므로 한 번만 생성)
    stroke_style: Option<ID2D1StrokeStyle>,
    /// 텍스트 layout + outline geometry 캐시 (크기 1 LRU).
    ///
    /// 현재 앱은 한 번에 한 텍스트만 표시하므로 1 entry 로 충분. 키가
    /// 일치하면 layout/geometry 재사용 → DirectWrite text shaping 과
    /// glyph outline → PathGeometry 변환 (paint 핫패스의 다수 비용)
    /// 을 건너뛴다.
    text_cache: Option<TextLayoutCache>,
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
    fn get_or_create_brush(
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

    /// 캐시 키 빌드.
    fn make_text_key(
        text: &str,
        style: &TextRenderStyle,
        max_width: f32,
        max_height: f32,
    ) -> TextLayoutKey {
        TextLayoutKey {
            text: text.to_string(),
            font_face: style.font_face.clone(),
            font_size: style.font_size,
            font_style: style.font_style,
            max_width_bits: max_width.to_bits(),
            max_height_bits: max_height.to_bits(),
        }
    }

    /// 캐시 hit 면 기존 layout, miss 면 새로 만들어 캐시 교체 후 반환.
    /// 키가 바뀌면 outline geometry 도 같이 무효화된다.
    fn get_or_create_layout(
        &mut self,
        text: &str,
        style: &TextRenderStyle,
        max_width: f32,
        max_height: f32,
    ) -> Result<IDWriteTextLayout> {
        let key = Self::make_text_key(text, style, max_width, max_height);
        if let Some(c) = &self.text_cache
            && c.key == key
        {
            return Ok(c.layout.clone());
        }
        let layout = self.create_text_layout_uncached(text, style, max_width, max_height)?;
        self.text_cache = Some(TextLayoutCache {
            key,
            layout: layout.clone(),
            outline: None,
        });
        Ok(layout)
    }

    /// outline geometry 캐시 진입. 같은 키의 layout 기준으로 한 번만
    /// `text_layout.Draw(OutlineTextRenderer)` 를 돌리고 그 결과를 보관.
    /// outline 은 layout 원점 (0, 0) 기준으로 만들어 두고, 그리기 측에서
    /// `SetTransform` 으로 (x, y) 평행이동만 곱해 재사용한다.
    fn get_or_create_outline_geometry(
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
    fn create_text_layout_uncached(
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
            self.dwrite_factory.CreateTextLayout(
                &text_wide,
                &text_format,
                max_width,
                max_height,
            )
        }
    }

    // ── 퍼블릭 렌더링 API ───────────────────────────────

    /// 한 프레임을 시작하기 직전에 호출. 브러시 캐시를 비운다.
    /// [`Self::configure_frame`] 이 내부에서 호출하므로, 별도 캐시 reset 만
    /// 필요한 경우에만 직접 사용한다.
    pub fn reset_frame_cache(&mut self) {
        self.brush_cache.clear();
    }

    /// 호출자가 매 프레임 호출하는 단일 진입점.
    ///
    /// 1) 직전 프레임 brush 캐시 폐기 + 2) target 의 안티앨리어싱 모드를
    /// grayscale 로 설정. `BeginDraw` 자체는 `CompositionRenderer::begin_draw`
    /// 가 이미 호출했다고 가정한다.
    pub fn configure_frame(&mut self, target: &ID2D1RenderTarget) {
        self.reset_frame_cache();
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
                &D2D_RECT_F { left: 0.0, top: 0.0, right: w, bottom: t },
                &brush,
            );
            // 하단
            target.FillRectangle(
                &D2D_RECT_F { left: 0.0, top: h - t, right: w, bottom: h },
                &brush,
            );
            // 좌측
            target.FillRectangle(
                &D2D_RECT_F { left: 0.0, top: 0.0, right: t, bottom: h },
                &brush,
            );
            // 우측
            target.FillRectangle(
                &D2D_RECT_F { left: w - t, top: 0.0, right: w, bottom: h },
                &brush,
            );
        }

        Ok(())
    }

    /// 텍스트 그리기 (외곽선, 그림자 포함).
    /// 렌더링 순서: 그림자 -> 외곽선2 -> 외곽선1 -> 주 텍스트.
    ///
    /// `target` 은 BeginDraw/EndDraw 사이의 활성 render target.
    pub fn draw_text(
        &mut self,
        target: &ID2D1RenderTarget,
        text: &str,
        x: f32,
        y: f32,
        max_width: f32,
        max_height: f32,
        style: &TextRenderStyle,
    ) -> Result<()> {
        let text_layout = self.get_or_create_layout(text, style, max_width, max_height)?;
        let outline_total = style.outline1_size + style.outline2_size;

        // SAFETY: target is a valid render target between BeginDraw/EndDraw.
        unsafe {
            // 1. 그림자 그리기
            if style.shadow_enabled && (style.shadow_offset_x != 0 || style.shadow_offset_y != 0) {
                let shadow_x = x + style.shadow_offset_x as f32;
                let shadow_y = y + style.shadow_offset_y as f32;

                if outline_total > 0 {
                    self.draw_text_outline(
                        target,
                        text,
                        style,
                        max_width,
                        max_height,
                        shadow_x,
                        shadow_y,
                        outline_total,
                        style.shadow_color,
                    )?;
                }

                let shadow_brush = self.get_or_create_brush(target, style.shadow_color)?;
                target.DrawTextLayout(
                    Vector2::new(shadow_x, shadow_y),
                    &text_layout,
                    &shadow_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            // 2. 외곽선2 그리기 (OutlineOut)
            if style.outline2_size > 0 && outline_total > 0 {
                self.draw_text_outline(
                    target,
                    text,
                    style,
                    max_width,
                    max_height,
                    x,
                    y,
                    outline_total,
                    style.outline2_color,
                )?;
            }

            // 3. 외곽선1 그리기 (OutlineIn)
            if style.outline1_size > 0 {
                self.draw_text_outline(
                    target,
                    text,
                    style,
                    max_width,
                    max_height,
                    x,
                    y,
                    style.outline1_size,
                    style.outline1_color,
                )?;
            }

            // 4. 주 텍스트 그리기
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

    /// Geometry 기반 텍스트 외곽선 그리기.
    /// 캐시된 outline PathGeometry (origin=0,0 기준) 를 `SetTransform` 평행이동
    /// 으로 (x, y) 에 그린다. paint 한 번에 최대 3 회 (shadow + outline2 +
    /// outline1) 호출돼도 geometry 자체는 한 번만 만들어 재사용된다.
    fn draw_text_outline(
        &mut self,
        target: &ID2D1RenderTarget,
        text: &str,
        style: &TextRenderStyle,
        max_width: f32,
        max_height: f32,
        x: f32,
        y: f32,
        thickness: i32,
        color: u32,
    ) -> Result<()> {
        let path_geometry =
            self.get_or_create_outline_geometry(text, style, max_width, max_height)?;

        // SAFETY: target 은 BeginDraw/EndDraw 사이의 유효 render target. 본
        // 메서드가 transform 을 임시로 평행이동으로 바꾼 뒤 항상 identity 로
        // 복구한다 — `configure_frame` 의 가정 (identity transform) 유지.
        unsafe {
            let brush = self.get_or_create_brush(target, color)?;
            target.SetTransform(&Matrix3x2::translation(x, y));

            target.DrawGeometry(
                &path_geometry,
                &brush,
                thickness as f32 * 2.0, // stroke는 양쪽으로 그려지므로 2배
                self.stroke_style.as_ref(),
            );
            target.FillGeometry(&path_geometry, &brush, None);

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
        max_width: f32,
        max_height: f32,
        origin_x: f32,
        origin_y: f32,
        inflate: f32,
    ) -> Result<Vec<RECT>> {
        if text.is_empty() {
            return Ok(Vec::new());
        }

        let layout = self.get_or_create_layout(text, style, max_width, max_height)?;
        let text_len: u32 = text.encode_utf16().count() as u32;
        if text_len == 0 {
            return Ok(Vec::new());
        }

        // SAFETY: layout 은 위에서 막 만든 유효 객체. HitTestTextRange 는
        // 먼저 None / 0 으로 호출해 필요한 metrics 개수를 받고, 그 크기로
        // 버퍼를 잡아 두 번째 호출에서 채운다 (E_NOT_SUFFICIENT_BUFFER
        // 는 정상 흐름).
        let metrics: Vec<DWRITE_HIT_TEST_METRICS> = unsafe {
            let mut needed: u32 = 0;
            let probe = layout.HitTestTextRange(0, text_len, 0.0, 0.0, None, &mut needed);
            // 첫 호출이 OK 면 (= 0 metrics 면) 빈 Vec 반환 — 빈 줄 케이스.
            if probe.is_ok() && needed == 0 {
                return Ok(Vec::new());
            }
            // ERROR_INSUFFICIENT_BUFFER 외 다른 에러는 그대로 전파.
            if let Err(e) = probe
                && needed == 0
            {
                return Err(e);
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
                RECT { left, top, right, bottom }
            })
            .collect();
        Ok(rects)
    }
}
