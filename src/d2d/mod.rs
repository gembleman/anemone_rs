//! `ID2D1RenderTarget` 구현체에 공통으로 쓰는 Direct2D renderer.

mod cache;
mod color;
mod composition;
mod outline_text_renderer;
mod renderer;
mod text;

pub use composition::{CompositionRenderer, WaitOutcome};
pub use renderer::D2DRenderer;

/// Text 그리기와 hit 영역 계산이 공유하는 원점과 최대 layout 크기.
#[derive(Clone, Copy)]
pub struct TextBox {
    pub x: f32,
    pub y: f32,
    pub max_width: f32,
    pub max_height: f32,
}
