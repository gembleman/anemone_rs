//! DirectWrite 기반 텍스트 렌더러
//!
//! GDI TextOutW 8방향 반복 외곽선을 대체하여
//! DirectWrite + Direct2D DC 렌더 타겟을 사용한 텍스트 렌더링

use std::mem::zeroed;

use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Gdi::HDC,
        },
    },
};
use windows_numerics::{Matrix3x2, Vector2};

use crate::window::TextRenderStyle;

/// DirectWrite 기반 텍스트 렌더러
pub struct DirectWriteRenderer {
    d2d_factory: ID2D1Factory,
    dwrite_factory: IDWriteFactory,
    render_target: Option<ID2D1DCRenderTarget>,
}

impl DirectWriteRenderer {
    /// DirectWriteRenderer 생성
    pub fn new() -> Result<Self> {
        unsafe {
            // D2D Factory 생성
            let d2d_factory: ID2D1Factory = D2D1CreateFactory(
                D2D1_FACTORY_TYPE_SINGLE_THREADED,
                None,
            )?;

            // DirectWrite Factory 생성
            let dwrite_factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            Ok(Self {
                d2d_factory,
                dwrite_factory,
                render_target: None,
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

            Ok(())
        }
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
            let font_face_wide: Vec<u16> = style.font_face.encode_utf16().chain(std::iter::once(0)).collect();
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

            target.BeginDraw();

            // 안티앨리어싱 설정 (투명 배경에서는 Grayscale AA 사용)
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);

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

            target.EndDraw(None, None)?;
        }

        Ok(())
    }

    /// 다중 DrawTextLayout으로 외곽선 근사 (8방향 + 추가 픽셀)
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
            let brush = self.create_solid_brush(target, color)?;

            // 외곽선 두께만큼 모든 방향으로 텍스트 그리기
            for dy in -thickness..=thickness {
                for dx in -thickness..=thickness {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    // 유클리드 거리로 원형 외곽선 근사
                    let dist_sq = dx * dx + dy * dy;
                    if dist_sq <= thickness * thickness {
                        target.DrawTextLayout(
                            Vector2::new(x + dx as f32, y + dy as f32),
                            text_layout,
                            &brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// ARGB 색상으로 SolidColorBrush 생성
    fn create_solid_brush(&self, target: &ID2D1DCRenderTarget, color: u32) -> Result<ID2D1SolidColorBrush> {
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
            let font_face_wide: Vec<u16> = style.font_face.encode_utf16().chain(std::iter::once(0)).collect();
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

            let mut metrics: DWRITE_TEXT_METRICS = zeroed();
            text_layout.GetMetrics(&mut metrics)?;

            Ok((metrics.width, metrics.height))
        }
    }
}
