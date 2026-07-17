use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Direct2D::*, DirectWrite::*},
    },
    core::*,
};
use windows_numerics::Matrix3x2;

// ── OutlineTextRenderer ─────────────────────────────────

/// IDWriteTextRenderer 구현체 - 글리프를 ID2D1GeometrySink로 출력
#[windows::core::implement(IDWriteTextRenderer, IDWritePixelSnapping)]
pub(super) struct OutlineTextRenderer {
    d2d_factory: ID2D1Factory1,
    sink: ID2D1GeometrySink,
}

impl OutlineTextRenderer {
    pub(super) fn new(d2d_factory: ID2D1Factory1, sink: ID2D1GeometrySink) -> Self {
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
