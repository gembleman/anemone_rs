//! EzTrans, Google, DeepL, Papago, LLM, 커스텀 API 번역 엔진과 비동기 dispatch를 제공한다.

mod cache_key;
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
#[cfg(mys_private)]
pub(crate) mod mys_signup;
#[cfg_attr(not(mys_private), path = "mys_translater_stub.rs")]
pub(crate) mod mys_translater;
#[cfg_attr(not(mys_private), path = "mys_usage_stub.rs")]
pub(crate) mod mys_usage;
pub(crate) mod papago_api;
mod postprocess;
#[cfg(feature = "benchmark")]
pub use postprocess::{
    EzTransPostprocessMatcher as BenchmarkEzTransPostprocessMatcher,
    benchmark_apply_eztrans_dictionary,
};
mod service;
pub(crate) mod settings;
mod tls_pin;
pub(crate) mod worker;

pub(crate) use cache_key::CacheKey;
#[cfg(feature = "benchmark")]
pub use cache_key::CacheKey as BenchmarkCacheKey;
pub(crate) use deepl::DeepLApiTier;
pub use error::{TranslationError, TranslationResult};
pub(crate) use eztrans::EzTransTranslator;
pub(crate) use eztrans_actor::{prepare_eztrans, translate_with_eztrans};
#[cfg(feature = "benchmark")]
pub use eztrans_process::EzTransBatchTranslator as BenchmarkEzTransBatchTranslator;
pub(crate) use eztrans_process::{EzTransBatchTranslator, EzTransProcessConfig};
pub(crate) use eztrans_process::{EzTransProcessPoolRegistry, run_eztrans_worker};
pub(crate) use job::{DeepLStrategy, PreparedEngineKind};
pub use job::{PreparedJob, resolve_configured_eztrans_path};
pub use language::{EnumParseError, Language, TranslationEngine, lang_utils};
pub use llm::LlmProvider;
pub use service::TranslationService;

pub static GOOGLE_SUPPORTED_LANGUAGES: &[Language] = language::GOOGLE_SUPPORTED_LANGUAGES;
pub static DEEPL_SUPPORTED_LANGUAGES: &[Language] = language::DEEPL_SUPPORTED_LANGUAGES;
pub static PAPAGO_SUPPORTED_LANGUAGES: &[Language] = language::PAPAGO_SUPPORTED_LANGUAGES;
