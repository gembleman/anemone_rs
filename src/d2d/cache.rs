use std::sync::Arc;

use crate::config::TextAlign;

use super::MeasureSlot;
use super::style::TextRenderStyle;
use windows::Win32::{
    Foundation::RECT,
    Graphics::{Direct2D::*, DirectWrite::*},
};

/// Layout 형태를 결정하는 값만 담은 캐시 키. 부동소수점은 bit 단위로 비교한다.
#[derive(Clone, Eq, PartialEq, Hash)]
pub(super) struct TextLayoutKey {
    pub(super) text: Arc<str>,
    pub(super) font_face: Arc<str>,
    pub(super) font_size: i32,
    pub(super) font_style: u8,
    pub(super) text_align: TextAlign,
    pub(super) max_width_bits: u32,
    pub(super) max_height_bits: u32,
}

/// Hit 조회에는 문자열을 빌리지 않고 miss일 때만 소유 키로 바꾸는 뷰.
pub(super) struct LayoutKeyRef<'a> {
    pub(super) text: &'a str,
    pub(super) font_face: &'a str,
    pub(super) font_size: i32,
    pub(super) font_style: u8,
    pub(super) text_align: TextAlign,
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
            text_align: style.text_align,
            max_width_bits: max_width.to_bits(),
            max_height_bits: max_height.to_bits(),
        }
    }

    pub(super) fn matches(&self, key: &TextLayoutKey) -> bool {
        self.font_size == key.font_size
            && self.font_style == key.font_style
            && self.text_align == key.text_align
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
            text_align: self.text_align,
            max_width_bits: self.max_width_bits,
            max_height_bits: self.max_height_bits,
        }
    }
}

/// 측정 결과 캐시 키 — 레이아웃 높이에 영향을 주는 값만 담는다.
///
/// measure는 높이를 계산하는 것이 목적이므로 `max_height`는 키에 넣지 않는다.
/// 호출측에서 항상 같은 상한(`MEASURE_MAX_HEIGHT`)을 쓰기 때문에 변별력이 없다.
/// 색상/외곽선/그림자는 layout metrics에 영향을 주지 않으므로 제외한다.
#[derive(Clone, Eq, PartialEq, Hash)]
pub(super) struct MeasureKey {
    pub(super) text: Arc<str>,
    pub(super) font_face: Arc<str>,
    pub(super) font_size: i32,
    pub(super) font_style: u8,
    pub(super) text_align: TextAlign,
    pub(super) max_width_bits: u32,
}

/// 문자열을 빌리는 조회용 뷰.
pub(super) struct MeasureKeyRef<'a> {
    pub(super) text: &'a str,
    pub(super) font_face: &'a str,
    pub(super) font_size: i32,
    pub(super) font_style: u8,
    pub(super) text_align: TextAlign,
    pub(super) max_width_bits: u32,
}

impl<'a> MeasureKeyRef<'a> {
    pub(super) fn from_style(
        text: &'a str,
        style: &'a TextRenderStyle,
        max_width: f32,
    ) -> Self {
        Self {
            text,
            font_face: &style.font_face,
            font_size: style.font_size,
            font_style: style.font_style,
            text_align: style.text_align,
            max_width_bits: max_width.to_bits(),
        }
    }

    pub(super) fn matches(&self, key: &MeasureKey) -> bool {
        self.font_size == key.font_size
            && self.font_style == key.font_style
            && self.text_align == key.text_align
            && self.max_width_bits == key.max_width_bits
            && self.text == &*key.text
            && self.font_face == &*key.font_face
    }

    pub(super) fn to_owned(&self) -> MeasureKey {
        MeasureKey {
            text: Arc::from(self.text),
            font_face: Arc::from(self.font_face),
            font_size: self.font_size,
            font_style: self.font_style,
            text_align: self.text_align,
            max_width_bits: self.max_width_bits,
        }
    }
}

