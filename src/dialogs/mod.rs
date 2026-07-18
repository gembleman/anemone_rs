//! 설정, 번역, backlog, file 작업, hook 관련 GUI dialog.

pub mod helpers;
pub(crate) mod host;
pub(crate) mod models;

pub mod backlog;
pub mod color;
pub mod file_dialog;
pub mod file_trans;
pub mod file_trans_progress;
pub mod font;
pub mod glossary;
pub mod settings;
pub mod translate;

pub use crate::backlog::{BacklogStore, LogEntry};
pub use backlog::BacklogDialog;
pub use file_trans::FileTransDialog;
pub use settings::SettingsDialog;
pub use translate::TranslateDialog;
