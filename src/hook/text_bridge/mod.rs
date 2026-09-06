//! TextOutput payload → 표시 가능한 UTF-8 문자열 변환과 텍스트 스레드 병합.
//!
//! - [`decode`]: payload 바이트 → UTF-8 디코딩 (codec/codepage 처리).
//!   [`LeadBytes`]가 한 바이트로 온 2바이트 글자의 앞뒤를 짝지어 준다.
//! - [`merger`]: 같은 출처의 연속 이벤트를 문장 단위로 병합.
//! - [`kirikiri`]: KiriKiri 계열 엔진의 화자 표기 후처리.

mod decode;
mod kirikiri;
mod merger;

pub use decode::LeadBytes;
pub(crate) use kirikiri::{is_kirikiri_engine, postprocess_hook_text};
pub use merger::{HookSource, HookText, TextMerger};