/// `measure_text_height` 결과 항목 하나.
pub(super) struct MeasureCacheEntry {
    pub(super) key: MeasureKey,
    pub(super) height: f32,
}

/// `measure_text_height` 결과 캐시 — 텍스트 유형별 direct-mapped 단일 엔트리.
///
/// paint는 유형별로 최대 1블록만 측정하므로(이름·원문·번역·notice), 슬롯을
/// 유형과 1:1로 두면 퇴거 정책이 필요 없다. "이름 고정 + 대사만 변경" 패턴에서
/// 고정 블록의 hit가 유지된다. 크기가 고정이라 무한 성장하지 않는다.
pub(super) struct MeasureCache {
    entries: [Option<MeasureCacheEntry>; MeasureSlot::COUNT],
}

impl MeasureCache {
    pub(super) fn new() -> Self {
        Self {
            entries: [const { None }; MeasureSlot::COUNT],
        }
    }

    pub(super) fn get(&self, slot: MeasureSlot, key: &MeasureKeyRef<'_>) -> Option<f32> {
        self.entries[slot as usize]
            .as_ref()
            .filter(|entry| key.matches(&entry.key))
            .map(|entry| entry.height)
    }

    /// miss 시 호출: 해당 슬롯을 덮어쓴다. 다른 슬롯은 건드리지 않는다.
    pub(super) fn insert(&mut self, slot: MeasureSlot, key: MeasureKey, height: f32) {
        self.entries[slot as usize] = Some(MeasureCacheEntry { key, height });
    }
}

/// Outline geometry를 필요할 때 만드는 layout 캐시 항목.
pub(super) struct TextLayoutCache {
    pub(super) key: TextLayoutKey,
    pub(super) layout: IDWriteTextLayout,
    pub(super) outline: Option<ID2D1PathGeometry>,
}

