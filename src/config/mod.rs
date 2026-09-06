mod app;
mod custom_api;
mod hook;
pub mod hotkey;
pub(crate) mod limits;
mod llm;
pub(crate) mod secret;
/// AES 키 원재료. 비공개 파일이 없으면 공개 stub이 쓰인다(`build.rs`).
#[cfg_attr(not(mys_private), path = "secret_key_stub.rs")]
mod secret_key;
mod text;
mod translation;

pub use app::Config;
pub use custom_api::CustomApiConfig;
pub use hook::{HookConfig, SavedHookProfile};
pub(crate) use hook::{MAX_MERGE_WINDOW_MS, MIN_MERGE_WINDOW_MS};
pub use hotkey::{HotkeyConfig, HotkeySlot, HotkeySpec};
pub use llm::{LlmConfig, LlmGlossaryEntry};
pub use text::{ColorType, TextAlign, TextStyle, TextType};
pub use translation::{EzTransPostprocessEntry, TranslationConfig};
