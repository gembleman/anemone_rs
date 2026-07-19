//! EzTrans, Google, DeepL, Papago, LLM, 커스텀 API 번역 엔진과 비동기 dispatch를 제공한다.

pub(crate) mod custom;
pub(crate) mod deepl;
mod error;
mod eztrans;
mod eztrans_actor;
mod eztrans_process;
pub(crate) mod google;
pub(crate) mod http_common;
mod job;
mod language;
pub(crate) mod llm;
pub(crate) mod manual;
pub(crate) mod papago_api;
mod service;
pub(crate) mod settings;
pub(crate) mod worker;

pub use error::{TranslationError, TranslationResult};
pub(crate) use eztrans::EzTransTranslator;
pub(crate) use eztrans_actor::{prepare_eztrans, translate_with_eztrans};
#[cfg(feature = "benchmark")]
pub use eztrans_process::EzTransBatchTranslator as BenchmarkEzTransBatchTranslator;
pub(crate) use eztrans_process::{EzTransBatchTranslator, EzTransProcessConfig};
pub(crate) use eztrans_process::{EzTransProcessPoolRegistry, run_eztrans_worker};
pub(crate) use job::{DeepLStrategy, PreparedEngineKind};
pub use job::{LanguagePair, PreparedEngine, PreparedJob};
pub use language::{EnumParseError, Language, TranslationEngine, lang_utils};
pub use llm::LlmProvider;
pub use service::TranslationService;

pub static GOOGLE_SUPPORTED_LANGUAGES: &[Language] = language::GOOGLE_SUPPORTED_LANGUAGES;
pub static DEEPL_SUPPORTED_LANGUAGES: &[Language] = language::DEEPL_SUPPORTED_LANGUAGES;
pub static PAPAGO_SUPPORTED_LANGUAGES: &[Language] = language::PAPAGO_SUPPORTED_LANGUAGES;
