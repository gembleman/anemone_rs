use super::{ProgressEvent, ProgressState};

#[test]
fn progress_events_update_model_state() {
    let mut state = ProgressState::default();
    for event in [
        ProgressEvent::TotalFiles(3),
        ProgressEvent::TotalLines(120),
        ProgressEvent::FileIndex(2),
        ProgressEvent::FileLines(40),
        ProgressEvent::TotalProgress(75),
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
    completed.apply(&ProgressEvent::TotalProgress(3));
    completed.apply(&ProgressEvent::Complete);
    completed.apply(&ProgressEvent::TotalProgress(99));
    assert_eq!(completed.current_line, 3);
    assert!(completed.terminal);

    let mut failed = ProgressState::default();
    failed.apply(&ProgressEvent::Error("failed".into()));
    failed.apply(&ProgressEvent::TotalFiles(99));
    assert_eq!(failed.total_files, 0);
    assert!(failed.terminal);
}

#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_cancel_and_close_request_task_cancellation() {
    use super::{FileTransProgressDialog, ctrl_id};
    use crate::file_trans::{FileTransJobData, FileTransRunner, WriteType};
    use crate::translation::{EngineCredentials, Language, TranslationEngine};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, GetDesktopWindow, GetDlgItem, SW_HIDE, SendMessageW, ShowWindow, WM_CLOSE,
        WM_COMMAND,
    };

    fn failing_job() -> FileTransJobData {
        FileTransJobData {
            input_files: vec![PathBuf::from("input.txt")],
            output_files: Vec::new(),
            write_type: WriteType::TranslationOnly,
            no_trans_linefeed: false,
            cancel_token: Arc::new(AtomicBool::new(false)),
            engine: TranslationEngine::Google,
            source_lang: Language::Jpn,
            target_lang: Language::Kor,
            credentials: EngineCredentials::None,
            eztrans_process: None,
        }
    }

    let parent = unsafe { GetDesktopWindow() };

    let close_task = FileTransRunner::start(failing_job());
    let close_cancel = close_task.cancel_handle();
    let close_dialog = FileTransProgressDialog::show(parent, close_task).unwrap();
    unsafe {
        let _ = ShowWindow(close_dialog, SW_HIDE);
        let _ = SendMessageW(close_dialog, WM_CLOSE, None, None);
    }
    assert!(close_cancel.is_cancelled());
    unsafe {
        DestroyWindow(close_dialog).unwrap();
    }

    let button_task = FileTransRunner::start(failing_job());
    let button_cancel = button_task.cancel_handle();
    let button_dialog = FileTransProgressDialog::show(parent, button_task).unwrap();
    let cancel_button = unsafe {
        let _ = ShowWindow(button_dialog, SW_HIDE);
        GetDlgItem(Some(button_dialog), ctrl_id::BTN_CANCEL as i32).unwrap()
    };
    unsafe {
        let command = WPARAM(usize::from(ctrl_id::BTN_CANCEL));
        let _ = SendMessageW(
            button_dialog,
            WM_COMMAND,
            Some(command),
            Some(LPARAM(cancel_button.0 as isize)),
        );
    }
    assert!(button_cancel.is_cancelled());
    assert!(!unsafe { IsWindowEnabled(cancel_button).as_bool() });
    unsafe {
        DestroyWindow(button_dialog).unwrap();
    }
}
