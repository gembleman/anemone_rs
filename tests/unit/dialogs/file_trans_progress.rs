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
        }
    );
}
