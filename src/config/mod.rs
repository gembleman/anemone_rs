mod app;
mod custom_api;
pub mod hotkey;
pub(crate) mod limits;
mod llm;
mod text;
mod translation;

pub use app::Config;
pub use custom_api::CustomApiConfig;
pub use hotkey::{HotkeyConfig, HotkeySlot, HotkeySpec};
pub use llm::{LlmConfig, LlmGlossaryEntry};
pub use text::{ColorType, TextAlign, TextStyle, TextType};
pub use translation::TranslationConfig;
