mod app;
mod custom_api;
mod hook;
pub mod hotkey;
pub(crate) mod limits;
mod llm;
#[cfg_attr(not(mys_private), path = "secret_stub.rs")]
pub(crate) mod secret;
#[cfg(mys_private)]
mod secret_key;
mod text;
mod translation;

pub use app::Config;
pub(crate) use app::{DEFAULT_WINDOW_HEIGHT, DEFAULT_WINDOW_WIDTH};
pub use custom_api::CustomApiConfig;
pub use hook::{HookConfig, SavedHookProfile};
pub(crate) use hook::{MAX_MERGE_WINDOW_MS, MIN_MERGE_WINDOW_MS};
pub use hotkey::{HotkeyConfig, HotkeySlot, HotkeySpec};
pub use llm::{LlmConfig, LlmGlossaryEntry};
pub use text::{ColorType, TextAlign, TextStyle, TextType};
pub use translation::{
    EzTransPostprocessEntry, TranslationConfig, TranslationRoute, TranslationRouteConfig,
};
