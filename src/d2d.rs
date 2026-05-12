//! Direct2D 기반 렌더러
//!
//! 그리기 메서드들은 모두 `&ID2D1RenderTarget` 을 받도록 일반화돼
//! `CompositionRenderer` (DComp 합성 경로) 의 `ID2D1DeviceContext` 와
//! 미래에 추가될 다른 render target 모두에서 그대로 사용된다.

use std::collections::HashMap;
use std::sync::Arc;

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
///
/// `text` / `font_face` 는 `Arc<str>` — 캐시에 저장된 키와 새 paint 의
/// `&str` 비교는 `LayoutKeyRef::matches` 가 `Arc::deref` 로 0-alloc 수행한다.
/// 새 캐시 엔트리 생성 시에만 `Arc::from(&str)` 1 회 alloc 발생.
#[derive(Clone, Eq, PartialEq, Hash)]
struct TextLayoutKey {
    text: Arc<str>,
    font_face: Arc<str>,
    font_size: i32,
    font_style: u8,
    max_width_bits: u32,
    max_height_bits: u32,
}

/// `TextLayoutKey` 와 동일한 의미를 갖지만 `&str` 참조로만 구성한 0-alloc
/// 캐시 조회 뷰. paint 핫패스에서 hit 판정용으로 사용한다 — miss 시에만
/// [`Self::to_owned`] 로 실제 키를 alloc.
struct LayoutKeyRef<'a> {
    text: &'a str,
    font_face: &'a str,
    font_size: i32,
    font_style: u8,
    max_width_bits: u32,
    max_height_bits: u32,
}

impl<'a> LayoutKeyRef<'a> {
    fn from_style(text: &'a str, style: &'a TextRenderStyle, max_width: f32, max_height: f32) -> Self {
        Self {
            text,
            font_face: &style.font_face,
            font_size: style.font_size,
            font_style: style.font_style,
            max_width_bits: max_width.to_bits(),
            max_height_bits: max_height.to_bits(),
        }
    }

    fn matches(&self, key: &TextLayoutKey) -> bool {
        self.font_size == key.font_size
            && self.font_style == key.font_style
            && self.max_width_bits == key.max_width_bits
            && self.max_height_bits == key.max_height_bits
            && self.text == &*key.text
            && self.font_face == &*key.font_face
    }

    fn to_owned(&self) -> TextLayoutKey {
        TextLayoutKey {
            text: Arc::from(self.text),
            font_face: Arc::from(self.font_face),
            font_size: self.font_size,
            font_style: self.font_style,
            max_width_bits: self.max_width_bits,
            max_height_bits: self.max_height_bits,
        }
    }
}

/// 캐시 엔트리. outline geometry 는 첫 외곽선 그리기 호출 시 lazy 생성.
struct TextLayoutCache {
    key: TextLayoutKey,
    layout: IDWriteTextLayout,
    outline: Option<ID2D1PathGeometry>,
}

/// outline+shadow 비트맵 캐시 키.
///
/// `TextLayoutKey` 와 달리 색상/두께/그림자 오프셋까지 모두 포함한다 —
/// 비트맵엔 "칠해진 픽셀" 이 들어가므로 색/두께가 바뀌면 비트맵을 다시
/// 그려야 한다.
///
/// `text` / `font_face` 는 `Arc<str>` — `TextLayoutKey` 와 같은 정책.
/// 핫패스 hit 판정은 [`OutlineBitmapKeyRef::matches`] 가 alloc 없이 처리.
#[derive(Clone, Eq, PartialEq, Hash)]
struct OutlineBitmapKey {
    text: Arc<str>,
    font_face: Arc<str>,
    font_size: i32,
    font_style: u8,
    max_width_bits: u32,
    max_height_bits: u32,
    outline1_size: i32,
    outline1_color: u32,
    outline2_size: i32,
    outline2_color: u32,
    shadow_enabled: bool,
    shadow_color: u32,
    shadow_offset_x: i32,
    shadow_offset_y: i32,
}

