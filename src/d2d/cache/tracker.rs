//! Prune 유예 추적기와 bitmap-cache miss 비율 추적기.

use super::super::MeasureSlot;
use super::outline_bitmap::OutlineBitmapKey;

/// prune 유예 기간 판정을 위한 슬롯별 연속 미사용 paint 추적기.
///
/// `D2DRenderer` 밖의 순수 상태로 분리해 유예 로직을 단위 테스트로 고정한다.
/// Name 블록은 줄 단위로 사라질 수 있으므로(구분자 없는 지문) paint 단위
/// 즉시 폐기는 대사↔지문 교대에서 캐시를 매번 파괴한다. 연속 `GRACE` paint
/// 미사용인 슬롯만 폐기 대상으로 보고한다. 여기서 paint는 **성공한 paint
/// 호출 1회**를 뜻한다 — 이 앱에는 렌더 루프가 없어 벽시계 시간과 무관하다.
pub(in crate::d2d) struct UnusedSlotTracker {
    paints: [u8; MeasureSlot::COUNT],
}

impl UnusedSlotTracker {
    /// 유예 paint 호출 수 — 이 이상 연속 미사용(성공한 paint)이면 폐기 대상.
    pub(in crate::d2d) const GRACE: u8 = 16;

    pub(in crate::d2d) fn new() -> Self {
        Self {
            paints: [0; MeasureSlot::COUNT],
        }
    }

    /// 이번 paint의 슬롯 사용 여부를 반영하고, 폐기 대상 슬롯 집합을 반환한다.
    ///
    /// 사용된 슬롯은 카운터를 리셋하고, 미사용 슬롯은 증가시킨다. `GRACE`
    /// 도달 시 해당 슬롯을 폐기 대상으로 보고하고 카운터를 리셋해 다음
    /// 유예 주기를 시작한다. 반환된 `to_prune`에서만 캐시·추적 상태를
    /// 폐기하면 판정 근거와 캐시 상태가 어긋나지 않는다.
    pub(in crate::d2d) fn record(
        &mut self,
        used: [bool; MeasureSlot::COUNT],
    ) -> [bool; MeasureSlot::COUNT] {
        let mut to_prune = [false; MeasureSlot::COUNT];
        for (slot, is_used) in MeasureSlot::ALL.into_iter().zip(used) {
            if is_used {
                self.paints[slot as usize] = 0;
                continue;
            }
            self.paints[slot as usize] = self.paints[slot as usize].saturating_add(1);
            if self.paints[slot as usize] >= Self::GRACE {
                self.paints[slot as usize] = 0;
                to_prune[slot as usize] = true;
            }
        }
        to_prune
    }
}

/// 최근 bitmap cache miss가 임계치를 넘으면 직접 렌더링으로 우회한다.
/// 텍스트가 안정되면 같은 key의 hit가 쌓여 자동으로 bitmap 경로로 복귀한다.
///
/// ring/filled도 슬롯별로 유지한다. 슬롯을 분리하지 않으면 안정 블록의 hit가
/// 변동 블록의 miss와 한 창에 섞여 (a) 2블록 구성에서 overload에 진입하지 못해
/// escape hatch가 꺼지고, (b) 3블록 구성에서 상시 overload가 되어 안정 블록까지
/// direct 경로로 끌려간다.
pub(in crate::d2d) struct MissTracker {
    /// 슬롯별 최근 결과 bit ring. 1은 miss이며 높은 bit일수록 최신이다.
    rings: [u16; MeasureSlot::COUNT],
    /// 슬롯별 ring에 쌓인 샘플 수 (0..=WINDOW). WINDOW 도달 후로는 계속 WINDOW.
    filled: [u8; MeasureSlot::COUNT],
    /// 환경 변수로 조정 가능한 직접 렌더링 전환 임계치.
    pub(in crate::d2d) threshold: u8,
    /// 슬롯별 직전 key — Bitmap 없이도 텍스트 안정화를 판정하기 위한 것.
    last_keys: [Option<OutlineBitmapKey>; MeasureSlot::COUNT],
}

impl MissTracker {
    const WINDOW: u8 = 8;
    const DEFAULT_THRESHOLD: u8 = 5;
    /// Bit ring을 `WINDOW` 폭으로 자르는 mask.
    const RING_MASK: u16 = (1u16 << Self::WINDOW) - 1;

    pub(in crate::d2d) fn new() -> Self {
        Self {
            rings: [0; MeasureSlot::COUNT],
            filled: [0; MeasureSlot::COUNT],
            threshold: Self::resolve_threshold(),
            last_keys: [const { None }; MeasureSlot::COUNT],
        }
    }

    /// 슬롯의 직전 key를 읽는다.
    pub(in crate::d2d) fn last_key(&self, slot: MeasureSlot) -> Option<&OutlineBitmapKey> {
        self.last_keys[slot as usize].as_ref()
    }

    /// 슬롯의 직전 key를 갱신한다.
    pub(in crate::d2d) fn set_last_key(
        &mut self,
        slot: MeasureSlot,
        key: Option<OutlineBitmapKey>,
    ) {
        self.last_keys[slot as usize] = key;
    }

    /// 슬롯의 ring과 last_key를 초기화한다.
    ///
    /// prune이 해당 슬롯의 캐시를 폐기할 때 함께 호출해, 정지된 옛 ring으로
    /// overload를 판정하거나 사라진 bitmap이 있는 것처럼 last_key가 일치하는
    /// 일을 막는다. 다른 슬롯에는 영향이 없다.
    pub(in crate::d2d) fn reset(&mut self, slot: MeasureSlot) {
        self.rings[slot as usize] = 0;
        self.filled[slot as usize] = 0;
        self.last_keys[slot as usize] = None;
    }

    /// 환경 변수의 1..=`WINDOW` 값을 읽고, 잘못되면 기본값을 쓴다.
    pub(in crate::d2d) fn resolve_threshold() -> u8 {
        std::env::var("ANEMONE_MISS_THRESHOLD")
            .ok()
            .and_then(|s| s.trim().parse::<u8>().ok())
            .filter(|&v| (1..=Self::WINDOW).contains(&v))
            .unwrap_or(Self::DEFAULT_THRESHOLD)
    }

    /// 슬롯의 블록 1회 결과를 기록. `miss=true` 면 outline 비트맵 캐시 miss.
    pub(in crate::d2d) fn record(&mut self, slot: MeasureSlot, miss: bool) {
        // 새 결과를 최상위 bit에 넣고 가장 오래된 bit를 버린다.
        let ring = &mut self.rings[slot as usize];
        *ring = ((*ring << 1) & Self::RING_MASK) | (miss as u16);
        let filled = &mut self.filled[slot as usize];
        if *filled < Self::WINDOW {
            *filled += 1;
        }
    }

    /// 슬롯의 window가 찬 뒤 miss 수가 임계치 이상인지 판정한다.
    pub(in crate::d2d) fn is_overloaded(&self, slot: MeasureSlot) -> bool {
        self.filled[slot as usize] >= Self::WINDOW
            && self.rings[slot as usize].count_ones() as u8 >= self.threshold
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/d2d/cache/tracker.rs"]
mod tests;
