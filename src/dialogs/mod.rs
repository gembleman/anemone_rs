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

pub mod color;
pub mod font;
pub mod settings;
pub mod translate;
pub mod backlog;
pub mod file_trans;
pub mod file_trans_progress;
pub mod file_trans_thread;

pub use settings::SettingsDialog;
pub use translate::TranslateDialog;
pub use backlog::{BacklogDialog, LogEntry, add_to_backlog};
pub use file_trans::FileTransDialog;