/// `OutlineBitmapKey` 의 0-alloc 조회 뷰. `LayoutKeyRef` 와 같은 패턴.
struct OutlineBitmapKeyRef<'a> {
    text: &'a str,
    font_face: &'a str,
    font_size: i32,
    font_style: u8,
    max_width_bits: u32,
    max_height_bits: u32,
    outline1_size: i32,
    outline1_color: u32,
    outline2_size: i32,
    outline2_color: u32,
    shadow_enabled: bool,
    shadow_color: u32,
    shadow_offset_x: i32,
    shadow_offset_y: i32,
}

impl<'a> OutlineBitmapKeyRef<'a> {
    fn from_style(
        text: &'a str,
        style: &'a TextRenderStyle,
        max_width: f32,
        max_height: f32,
    ) -> Self {
        Self {
            text,
            font_face: &style.font_face,
            font_size: style.font_size,
            font_style: style.font_style,
            max_width_bits: max_width.to_bits(),
            max_height_bits: max_height.to_bits(),
            outline1_size: style.outline1_size,
            outline1_color: style.outline1_color,
            outline2_size: style.outline2_size,
            outline2_color: style.outline2_color,
            shadow_enabled: style.shadow_enabled,
            shadow_color: style.shadow_color,
            shadow_offset_x: style.shadow_offset_x,
            shadow_offset_y: style.shadow_offset_y,
        }
    }

    fn matches(&self, key: &OutlineBitmapKey) -> bool {
        self.font_size == key.font_size
            && self.font_style == key.font_style
            && self.max_width_bits == key.max_width_bits
            && self.max_height_bits == key.max_height_bits
            && self.outline1_size == key.outline1_size
            && self.outline1_color == key.outline1_color
            && self.outline2_size == key.outline2_size
            && self.outline2_color == key.outline2_color
            && self.shadow_enabled == key.shadow_enabled
            && self.shadow_color == key.shadow_color
            && self.shadow_offset_x == key.shadow_offset_x
            && self.shadow_offset_y == key.shadow_offset_y
            && self.text == &*key.text
            && self.font_face == &*key.font_face
    }

    fn to_owned(&self) -> OutlineBitmapKey {
        OutlineBitmapKey {
            text: Arc::from(self.text),
            font_face: Arc::from(self.font_face),
            font_size: self.font_size,
            font_style: self.font_style,
            max_width_bits: self.max_width_bits,
            max_height_bits: self.max_height_bits,
            outline1_size: self.outline1_size,
            outline1_color: self.outline1_color,
            outline2_size: self.outline2_size,
            outline2_color: self.outline2_color,
            shadow_enabled: self.shadow_enabled,
            shadow_color: self.shadow_color,
            shadow_offset_x: self.shadow_offset_x,
            shadow_offset_y: self.shadow_offset_y,
        }
    }
}

/// outline+shadow 결과를 담은 중간 비트맵.
///
/// 비트맵 좌표계는 layout 원점이 `(pad_left, pad_top)` 에 떨어지도록
/// 패딩을 둔다. 그래야 음수 좌표로 나가는 그림자/외곽선이 비트맵 안의
/// 양수 영역에 들어간다. 그리기 측은 `DrawBitmap` 의 `dest` 를
/// `(x - pad_left, y - pad_top, x - pad_left + w, y - pad_top + h)` 로
/// 잡아 layout 원점이 정확히 `(x, y)` 에 떨어지도록 한다.
struct OutlineBitmap {
    key: OutlineBitmapKey,
    bitmap: ID2D1Bitmap,
    /// 비트맵 가로/세로 크기 (DIP). DrawBitmap 의 dest 사각형 크기.
    width: f32,
    height: f32,
    /// 비트맵 안에서 layout 원점이 떨어지는 위치 (DIP).
    pad_left: f32,
    pad_top: f32,
}