/// 색상, 두께, 그림자까지 포함한 outline/shadow bitmap 캐시 키.
#[derive(Clone, Eq, PartialEq, Hash)]
pub(super) struct OutlineBitmapKey {
    pub(super) text: Arc<str>,
    pub(super) font_face: Arc<str>,
    pub(super) font_size: i32,
    pub(super) font_style: u8,
    pub(super) text_align: TextAlign,
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

/// 문자열 할당 없이 bitmap 캐시를 조회하는 뷰.
pub(super) struct OutlineBitmapKeyRef<'a> {
    pub(super) text: &'a str,
    pub(super) font_face: &'a str,
    pub(super) font_size: i32,
    pub(super) font_style: u8,
    pub(super) text_align: TextAlign,
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
        let effects = EffectiveOutlineStyle::from_style(style);
        Self {
            text,
            font_face: &style.font_face,
            font_size: style.font_size,
            font_style: style.font_style,
            text_align: style.text_align,
            max_width_bits: max_width.to_bits(),
            max_height_bits: max_height.to_bits(),
            outline1_size: effects.outline1_size,
            outline1_color: effects.outline1_color,
            outline2_size: effects.outline2_size,
            outline2_color: effects.outline2_color,
            shadow_enabled: effects.has_shadow,
            shadow_color: effects.shadow_color,
            shadow_offset_x: effects.shadow_offset_x,
            shadow_offset_y: effects.shadow_offset_y,
        }
    }

    pub(super) fn matches(&self, key: &OutlineBitmapKey) -> bool {
        self.font_size == key.font_size
            && self.font_style == key.font_style
            && self.text_align == key.text_align
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

    #[cfg(test)]
    pub(super) fn to_owned(&self) -> OutlineBitmapKey {
        OutlineBitmapKey {
            text: Arc::from(self.text),
            font_face: Arc::from(self.font_face),
            font_size: self.font_size,
            font_style: self.font_style,
            text_align: self.text_align,
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

    /// 이미 생성된 layout key의 문자열 소유권을 공유해 중복 할당을 피한다.
    pub(super) fn to_owned_reusing_layout(&self, layout_key: &TextLayoutKey) -> OutlineBitmapKey {
        debug_assert!(
            LayoutKeyRef {
                text: self.text,
                font_face: self.font_face,
                font_size: self.font_size,
                font_style: self.font_style,
                text_align: self.text_align,
                max_width_bits: self.max_width_bits,
                max_height_bits: self.max_height_bits,
            }
            .matches(layout_key)
        );
        OutlineBitmapKey {
            text: Arc::clone(&layout_key.text),
            font_face: Arc::clone(&layout_key.font_face),
            font_size: self.font_size,
            font_style: self.font_style,
            text_align: self.text_align,
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

/// 줄별 hit-test 결과의 layout 및 위치 입력.
pub(super) struct HitTestKey {
    pub(super) layout: TextLayoutKey,
    origin_x_bits: u32,
    origin_y_bits: u32,
    inflate_bits: u32,
}

pub(super) struct HitTestKeyRef<'a> {
    layout: LayoutKeyRef<'a>,
    origin_x_bits: u32,
    origin_y_bits: u32,
    inflate_bits: u32,
}

impl<'a> HitTestKeyRef<'a> {
    pub(super) fn from_style(
        text: &'a str,
        style: &'a TextRenderStyle,
        origin_x: f32,
        origin_y: f32,
        max_width: f32,
        max_height: f32,
        inflate: f32,
    ) -> Self {
        Self {
            layout: LayoutKeyRef::from_style(text, style, max_width, max_height),
            origin_x_bits: origin_x.to_bits(),
            origin_y_bits: origin_y.to_bits(),
            inflate_bits: inflate.to_bits(),
        }
    }

    pub(super) fn matches(&self, key: &HitTestKey) -> bool {
        self.layout.matches(&key.layout)
            && self.origin_x_bits == key.origin_x_bits
            && self.origin_y_bits == key.origin_y_bits
            && self.inflate_bits == key.inflate_bits
    }

    pub(super) fn to_owned_reusing_layout(&self, layout: &TextLayoutKey) -> HitTestKey {
        debug_assert!(self.layout.matches(layout));
        HitTestKey {
            layout: layout.clone(),
            origin_x_bits: self.origin_x_bits,
            origin_y_bits: self.origin_y_bits,
            inflate_bits: self.inflate_bits,
        }
    }
}

pub(super) struct HitTestCache {
    pub(super) key: HitTestKey,
    pub(super) rects: Vec<RECT>,
}

/// 실제 픽셀에 영향을 주는 outline/shadow 값만 남긴 정규형.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EffectiveOutlineStyle {
    pub(super) outline1_size: i32,
    pub(super) outline1_color: u32,
    pub(super) outline2_size: i32,
    pub(super) outline2_color: u32,
    pub(super) outline_total: i32,
    pub(super) has_shadow: bool,
    pub(super) shadow_color: u32,
    pub(super) shadow_offset_x: i32,
    pub(super) shadow_offset_y: i32,
}

impl EffectiveOutlineStyle {
    pub(super) fn from_style(style: &TextRenderStyle) -> Self {
        let outline1_size = style.outline1_size.max(0);
        let outline2_size = style.outline2_size.max(0);
        let outline_total = outline1_size.saturating_add(outline2_size);
        let has_shadow =
            style.shadow_enabled && (style.shadow_offset_x != 0 || style.shadow_offset_y != 0);
        Self {
            outline1_size,
            outline1_color: if outline1_size > 0 {
                style.outline1_color
            } else {
                0
            },
            outline2_size,
            outline2_color: if outline2_size > 0 {
                style.outline2_color
            } else {
                0
            },
            outline_total,
            has_shadow,
            shadow_color: if has_shadow { style.shadow_color } else { 0 },
            shadow_offset_x: if has_shadow { style.shadow_offset_x } else { 0 },
            shadow_offset_y: if has_shadow { style.shadow_offset_y } else { 0 },
        }
    }
}

/// 음수 방향 효과까지 담도록 layout 원점 앞에 padding을 둔 중간 bitmap.
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

/// 최근 bitmap cache miss가 임계치를 넘으면 직접 렌더링으로 우회한다.
/// 텍스트가 안정되면 같은 key의 hit가 쌓여 자동으로 bitmap 경로로 복귀한다.
///
/// ring/filled도 슬롯별로 유지한다. 슬롯을 분리하지 않으면 안정 블록의 hit가
/// 변동 블록의 miss와 한 창에 섞여 (a) 2블록 구성에서 overload에 진입하지 못해
/// escape hatch가 꺼지고, (b) 3블록 구성에서 상시 overload가 되어 안정 블록까지
/// direct 경로로 끌려간다.
pub(super) struct MissTracker {
    /// 슬롯별 최근 결과 bit ring. 1은 miss이며 높은 bit일수록 최신이다.
    rings: [u16; MeasureSlot::COUNT],
    /// 슬롯별 ring에 쌓인 샘플 수 (0..=WINDOW). WINDOW 도달 후로는 계속 WINDOW.
    filled: [u8; MeasureSlot::COUNT],
    /// 환경 변수로 조정 가능한 직접 렌더링 전환 임계치.
    pub(super) threshold: u8,
    /// 슬롯별 직전 key — Bitmap 없이도 텍스트 안정화를 판정하기 위한 것.
    last_keys: [Option<OutlineBitmapKey>; MeasureSlot::COUNT],
}

impl MissTracker {
    const WINDOW: u8 = 8;
    const DEFAULT_THRESHOLD: u8 = 5;
    /// Bit ring을 `WINDOW` 폭으로 자르는 mask.
    const RING_MASK: u16 = (1u16 << Self::WINDOW) - 1;

    pub(super) fn new() -> Self {
        Self {
            rings: [0; MeasureSlot::COUNT],
            filled: [0; MeasureSlot::COUNT],
            threshold: Self::resolve_threshold(),
            last_keys: [const { None }; MeasureSlot::COUNT],
        }
    }

    /// 슬롯의 직전 key를 읽는다.
    pub(super) fn last_key(&self, slot: MeasureSlot) -> Option<&OutlineBitmapKey> {
        self.last_keys[slot as usize].as_ref()
    }

    /// 슬롯의 직전 key를 갱신한다.
    pub(super) fn set_last_key(&mut self, slot: MeasureSlot, key: Option<OutlineBitmapKey>) {
        self.last_keys[slot as usize] = key;
    }

    /// 환경 변수의 1..=`WINDOW` 값을 읽고, 잘못되면 기본값을 쓴다.
    pub(super) fn resolve_threshold() -> u8 {
        std::env::var("ANEMONE_MISS_THRESHOLD")
            .ok()
            .and_then(|s| s.trim().parse::<u8>().ok())
            .filter(|&v| (1..=Self::WINDOW).contains(&v))
            .unwrap_or(Self::DEFAULT_THRESHOLD)
    }

    /// 슬롯의 블록 1회 결과를 기록. `miss=true` 면 outline 비트맵 캐시 miss.
    pub(super) fn record(&mut self, slot: MeasureSlot, miss: bool) {
        // 새 결과를 최상위 bit에 넣고 가장 오래된 bit를 버린다.
        let ring = &mut self.rings[slot as usize];
        *ring = ((*ring << 1) & Self::RING_MASK) | (miss as u16);
        let filled = &mut self.filled[slot as usize];
        if *filled < Self::WINDOW {
            *filled += 1;
        }
    }

    /// 슬롯의 window가 찬 뒤 miss 수가 임계치 이상인지 판정한다.
    pub(super) fn is_overloaded(&self, slot: MeasureSlot) -> bool {
        self.filled[slot as usize] >= Self::WINDOW
            && self.rings[slot as usize].count_ones() as u8 >= self.threshold
    }
}

#[cfg(test)]
#[path = "../../tests/unit/d2d/cache.rs"]
mod tests;
