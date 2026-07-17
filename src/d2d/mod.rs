//! Direct2D 기반 렌더러
//!
//! 그리기 메서드들은 모두 `&ID2D1RenderTarget` 을 받도록 일반화돼
//! `CompositionRenderer` (DComp 합성 경로) 의 `ID2D1DeviceContext` 와
//! 미래에 추가될 다른 render target 모두에서 그대로 사용된다.

mod cache;
mod color;
mod outline_text_renderer;
mod renderer;
mod text;

pub use renderer::D2DRenderer;

/// 텍스트 layout 박스: 그리기 원점 `(x, y)` 과 layout 최대 크기.
///
/// `draw_text` / `compute_text_line_rects` 가 공유. layout 의 `SetMaxWidth`
/// / `SetMaxHeight` 에 `max_width` / `max_height` 가, 본문/그림자 등 모든
/// 그리기 명령의 원점에 `(x, y)` 가 들어간다.
#[derive(Clone, Copy)]
pub struct TextBox {
    pub x: f32,
    pub y: f32,
    pub max_width: f32,
    pub max_height: f32,
}
