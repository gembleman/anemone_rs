use std::sync::Arc;

use crate::window::TextRenderStyle;
use windows::Win32::Graphics::{Direct2D::*, DirectWrite::*};

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
pub(super) struct TextLayoutKey {
    pub(super) text: Arc<str>,
    pub(super) font_face: Arc<str>,
    pub(super) font_size: i32,
    pub(super) font_style: u8,
    pub(super) max_width_bits: u32,
    pub(super) max_height_bits: u32,
}

/// `TextLayoutKey` 와 동일한 의미를 갖지만 `&str` 참조로만 구성한 0-alloc
/// 캐시 조회 뷰. paint 핫패스에서 hit 판정용으로 사용한다 — miss 시에만
/// [`Self::to_owned`] 로 실제 키를 alloc.
pub(super) struct LayoutKeyRef<'a> {
    pub(super) text: &'a str,
    pub(super) font_face: &'a str,
    pub(super) font_size: i32,
    pub(super) font_style: u8,
    pub(super) max_width_bits: u32,
    pub(super) max_height_bits: u32,
}

impl<'a> LayoutKeyRef<'a> {
    pub(super) fn from_style(
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
        }
    }

    pub(super) fn matches(&self, key: &TextLayoutKey) -> bool {
        self.font_size == key.font_size
            && self.font_style == key.font_style
            && self.max_width_bits == key.max_width_bits
            && self.max_height_bits == key.max_height_bits
            && self.text == &*key.text
            && self.font_face == &*key.font_face
    }

    pub(super) fn to_owned(&self) -> TextLayoutKey {
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
pub(super) struct TextLayoutCache {
    pub(super) key: TextLayoutKey,
    pub(super) layout: IDWriteTextLayout,
    pub(super) outline: Option<ID2D1PathGeometry>,
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
pub(super) struct OutlineBitmapKey {
    pub(super) text: Arc<str>,
    pub(super) font_face: Arc<str>,
    pub(super) font_size: i32,
    pub(super) font_style: u8,
    pub(super) max_width_bits: u32,
    pub(super) max_height_bits: u32,
    pub(super) outline1_size: i32,
    pub(super) outline1_color: u32,
    pub(super) outline2_size: i32,
    pub(super) outline2_color: u32,
    pub(super) shadow_enabled: bool,
    pub(super) shadow_color: u32,
    pub(super) shadow_offset_x: i32,
    pub(super) shadow_offset_y: i32,
}

/// `OutlineBitmapKey` 의 0-alloc 조회 뷰. `LayoutKeyRef` 와 같은 패턴.
pub(super) struct OutlineBitmapKeyRef<'a> {
    pub(super) text: &'a str,
    pub(super) font_face: &'a str,
    pub(super) font_size: i32,
    pub(super) font_style: u8,
    pub(super) max_width_bits: u32,
    pub(super) max_height_bits: u32,
    pub(super) outline1_size: i32,
    pub(super) outline1_color: u32,
    pub(super) outline2_size: i32,
    pub(super) outline2_color: u32,
    pub(super) shadow_enabled: bool,
    pub(super) shadow_color: u32,
    pub(super) shadow_offset_x: i32,
    pub(super) shadow_offset_y: i32,
}

impl<'a> OutlineBitmapKeyRef<'a> {
    pub(super) fn from_style(
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

    pub(super) fn matches(&self, key: &OutlineBitmapKey) -> bool {
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

    pub(super) fn to_owned(&self) -> OutlineBitmapKey {
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
pub(super) struct OutlineBitmap {
    pub(super) key: OutlineBitmapKey,
    pub(super) bitmap: ID2D1Bitmap,
    /// 비트맵 가로/세로 크기 (DIP). DrawBitmap 의 dest 사각형 크기.
    pub(super) width: f32,
    pub(super) height: f32,
    /// 비트맵 안에서 layout 원점이 떨어지는 위치 (DIP).
    pub(super) pad_left: f32,
    pub(super) pad_top: f32,
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
pub(super) struct MissTracker {
    /// 최근 paint 결과 비트마스크. bit 0 = 가장 오래된, bit (WINDOW-1) =
    /// 가장 최근. 1 = miss, 0 = hit. ring buffer 를 u16 한 워드로 압축.
    pub(super) ring: u16,
    /// ring 에 쌓인 샘플 수 (0..=WINDOW). WINDOW 도달 후로는 계속 WINDOW.
    pub(super) filled: u8,
    /// 폭주 판정 임계치 — `ANEMONE_MISS_THRESHOLD` 환경변수로 측정 시 오버라이드 가능.
    /// 기본값 `DEFAULT_THRESHOLD`. WINDOW 는 ring buffer 폭이 16 비트로 고정돼
    /// 있어 const 유지 (재컴파일 없이 폭을 바꿀 수 없음).
    pub(super) threshold: u8,
    /// 직전 paint 의 outline 비트맵 캐시 키. 폭주 모드에서 비트맵을
    /// 만들지 않으면서도 "텍스트가 안정화됐는지" 를 판정하기 위해 별도
    /// 보관. 정상 모드에서도 같은 키로 기록되므로 일관성이 유지된다.
    pub(super) last_key: Option<OutlineBitmapKey>,
}

impl MissTracker {
    const WINDOW: u8 = 8;
    const DEFAULT_THRESHOLD: u8 = 5;
    /// 하위 WINDOW 비트만 남기는 마스크. record() 의 hot loop 에서 매번
    /// 재계산하지 않도록 const 로 추출 — WINDOW=8 → 0x00FF.
    const RING_MASK: u16 = (1u16 << Self::WINDOW) - 1;

    pub(super) fn new() -> Self {
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
    pub(super) fn resolve_threshold() -> u8 {
        std::env::var("ANEMONE_MISS_THRESHOLD")
            .ok()
            .and_then(|s| s.trim().parse::<u8>().ok())
            .filter(|&v| (1..=Self::WINDOW).contains(&v))
            .unwrap_or(Self::DEFAULT_THRESHOLD)
    }

    /// paint 1 회 결과를 기록. `miss=true` 면 outline 비트맵 캐시 miss.
    pub(super) fn record(&mut self, miss: bool) {
        // 새 비트를 최상위에 추가하고 1 비트 shift — bit (WINDOW-1) 이
        // 최신, bit 0 이 가장 오래된 것.
        self.ring = ((self.ring << 1) & Self::RING_MASK) | (miss as u16);
        if self.filled < Self::WINDOW {
            self.filled += 1;
        }
    }

    /// 폭주 모드 판정. ring 이 WINDOW 만큼 차고 miss 가 threshold 이상.
    /// WINDOW 미만이면 false (warmup 동안은 정상 경로 유지).
    pub(super) fn is_overloaded(&self) -> bool {
        self.filled >= Self::WINDOW && self.ring.count_ones() as u8 >= self.threshold
    }
}
