//! `measure_text_height` 결과 캐시 키/항목.

use std::sync::Arc;

use crate::config::TextAlign;

use super::super::MeasureSlot;
use super::super::style::TextRenderStyle;

/// 측정 결과 캐시 키 — 레이아웃 높이에 영향을 주는 값만 담는다.
///
/// measure는 높이를 계산하는 것이 목적이므로 `max_height`는 키에 넣지 않는다.
/// 호출측에서 항상 같은 상한(`MEASURE_MAX_HEIGHT`)을 쓰기 때문에 변별력이 없다.
/// 색상/외곽선/그림자는 layout metrics에 영향을 주지 않으므로 제외한다.
#[derive(Clone, Eq, PartialEq, Hash)]
pub(in crate::d2d) struct MeasureKey {
    pub(in crate::d2d) text: Arc<str>,
    pub(in crate::d2d) font_face: Arc<str>,
    pub(in crate::d2d) font_size: i32,
    pub(in crate::d2d) font_style: u8,
    pub(in crate::d2d) text_align: TextAlign,
    pub(in crate::d2d) max_width_bits: u32,
}

/// 문자열을 빌리는 조회용 뷰.
pub(in crate::d2d) struct MeasureKeyRef<'a> {
    pub(in crate::d2d) text: &'a str,
    pub(in crate::d2d) font_face: &'a str,
    pub(in crate::d2d) font_size: i32,
    pub(in crate::d2d) font_style: u8,
    pub(in crate::d2d) text_align: TextAlign,
    pub(in crate::d2d) max_width_bits: u32,
}

impl<'a> MeasureKeyRef<'a> {
    pub(in crate::d2d) fn from_style(
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

    pub(in crate::d2d) fn matches(&self, key: &MeasureKey) -> bool {
        self.font_size == key.font_size
            && self.font_style == key.font_style
            && self.text_align == key.text_align
            && self.max_width_bits == key.max_width_bits
            && self.text == &*key.text
            && self.font_face == &*key.font_face
    }

    pub(in crate::d2d) fn to_owned(&self) -> MeasureKey {
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
pub(in crate::d2d) struct MeasureCacheEntry {
    pub(in crate::d2d) key: MeasureKey,
    pub(in crate::d2d) height: f32,
}

/// `measure_text_height` 결과 캐시 — 텍스트 유형별 direct-mapped 단일 엔트리.
///
/// paint는 유형별로 최대 1블록만 측정하므로(이름·원문·번역·notice), 슬롯을
/// 유형과 1:1로 두면 퇴거 정책이 필요 없다. "이름 고정 + 대사만 변경" 패턴에서
/// 고정 블록의 hit가 유지된다. 크기가 고정이라 무한 성장하지 않는다.
pub(in crate::d2d) struct MeasureCache {
    entries: [Option<MeasureCacheEntry>; MeasureSlot::COUNT],
}

impl MeasureCache {
    pub(in crate::d2d) fn new() -> Self {
        Self {
            entries: [const { None }; MeasureSlot::COUNT],
        }
    }

    pub(in crate::d2d) fn get(&self, slot: MeasureSlot, key: &MeasureKeyRef<'_>) -> Option<f32> {
        self.entries[slot as usize]
            .as_ref()
            .filter(|entry| key.matches(&entry.key))
            .map(|entry| entry.height)
    }

    /// miss 시 호출: 해당 슬롯을 덮어쓴다. 다른 슬롯은 건드리지 않는다.
    pub(in crate::d2d) fn insert(&mut self, slot: MeasureSlot, key: MeasureKey, height: f32) {
        self.entries[slot as usize] = Some(MeasureCacheEntry { key, height });
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/d2d/cache/measure.rs"]
mod tests;
