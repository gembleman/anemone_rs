//! EzTrans, Google, DeepL, Papago, LLM, 커스텀 API 번역 엔진과 비동기 dispatch를 제공한다.

pub mod custom;
pub mod deepl;
mod error;
mod eztrans;
mod eztrans_actor;
mod eztrans_process;
pub mod google;
pub(crate) mod http_common;
mod job;
mod language;
pub mod llm;
pub mod manual;
pub mod papago;
pub mod settings;
pub mod worker;

pub use error::{TranslationError, TranslationResult};
pub use eztrans::EzTransTranslator;
pub use eztrans_actor::{prepare_eztrans, translate_with_eztrans};
pub use eztrans_process::{EzTransBatchTranslator, EzTransProcessConfig};
pub(crate) use eztrans_process::{global_eztrans_process_pool, run_eztrans_worker};
pub use job::TranslationJobSpec;
pub use language::{EnumParseError, Language, TranslationEngine, lang_utils};
pub use llm::LlmProvider;
pub use worker::EngineCredentials;

pub static GOOGLE_SUPPORTED_LANGUAGES: &[Language] = language::GOOGLE_SUPPORTED_LANGUAGES;
pub static DEEPL_SUPPORTED_LANGUAGES: &[Language] = language::DEEPL_SUPPORTED_LANGUAGES;
pub static PAPAGO_SUPPORTED_LANGUAGES: &[Language] = language::PAPAGO_SUPPORTED_LANGUAGES;
