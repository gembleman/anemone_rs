use super::{FileTranslationProgress, ProgressState};

#[test]
fn progress_events_update_model_state() {
    let mut state = ProgressState::default();
    for event in [
        FileTranslationProgress::TotalFiles(3),
        FileTranslationProgress::TotalLines(120),
        FileTranslationProgress::FileIndex(2),
        FileTranslationProgress::FileLines(40),
        FileTranslationProgress::TotalProgress(75),
    ] {
        state.apply(&event);
    }

    assert_eq!(
        state,
        ProgressState {
            total_files: 3,
            current_file_index: 2,
            total_lines: 120,
            current_line: 75,
            list_size: 40,
            terminal: false,
        }
    );
}

#[test]
fn terminal_event_prevents_later_state_updates() {
    let mut completed = ProgressState::default();
    completed.apply(&FileTranslationProgress::TotalProgress(3));
    completed.apply(&FileTranslationProgress::Finished(Ok(
        crate::file_trans::FileTranslationSummary {
            total_files: 1,
            total_lines: 3,
        },
    )));
    completed.apply(&FileTranslationProgress::TotalProgress(99));
    assert_eq!(completed.current_line, 3);
    assert!(completed.terminal);

    let mut failed = ProgressState::default();
    failed.apply(&FileTranslationProgress::Finished(Err(
        crate::file_trans::FileTranslationError::Runtime("failed".into()),
    )));
    failed.apply(&FileTranslationProgress::TotalFiles(99));
    assert_eq!(failed.total_files, 0);
    assert!(failed.terminal);
}

#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_cancel_and_close_request_task_cancellation() {
    use super::{FileTransProgressDialog, ctrl_id};
    use crate::file_trans::{FileTranslationRequest, FileTranslationSupervisor, WriteType};
    use crate::translation::{Language, PreparedJob};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, GetDesktopWindow, GetDlgItem, SW_HIDE, SendMessageW, ShowWindow, WM_CLOSE,
        WM_COMMAND,
    };

    fn failing_job() -> FileTranslationRequest {
        FileTranslationRequest {
            input_files: vec![PathBuf::from("input.txt")],
            output_files: Vec::new(),
            write_type: WriteType::TranslationOnly,
            no_trans_linefeed: false,
            cancel_token: Arc::new(AtomicBool::new(false)),
            translation: PreparedJob::google(Language::Jpn, Language::Kor).unwrap(),
        }
    }

    let parent = unsafe { GetDesktopWindow() };

    let close_supervisor = FileTranslationSupervisor::new();
    let close_task = close_supervisor.start(failing_job()).unwrap();
    let close_cancel = close_task.cancel_handle();
    let close_dialog = FileTransProgressDialog::show(parent, close_task).unwrap();
    unsafe {
        let _ = ShowWindow(close_dialog, SW_HIDE);
        let _ = SendMessageW(close_dialog, WM_CLOSE, 0, 0);
    }
    assert!(close_cancel.is_cancelled());
    unsafe {
        DestroyWindow(close_dialog);
    }

    let button_supervisor = FileTranslationSupervisor::new();
    let button_task = button_supervisor.start(failing_job()).unwrap();
    let button_cancel = button_task.cancel_handle();
    let button_dialog = FileTransProgressDialog::show(parent, button_task).unwrap();
    let cancel_button = unsafe {
        let _ = ShowWindow(button_dialog, SW_HIDE);
        GetDlgItem(button_dialog, ctrl_id::BTN_CANCEL as i32)
    };
    unsafe {
        let command = usize::from(ctrl_id::BTN_CANCEL);
        let _ = SendMessageW(button_dialog, WM_COMMAND, command, cancel_button as isize);
    }
    assert!(button_cancel.is_cancelled());
    assert_eq!(unsafe { IsWindowEnabled(cancel_button) }, 0);
    unsafe {
        DestroyWindow(button_dialog);
    }
}
