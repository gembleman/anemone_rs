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
}