/// outline 비트맵 캐시 miss 비율을 추적해 "폭주 모드" 를 감지한다.
///
/// 항목 10 의 알려진 한계 — 텍스트가 ms 단위로 폭주 변경되면 매 paint
/// 가 캐시 miss 가 되어 비트맵 빌드 비용 (~1.5 ms) 이 paint 핫패스로
/// 흘러들어 baseline (~1 ms) 보다 느려지는 문제 — 의 안전망.
///
/// 동작: 직전 [`Self::WINDOW`] 회 paint 의 hit/miss 를 ring buffer 로
/// 보관. miss 가 [`Self::THRESHOLD`] 이상이면 "폭주" 로 판정하고,
/// 그 동안은 [`D2DRenderer::draw_text`] 가 비트맵 빌드를 건너뛴 채
/// outline geometry stroke+fill 을 paint 안에서 직접 수행한다. 폭주
/// 상황의 paint 1 회 GPU 명령은 9 개로 baseline 과 동일 수준이지만
/// "비트맵 RT 생성+해제+합성" 비용 (실측 ~1.4 ms) 을 회피해 baseline
/// (~1 ms) 수준으로 회복된다.
///
/// 자동 복귀: 폭주 모드 중에도 직전 key (`last_key`) 와 일치하는 paint
/// 는 "hit" 로 ring 에 기록 — 텍스트가 안정화되면 ring 이 hit 으로
/// 채워져 [`Self::THRESHOLD`] 아래로 떨어진다. 그 다음 paint 부터
/// 정상 경로로 복귀해 `ensure_outline_bitmap` 이 새 비트맵을 빌드한다
/// (이 1 회는 miss 로 기록되지만 곧 hit 으로 안정).
///
/// 임계치 산정 근거: 측정 (`ANEMONE_BENCH_PAINT_CACHE_MISS=1`) 에서
/// 매 paint miss 시 paint p50 244 → 1735 µs (7× 악화) 확인. 직접 경로
/// 의 paint 1 회 비용은 ~1 ms 이고 비트맵 경로 hit 은 ~244 µs 이므로
/// **miss 1 회의 추가 비용 (~1.5 ms) ≥ 향후 hit 5 회의 절감 (~5 ×
/// 800 µs ≈ 4 ms)** 의 손익 분기점이 대략 miss-rate 30 % 부근. 안전
/// 마진을 두고 5/8 = 62.5 % 를 임계로 잡음 — 자동 번역 폭주 같은
/// 명확한 패턴만 폭주 모드로 진입시킨다.
struct MissTracker {
    /// 최근 paint 결과 비트마스크. bit 0 = 가장 오래된, bit (WINDOW-1) =
    /// 가장 최근. 1 = miss, 0 = hit. ring buffer 를 u16 한 워드로 압축.
    ring: u16,
    /// ring 에 쌓인 샘플 수 (0..=WINDOW). WINDOW 도달 후로는 계속 WINDOW.
    filled: u8,
    /// 폭주 판정 임계치 — `ANEMONE_MISS_THRESHOLD` 환경변수로 측정 시 오버라이드 가능.
    /// 기본값 `DEFAULT_THRESHOLD`. WINDOW 는 ring buffer 폭이 16 비트로 고정돼
    /// 있어 const 유지 (재컴파일 없이 폭을 바꿀 수 없음).
    threshold: u8,
    /// 직전 paint 의 outline 비트맵 캐시 키. 폭주 모드에서 비트맵을
    /// 만들지 않으면서도 "텍스트가 안정화됐는지" 를 판정하기 위해 별도
    /// 보관. 정상 모드에서도 같은 키로 기록되므로 일관성이 유지된다.
    last_key: Option<OutlineBitmapKey>,
}

impl MissTracker {
    const WINDOW: u8 = 8;
    const DEFAULT_THRESHOLD: u8 = 5;
    /// 하위 WINDOW 비트만 남기는 마스크. record() 의 hot loop 에서 매번
    /// 재계산하지 않도록 const 로 추출 — WINDOW=8 → 0x00FF.
    const RING_MASK: u16 = (1u16 << Self::WINDOW) - 1;

    fn new() -> Self {
        Self {
            ring: 0,
            filled: 0,
            threshold: Self::resolve_threshold(),
            last_key: None,
        }
    }

