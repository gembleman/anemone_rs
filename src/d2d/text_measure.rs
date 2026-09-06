//! 텍스트 블록의 시각적 줄 높이 측정.

use windows::{Win32::Graphics::DirectWrite::*, core::*};

use super::MeasureSlot;
use super::{cache::MeasureKeyRef, renderer::D2DRenderer, style::TextRenderStyle};

impl D2DRenderer {
    /// 주어진 폭에서 DirectWrite가 계산한 실제 시각적 줄 높이를 반환한다.
    /// 명시적 개행 수가 아니라 layout metrics를 사용하므로 자동 줄바꿈도 포함한다.
    ///
    /// `slot`은 결과 캐시의 direct-mapped 슬롯이다. 텍스트 유형(Name/Original/
    /// Translation/Notice)별로 1블록만 측정되므로, "이름 고정 + 대사만 변경"
    /// 패턴에서 고정 블록의 측정 결과가 유지된다. 같은 슬롯이라도 키가 다르면
    /// miss로 보고 재측정한다. 캐시는 `invalidate_device_caches`(장치 손실·DPI
    /// 변경)에서 폐기된다.
    pub fn measure_text_height(
        &mut self,
        slot: MeasureSlot,
        text: &str,
        style: &TextRenderStyle,
        max_width: f32,
    ) -> Result<f32> {
        let key_ref = MeasureKeyRef::from_style(text, style, max_width);
        if let Some(height) = self.measure_cache.get(slot, &key_ref) {
            return Ok(height);
        }
        // 만든 layout은 draw/hit-test 경로와 공유한다 — `max_height`를
        // [`MEASURE_MAX_HEIGHT`]로 통일해 `get_or_create_layout`이 같은 키로
        // hit하게 한다 (텍스트 변경당 layout 생성 2→1회).
        let layout =
            self.get_or_create_layout(slot, text, style, max_width, super::MEASURE_MAX_HEIGHT)?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe {
            layout.GetMetrics(&mut metrics)?;
        }
        let height = metrics.height.max(style.font_size.max(1) as f32);
        self.measure_cache.insert(slot, key_ref.to_owned(), height);
        Ok(height)
    }
}
