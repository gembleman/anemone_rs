//! GUI 대화상자 모듈
//!
//! Phase 1 구현:
//! - 색상 선택 대화상자 (color.rs)
//! - 폰트 선택 대화상자 (font.rs)
//! - 설정 대화상자 (settings.rs)
//!
//! Phase 2 구현:
//! - 번역 대화상자 (translate.rs)
//! - 백로그 대화상자 (backlog.rs)
//!
//! Phase 3 구현:
//! - 파일 번역 대화상자 (file_trans.rs)
//! - 진행률 대화상자 (file_trans_progress.rs)
//!
//! 추가 구현:
//! - 후크 설정 대화상자 (hook_settings.rs)

pub mod helpers;

pub mod backlog;
pub mod color;
pub mod file_dialog;
pub mod file_trans;
pub mod file_trans_progress;
pub mod font;
pub mod glossary;
pub mod hook_settings;
pub mod settings;
pub mod translate;

pub use backlog::{BacklogDialog, BacklogStore, LogEntry, add_to_backlog};
pub use file_trans::FileTransDialog;
pub use hook_settings::HookSettingsDialog;
pub use settings::SettingsDialog;
pub use translate::TranslateDialog;