    /// `ANEMONE_MISS_THRESHOLD` 환경변수 파싱. 유효 범위는 1..=WINDOW.
    /// 미설정/파싱 실패/범위 밖이면 `DEFAULT_THRESHOLD`. 측정 (`ANEMONE_
    /// BENCH_PAINT_CACHE_MISS=1`) 과 짝지어 폭주 모드 진입 임계를 조절할 때
    /// 사용.
    fn resolve_threshold() -> u8 {
        std::env::var("ANEMONE_MISS_THRESHOLD")
            .ok()
            .and_then(|s| s.trim().parse::<u8>().ok())
            .filter(|&v| (1..=Self::WINDOW).contains(&v))
            .unwrap_or(Self::DEFAULT_THRESHOLD)
    }

    /// paint 1 회 결과를 기록. `miss=true` 면 outline 비트맵 캐시 miss.
    fn record(&mut self, miss: bool) {
        // 새 비트를 최상위에 추가하고 1 비트 shift — bit (WINDOW-1) 이
        // 최신, bit 0 이 가장 오래된 것.
        self.ring = ((self.ring << 1) & Self::RING_MASK) | (miss as u16);
        if self.filled < Self::WINDOW {
            self.filled += 1;
        }
    }

    /// 폭주 모드 판정. ring 이 WINDOW 만큼 차고 miss 가 threshold 이상.
    /// WINDOW 미만이면 false (warmup 동안은 정상 경로 유지).
    fn is_overloaded(&self) -> bool {
        self.filled >= Self::WINDOW && self.ring.count_ones() as u8 >= self.threshold
    }
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
    /// outline+shadow 비트맵 캐시 (크기 1 LRU).
    ///
    /// paint 1 회의 GPU 명령을 9 개 (Clear + 3×DrawGeometry +
    /// 3×FillGeometry + 2×DrawTextLayout) 에서 3 개
    /// (Clear + DrawBitmap + DrawTextLayout) 로 줄인다. paint floor 의 ~95% 를
    /// 차지하던 outline stroke+fill 비용 제거가 핵심.
    outline_bitmap: Option<OutlineBitmap>,
    /// 캐시 miss 비율 추적 — 폭주 시 비트맵 경로 우회.
    miss_tracker: MissTracker,
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

