//! Direct2D 기반 렌더러
//!
//! ID2D1DCRenderTarget을 사용하여 레이어드 윈도우와 호환되는
//! Direct2D 렌더링을 제공합니다.

use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Gdi::HDC,
        },
    },
    core::*,
};
use windows_numerics::{Matrix3x2, Vector2};

use crate::window::TextRenderStyle;

/// IDWriteTextRenderer 구현체 - 글리프를 ID2D1GeometrySink로 출력
#[windows::core::implement(IDWriteTextRenderer, IDWritePixelSnapping)]
struct OutlineTextRenderer {
    d2d_factory: ID2D1Factory,
    sink: ID2D1GeometrySink,
}

impl OutlineTextRenderer {
    fn new(d2d_factory: ID2D1Factory, sink: ID2D1GeometrySink) -> Self {
        Self {
            d2d_factory,
            sink,
        }
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
        unsafe {
            let glyph_run = &*glyphrun;

            // FontFace에서 글리프 아웃라인을 geometry sink로 출력
            if let Some(font_face) = &*glyph_run.fontFace {
                // 임시 PathGeometry를 만들어서 글리프 아웃라인 추출
                let temp_geometry: ID2D1PathGeometry = self.d2d_factory.CreatePathGeometry()?;
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
                let transformed: ID2D1TransformedGeometry = self.d2d_factory
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

/// Direct2D 기반 렌더러
pub struct D2DRenderer {
    d2d_factory: ID2D1Factory,
    dwrite_factory: IDWriteFactory,
    render_target: Option<ID2D1DCRenderTarget>,
    bound_width: i32,
    bound_height: i32,
}

impl D2DRenderer {
    /// D2DRenderer 생성
    pub fn new() -> Result<Self> {
        unsafe {
            // D2D Factory 생성
            let d2d_factory: ID2D1Factory =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;

            // DirectWrite Factory 생성
            let dwrite_factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            Ok(Self {
                d2d_factory,
                dwrite_factory,
                render_target: None,
                bound_width: 0,
                bound_height: 0,
            })
        }
    }

    /// DC에 렌더 타겟 바인딩
    pub fn bind_dc(&mut self, hdc: HDC, width: i32, height: i32) -> Result<()> {
        unsafe {
            // 렌더 타겟이 없으면 생성
            if self.render_target.is_none() {
                let props = D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 0.0,
                    dpiY: 0.0,
                    usage: D2D1_RENDER_TARGET_USAGE_NONE,
                    minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
                };

                let target = self.d2d_factory.CreateDCRenderTarget(&props)?;
                self.render_target = Some(target);
            }

            // DC 바인딩
            let rect = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };

            if let Some(ref target) = self.render_target {
                target.BindDC(hdc, &rect)?;
            }

            self.bound_width = width;
            self.bound_height = height;

            Ok(())
        }
    }

    /// 렌더링 시작
    pub fn begin_draw(&self) {
        if let Some(ref target) = self.render_target {
            unsafe {
                target.BeginDraw();
                // 안티앨리어싱 설정 (투명 배경에서는 Grayscale AA 사용)
                target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            }
        }
    }

    /// 렌더링 종료
    pub fn end_draw(&self) -> Result<()> {
        if let Some(ref target) = self.render_target {
            unsafe {
                target.EndDraw(None, None)?;
            }
        }
        Ok(())
    }

    /// 배경 클리어 (ARGB)
    pub fn clear(&self, color: u32) {
        if let Some(ref target) = self.render_target {
            let a = ((color >> 24) & 0xFF) as f32 / 255.0;
            let r = ((color >> 16) & 0xFF) as f32 / 255.0;
            let g = ((color >> 8) & 0xFF) as f32 / 255.0;
            let b = (color & 0xFF) as f32 / 255.0;

            unsafe {
                target.Clear(Some(&D2D1_COLOR_F { r, g, b, a }));
            }
        }
    }

    /// 사각형 채우기 (ARGB)
    pub fn fill_rect(&self, x: f32, y: f32, w: f32, h: f32, color: u32) -> Result<()> {
        let target = match &self.render_target {
            Some(t) => t,
            None => return Ok(()),
        };

        unsafe {
            let brush = self.create_solid_brush(target, color)?;
            let rect = D2D_RECT_F {
                left: x,
                top: y,
                right: x + w,
                bottom: y + h,
            };
            target.FillRectangle(&rect, &brush);
        }

        Ok(())
    }

    /// 테두리 그리기 (ARGB)
    pub fn draw_border(&self, thickness: i32, color: u32) -> Result<()> {
        let target = match &self.render_target {
            Some(t) => t,
            None => return Ok(()),
        };

        let w = self.bound_width as f32;
        let h = self.bound_height as f32;
        let t = thickness as f32;

        unsafe {
            let brush = self.create_solid_brush(target, color)?;

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

    /// 둥근 사각형 채우기
    pub fn fill_rounded_rect(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: u32,
    ) -> Result<()> {
        let target = match &self.render_target {
            Some(t) => t,
            None => return Ok(()),
        };

        unsafe {
            let brush = self.create_solid_brush(target, color)?;
            let rounded_rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: x,
                    top: y,
                    right: x + w,
                    bottom: y + h,
                },
                radiusX: radius,
                radiusY: radius,
            };
            target.FillRoundedRectangle(&rounded_rect, &brush);
        }

        Ok(())
    }

    /// 둥근 사각형 테두리 그리기
    pub fn draw_rounded_rect(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        stroke_width: f32,
        color: u32,
    ) -> Result<()> {
        let target = match &self.render_target {
            Some(t) => t,
            None => return Ok(()),
        };

        unsafe {
            let brush = self.create_solid_brush(target, color)?;
            let rounded_rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: x,
                    top: y,
                    right: x + w,
                    bottom: y + h,
                },
                radiusX: radius,
                radiusY: radius,
            };
            target.DrawRoundedRectangle(&rounded_rect, &brush, stroke_width, None);
        }

        Ok(())
    }

