//! `ID2D1RenderTarget` 구현체에 공통으로 쓰는 Direct2D renderer.

mod cache;
mod color;
mod composition;
mod outline_text_renderer;
mod renderer;
mod style;
mod text;

pub use composition::{CompositionRenderer, WaitOutcome};
pub use renderer::D2DRenderer;
pub(crate) use style::TextRenderStyle;

/// Text 그리기와 hit 영역 계산이 공유하는 원점과 최대 layout 크기.
#[derive(Clone, Copy)]
pub struct TextBox {
    pub x: f32,
    pub y: f32,
    pub max_width: f32,
    pub max_height: f32,
}

/// measure 결과 캐시의 direct-mapped 슬롯. 값은 배열 인덱스로 쓰인다.
///
/// paint가 측정하는 블록은 유형별로 최대 1개이므로, 슬롯을 텍스트 유형과
/// 1:1로 두면 퇴거 정책이 정의상 불필요하다. `config::TextType`을 직접
/// 쓰지 않고 분리하는 이유는 설정 도메인에 variant를 추가하지 않기 위해서다.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MeasureSlot {
    Name = 0,
    Original = 1,
    Translation = 2,
    Notice = 3,
}

impl MeasureSlot {
    pub(crate) const COUNT: usize = 4;

    /// 슬롯 전체 목록 — 배열 순회/정리에 쓴다.
    pub(crate) const ALL: [MeasureSlot; Self::COUNT] = [
        Self::Name,
        Self::Original,
        Self::Translation,
        Self::Notice,
    ];
}

/// `ALL`의 위치가 discriminant와 일치함을 컴파일 타임에 봉인한다.
///
/// `MeasureSlot::ALL.into_iter().zip(used)`는 슬롯 순서를 배열 위치에
/// 암묵 의존한다 — `ALL`을 재정렬하면 조용히 엉뚱한 슬롯을 폐기한다.
/// variant를 추가했는데 `ALL`/`COUNT`를 갱신하지 않으면 배열이 예전 크기
/// 그대로 컴파일되고, 새 슬롯이 쓰이는 순간 인덱스 초과 패닉이 난다 —
/// 아래 exhaustive match가 variant 추가를 컴파일 에러로 승격한다.
const _: () = {
    let mut index = 0;
    while index < MeasureSlot::COUNT {
        assert!(MeasureSlot::ALL[index] as usize == index);
        index += 1;
    }
    // variant를 추가하면 이 match가 깨진다 → `ALL`/`COUNT` 갱신을 강제한다.
    const fn _exhaustive(slot: MeasureSlot) {
        match slot {
            MeasureSlot::Name
            | MeasureSlot::Original
            | MeasureSlot::Translation
            | MeasureSlot::Notice => {}
        }
    }
};