    /// 캐시 hit 면 기존 layout, miss 면 새로 만들어 캐시 교체 후 반환.
    /// 키가 바뀌면 outline geometry 도 같이 무효화된다.
    ///
    /// hit 판정은 [`LayoutKeyRef`] 로 alloc 없이 처리 — `Arc::from(&str)` 은
    /// miss 시 새 캐시 엔트리 생성 시점에만 발생한다.
    fn get_or_create_layout(
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
        x: f32,
        y: f32,
        max_width: f32,
        max_height: f32,
        style: &TextRenderStyle,
    ) -> Result<()> {
        let outline_total = style.outline1_size + style.outline2_size;
        let has_shadow =
            style.shadow_enabled && (style.shadow_offset_x != 0 || style.shadow_offset_y != 0);
        let has_outline = outline_total > 0;

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
        // 폭주 모드 진입 시 `outline_bitmap` 을 명시적으로 `take()` — stale 한
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
            return self.draw_text_direct(target, text, x, y, max_width, max_height, style);
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
    #[allow(clippy::too_many_arguments)]
    fn draw_text_direct(
        &mut self,
        target: &ID2D1RenderTarget,
        text: &str,
        x: f32,
        y: f32,
        max_width: f32,
        max_height: f32,
        style: &TextRenderStyle,
    ) -> Result<()> {
        let outline_total = (style.outline1_size + style.outline2_size).max(0);
        let has_shadow =
            style.shadow_enabled && (style.shadow_offset_x != 0 || style.shadow_offset_y != 0);

        // outline geometry + layout 확보 (캐시 hit/miss 처리 포함).
        let _ = self.get_or_create_outline_geometry(text, style, max_width, max_height)?;
        let text_layout = self.get_or_create_layout(text, style, max_width, max_height)?;

        // SAFETY: target 은 caller (paint) 의 BeginDraw 안의 유효 RT.
        // SetTransform 은 각 블록 끝에서 identity 로 복구.
        unsafe {
            // 1. 그림자 (outline + 본문 모두 shadow_color 로).
            if has_shadow {
                let sx = x + style.shadow_offset_x as f32;
                let sy = y + style.shadow_offset_y as f32;
                let shadow_brush = self.get_or_create_brush(target, style.shadow_color)?;
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
            if style.outline2_size > 0 && outline_total > 0 {
                let brush = self.get_or_create_brush(target, style.outline2_color)?;
                self.stroke_fill_outline_at(target, x, y, outline_total, &brush);
            }

            // 3. 외곽선1 (OutlineIn) — outline1_size 두께.
            if style.outline1_size > 0 {
                let brush = self.get_or_create_brush(target, style.outline1_color)?;
                self.stroke_fill_outline_at(target, x, y, style.outline1_size, &brush);
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
        let outline_total = (style.outline1_size + style.outline2_size).max(0) as f32;
        let (shadow_dx, shadow_dy) =
            if style.shadow_enabled && (style.shadow_offset_x != 0 || style.shadow_offset_y != 0) {
                (style.shadow_offset_x as f32, style.shadow_offset_y as f32)
            } else {
                (0.0, 0.0)
            };
        let pad_left = outline_total + (-shadow_dx).max(0.0);
        let pad_top = outline_total + (-shadow_dy).max(0.0);
        let pad_right = outline_total + shadow_dx.max(0.0);
        let pad_bot = outline_total + shadow_dy.max(0.0);

        // 실제 텍스트 폭/높이 (layout box 가 아니라 글리프 점유 영역).
        // widthIncludingTrailingWhitespace 는 마지막 공백까지 포함한 layout
        // 폭으로, 캐시 hit 시 그리기 위치 일관성에 도움.
        let layout = self.get_or_create_layout(text, style, max_width, max_height)?;
        let mut tm = DWRITE_TEXT_METRICS::default();
        // SAFETY: layout 은 위에서 막 확보. tm 은 out 파라미터.
        unsafe { layout.GetMetrics(&mut tm)?; }
        let text_w = tm.widthIncludingTrailingWhitespace.max(0.0);
        let text_h = tm.height.max(0.0);

        // 비트맵 크기 — 최소 1×1 보장 (D2D 가 0 사이즈 거부).
        let bm_w = (pad_left + text_w + pad_right).ceil().max(1.0);
        let bm_h = (pad_top + text_h + pad_bot).ceil().max(1.0);

        // SAFETY: target 은 caller 의 BeginDraw 안의 유효 render target.
        // CreateCompatibleRenderTarget 은 그 target 의 디바이스 위에 새 RT 를
        // 만들고, 같은 픽셀 포맷 + premultiplied alpha 를 자동 적용한다.
        unsafe {
            let size = D2D_SIZE_F { width: bm_w, height: bm_h };
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
            if style.shadow_enabled
                && (style.shadow_offset_x != 0 || style.shadow_offset_y != 0)
            {
                let sx = origin_x + shadow_dx;
                let sy = origin_y + shadow_dy;
                let shadow_brush = inner_rt.CreateSolidColorBrush(
                    &argb_to_color_f(style.shadow_color),
                    None,
                )?;

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
            if style.outline2_size > 0 && outline_total > 0.0 {
                let brush = inner_rt.CreateSolidColorBrush(
                    &argb_to_color_f(style.outline2_color),
                    None,
                )?;
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
            if style.outline1_size > 0 {
                let brush = inner_rt.CreateSolidColorBrush(
                    &argb_to_color_f(style.outline1_color),
                    None,
                )?;
                self.draw_outline_only(
                    inner_rt,
                    text,
                    style,
                    max_width,
                    max_height,
                    origin_x,
                    origin_y,
                    style.outline1_size,
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
                RECT { left, top, right, bottom }
            })
            .collect();
        Ok(rects)
    }
}
