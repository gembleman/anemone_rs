//! `WM_NCHITTEST`용 줄별 text 사각형 계산.

use windows::{
    Win32::{
        Foundation::{E_UNEXPECTED, RECT},
        Graphics::DirectWrite::*,
    },
    core::*,
};

use super::MeasureSlot;
use super::{
    TextBox,
    cache::{HitTestCache, HitTestKeyRef},
    renderer::D2DRenderer,
    style::TextRenderStyle,
};

impl D2DRenderer {
    /// `WM_NCHITTEST`용 줄별 text 사각형을 client 좌표로 반환한다.
    /// `inflate`는 outline/shadow 여유이며 빈 text면 빈 배열을 반환한다.
    /// `slot`은 layout 캐시의 유형별 슬롯이다.
    pub fn compute_text_line_rects(
        &mut self,
        slot: MeasureSlot,
        text: &str,
        style: &TextRenderStyle,
        bbox: TextBox,
        inflate: f32,
    ) -> Result<&[RECT]> {
        if text.is_empty() {
            self.hit_test_cache[slot as usize] = None;
            return Ok(&[]);
        }

        let TextBox {
            x: origin_x,
            y: origin_y,
            max_width,
            max_height: _,
        } = bbox;
        // layout은 measure와 공유하는 1M 박스로 만들어지므로 키도 같은 값을 쓴다.
        // bbox.max_height(= bitmap 높이 상한)를 넣으면 저장 키(1M)와 어긋나
        // 영구 miss가 된다 — hit-test 경로는 bbox.max_height를 쓰지 않는다.
        let key_ref = HitTestKeyRef::from_style(
            text,
            style,
            origin_x,
            origin_y,
            max_width,
            super::MEASURE_MAX_HEIGHT,
            inflate,
        );
        if self.hit_test_cache[slot as usize]
            .as_ref()
            .is_some_and(|cache| key_ref.matches(&cache.key))
        {
            // 위에서 Some·key 일치를 확인했으므로 도달 불가능하지만, 프로덕션
            // 경로에서 panic 대신 명시적 오류로 처리한다.
            let Some(cache) = self.hit_test_cache[slot as usize].as_ref() else {
                return Err(Error::new(
                    E_UNEXPECTED,
                    "hit-test cache missing after match check",
                ));
            };
            return Ok(&cache.rects);
        }

        // layout은 measure/draw와 공유한다 — 캐시 키에 맞춰 1M 박스로 조회.
        let layout =
            self.get_or_create_layout(slot, text, style, max_width, super::MEASURE_MAX_HEIGHT)?;
        // 함수 진입부의 `text.is_empty()` 가드 때문에 여기 텍스트는 항상
        // 비어있지 않다 — UTF-16 단위 수도 0이 될 수 없다.
        let text_len: u32 = text.encode_utf16().count() as u32;

        // SAFETY: 첫 호출로 크기를 얻고 정확한 buffer를 할당해 다시 호출한다.
        // 크기가 0이면 빈 결과와 실제 오류를 구분하도록 probe 결과를 반환한다.
        let metrics: Vec<DWRITE_HIT_TEST_METRICS> = unsafe {
            let mut needed: u32 = 0;
            let probe = layout.HitTestTextRange(0, text_len, 0.0, 0.0, None, &mut needed);
            if needed == 0 {
                return match probe {
                    Ok(()) => {
                        self.hit_test_cache[slot as usize] = None;
                        Ok(&[])
                    }
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
        let rects: Vec<RECT> = metrics
            .into_iter()
            .map(|m| {
                let left = (origin_x + m.left).floor() as i32 - inflate_i;
                let top = (origin_y + m.top).floor() as i32 - inflate_i;
                let right = (origin_x + m.left + m.width).ceil() as i32 + inflate_i;
                let bottom = (origin_y + m.top + m.height).ceil() as i32 + inflate_i;
                RECT {
                    left,
                    top,
                    right,
                    bottom,
                }
            })
            .collect();
        // get_or_create_layout이 방금 populate했으므로 도달 불가능하지만,
        // 프로덕션 경로에서 panic 대신 명시적 오류로 처리한다.
        let Some(cached_layout) = self.text_cache[slot as usize].as_ref() else {
            return Err(Error::new(
                E_UNEXPECTED,
                "layout cache not populated by get_or_create_layout",
            ));
        };
        let key = key_ref.to_owned_reusing_layout(&cached_layout.key);
        self.hit_test_cache[slot as usize] = Some(HitTestCache { key, rects });
        // 바로 위에서 채웠으므로 도달 불가능하지만, 동일한 이유로 panic 대신 처리한다.
        let Some(cache) = self.hit_test_cache[slot as usize].as_ref() else {
            return Err(Error::new(
                E_UNEXPECTED,
                "hit-test cache missing after insert",
            ));
        };
        Ok(&cache.rects)
    }
}
