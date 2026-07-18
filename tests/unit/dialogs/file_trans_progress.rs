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