    /// 텍스트 그리기 (외곽선, 그림자 포함)
    /// 렌더링 순서: 그림자 -> 외곽선2 -> 외곽선1 -> 주 텍스트
    pub fn draw_text(
        &self,
        text: &str,
        x: f32,
        y: f32,
        max_width: f32,
        max_height: f32,
        style: &TextRenderStyle,
    ) -> Result<()> {
        let target = match &self.render_target {
            Some(t) => t,
            None => return Err(Error::from_hresult(HRESULT(-1))),
        };

        unsafe {
            // 텍스트 포맷 생성
            let font_face_wide: Vec<u16> = style
                .font_face
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let font_weight = if style.font_style & 1 != 0 {
                DWRITE_FONT_WEIGHT_BOLD
            } else {
                DWRITE_FONT_WEIGHT_NORMAL
            };
            let font_style_dw = if style.font_style & 2 != 0 {
                DWRITE_FONT_STYLE_ITALIC
            } else {
                DWRITE_FONT_STYLE_NORMAL
            };

            let text_format = self.dwrite_factory.CreateTextFormat(
                PCWSTR(font_face_wide.as_ptr()),
                None,
                font_weight,
                font_style_dw,
                DWRITE_FONT_STRETCH_NORMAL,
                style.font_size as f32,
                w!(""),
            )?;

            // 텍스트 레이아웃 생성
            let text_wide: Vec<u16> = text.encode_utf16().collect();
            let text_layout = self.dwrite_factory.CreateTextLayout(
                &text_wide,
                &text_format,
                max_width,
                max_height,
            )?;

            let outline_total = style.outline1_size + style.outline2_size;

            // 1. 그림자 그리기
            if style.shadow_enabled && (style.shadow_offset_x != 0 || style.shadow_offset_y != 0) {
                let shadow_x = x + style.shadow_offset_x as f32;
                let shadow_y = y + style.shadow_offset_y as f32;

                if outline_total > 0 {
                    self.draw_text_outline(
                        target,
                        &text_layout,
                        shadow_x,
                        shadow_y,
                        outline_total,
                        style.shadow_color,
                    )?;
                }

                let shadow_brush = self.create_solid_brush(target, style.shadow_color)?;
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
                    &text_layout,
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
                    &text_layout,
                    x,
                    y,
                    style.outline1_size,
                    style.outline1_color,
                )?;
            }

            // 4. 주 텍스트 그리기
            let text_brush = self.create_solid_brush(target, style.color)?;
            target.DrawTextLayout(
                Vector2::new(x, y),
                &text_layout,
                &text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        Ok(())
    }

    /// Geometry 기반 텍스트 외곽선 그리기
    /// IDWriteFontFace::GetGlyphRunOutline을 사용하여 정확한 벡터 외곽선 생성
    fn draw_text_outline(
        &self,
        target: &ID2D1DCRenderTarget,
        text_layout: &IDWriteTextLayout,
        x: f32,
        y: f32,
        thickness: i32,
        color: u32,
    ) -> Result<()> {
        unsafe {
            // 1. PathGeometry 생성
            let path_geometry: ID2D1PathGeometry = self.d2d_factory.CreatePathGeometry()?;
            let sink = path_geometry.Open()?;

            // 2. 커스텀 텍스트 렌더러로 글리프 아웃라인 추출
            let renderer: IDWriteTextRenderer = OutlineTextRenderer::new(
                self.d2d_factory.clone(),
                sink.clone(),
            )
            .into();

            text_layout.Draw(None, &renderer, x, y)?;

            sink.Close()?;

            // 3. 외곽선 스타일 설정 (둥근 조인과 캡)
            let stroke_style = self.d2d_factory.CreateStrokeStyle(
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

            // 4. 외곽선 그리기
            let brush = self.create_solid_brush(target, color)?;
            target.DrawGeometry(
                &path_geometry,
                &brush,
                thickness as f32 * 2.0, // stroke는 양쪽으로 그려지므로 2배
                Some(&stroke_style),
            );

            // 5. 내부 채우기 (외곽선 색상으로)
            target.FillGeometry(&path_geometry, &brush, None);
        }

        Ok(())
    }

    /// ARGB 색상으로 SolidColorBrush 생성
    fn create_solid_brush(
        &self,
        target: &ID2D1DCRenderTarget,
        color: u32,
    ) -> Result<ID2D1SolidColorBrush> {
        let a = ((color >> 24) & 0xFF) as f32 / 255.0;
        let r = ((color >> 16) & 0xFF) as f32 / 255.0;
        let g = ((color >> 8) & 0xFF) as f32 / 255.0;
        let b = (color & 0xFF) as f32 / 255.0;

        unsafe {
            target.CreateSolidColorBrush(
                &D2D1_COLOR_F { r, g, b, a },
                Some(&D2D1_BRUSH_PROPERTIES {
                    opacity: 1.0,
                    transform: Matrix3x2::identity(),
                }),
            )
        }
    }

    /// 텍스트 메트릭스 가져오기
    pub fn get_text_metrics(
        &self,
        text: &str,
        style: &TextRenderStyle,
        max_width: f32,
        max_height: f32,
    ) -> Result<(f32, f32)> {
        unsafe {
            let font_face_wide: Vec<u16> = style
                .font_face
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let font_weight = if style.font_style & 1 != 0 {
                DWRITE_FONT_WEIGHT_BOLD
            } else {
                DWRITE_FONT_WEIGHT_NORMAL
            };
            let font_style_dw = if style.font_style & 2 != 0 {
                DWRITE_FONT_STYLE_ITALIC
            } else {
                DWRITE_FONT_STYLE_NORMAL
            };

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
            let text_layout = self.dwrite_factory.CreateTextLayout(
                &text_wide,
                &text_format,
                max_width,
                max_height,
            )?;

            let mut metrics: DWRITE_TEXT_METRICS = std::mem::zeroed();
            text_layout.GetMetrics(&mut metrics)?;

            Ok((metrics.width, metrics.height))
        }
    }

    /// 바인딩된 크기 반환
    pub fn get_size(&self) -> (i32, i32) {
        (self.bound_width, self.bound_height)
    }
}
