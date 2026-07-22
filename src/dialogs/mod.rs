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

fn trackbar_thumb_position(code: u32, wparam: usize) -> Option<i32> {
    use windows::Win32::UI::Controls::{TB_THUMBPOSITION, TB_THUMBTRACK};

    match code {
        TB_THUMBPOSITION | TB_THUMBTRACK => Some(((wparam >> 16) & 0xffff) as i32),
        _ => None,
    }
}
