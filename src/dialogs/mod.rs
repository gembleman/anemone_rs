//! GUI 대화상자 모듈
//!
//! Phase 1 구현:
//! - 색상 선택 대화상자 (color.rs)
//! - 폰트 선택 대화상자 (font.rs)
//! - 설정 대화상자 (settings.rs)

pub mod color;
pub mod font;
pub mod settings;

pub use settings::SettingsDialog;
