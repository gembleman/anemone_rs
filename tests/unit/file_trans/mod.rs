use super::{
    FileTransJobData, FileTransTask, FileTranslationError, FileTranslationSummary,
    FileTranslationSupervisor, ProgressEvent, WriteType, default_output_paths, run,
    validate_job_paths,
};
use crate::translation::{Language, PreparedJob};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "anemone-runner-test-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn job(input_files: Vec<PathBuf>, output_files: Vec<PathBuf>) -> FileTransJobData {
    FileTransJobData {
        input_files,
        output_files,
        write_type: WriteType::TranslationOnly,
        no_trans_linefeed: true,
        cancel_token: Arc::new(AtomicBool::new(false)),
        translation: PreparedJob::google(Language::Jpn, Language::Kor).unwrap(),
    }
}

fn receive_through_terminal(task: &FileTransTask) -> Vec<ProgressEvent> {
    let mut events = Vec::new();
    loop {
        let event = task
            .recv_event_timeout(Duration::from_secs(5))
            .expect("file translation terminal event");
        let terminal = event.is_terminal();
        events.push(event);
        if terminal {
            events.extend(task.drain_events());
            return events;
        }
    }
}

#[test]
fn rejects_output_matching_input_after_normalization() {
    let input = PathBuf::from(r"C:\work\text\input.txt");
    let output = PathBuf::from(r"c:\WORK\text\.\input.txt");

    assert!(validate_job_paths(&[input], &[output]).is_err());
}

#[test]
fn rejects_duplicate_outputs_after_normalization() {
    let inputs = [
        PathBuf::from(r"C:\work\a.txt"),
        PathBuf::from(r"C:\work\b.txt"),
    ];
    let outputs = [
        PathBuf::from(r"C:\work\result.txt"),
        PathBuf::from(r"c:\WORK\.\result.txt"),
    ];

    assert!(validate_job_paths(&inputs, &outputs).is_err());
}

#[test]
fn makes_distinct_defaults_for_equal_stems() {
    let inputs = [
        PathBuf::from(r"C:\work\a.txt"),
        PathBuf::from(r"C:\work\a.log"),
    ];

    let outputs = default_output_paths(&inputs).unwrap();

    assert_eq!(outputs[0], PathBuf::from(r"C:\work\a_번역.txt"));
    assert_eq!(outputs[1], PathBuf::from(r"C:\work\a_번역_2.txt"));
    validate_job_paths(&inputs, &outputs).unwrap();
}

#[test]
fn dropping_task_requests_cancellation_without_joining() {
    let directory = TestDirectory::new();
    let input = directory.0.join("empty.txt");
    let output = directory.0.join("result.txt");
    std::fs::write(&input, "").unwrap();
    let supervisor = FileTranslationSupervisor::new();
    let task = supervisor.start(job(vec![input], vec![output])).unwrap();
    let cancel = task.cancel_handle();
    drop(task);
    assert!(cancel.is_cancelled());
}

#[test]
fn runner_delivers_ordered_progress_and_one_complete() {
    let directory = TestDirectory::new();
    let input = directory.0.join("empty.txt");
    let output = directory.0.join("result.txt");
    std::fs::write(&input, "").unwrap();

    let supervisor = FileTranslationSupervisor::new();
    let task = supervisor.start(job(vec![input], vec![output])).unwrap();
    let events = receive_through_terminal(&task);

    assert_eq!(
        events,
        [
            ProgressEvent::TotalFiles(1),
            ProgressEvent::TotalLines(0),
            ProgressEvent::FileIndex(1),
            ProgressEvent::FileName("empty.txt".into()),
            ProgressEvent::FileLines(0),
            ProgressEvent::Finished(Ok(FileTranslationSummary {
                total_files: 1,
                total_lines: 0,
            })),
        ]
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ProgressEvent::Finished(Ok(_))))
            .count(),
        1
    );
}

#[test]
fn runner_error_has_one_terminal_error_and_no_complete() {
    let directory = TestDirectory::new();
    let input = directory.0.join("input.txt");
    std::fs::write(&input, "").unwrap();

    let supervisor = FileTranslationSupervisor::new();
    let task = supervisor.start(job(vec![input], vec![])).unwrap();
    let events = receive_through_terminal(&task);

    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ProgressEvent::Finished(Err(_))))
            .count(),
        1
    );
}

#[test]
fn runner_cancellation_has_one_cancelled_and_no_complete() {
    let directory = TestDirectory::new();
    let input = directory.0.join("large.txt");
    let output = directory.0.join("result.txt");
    std::fs::write(&input, " \n".repeat(100_000)).unwrap();
    std::fs::write(&output, "기존 결과").unwrap();

    let supervisor = FileTranslationSupervisor::new();
    let task = supervisor
        .start(job(vec![input], vec![output.clone()]))
        .unwrap();
    task.cancel();
    let events = receive_through_terminal(&task);

    assert_eq!(std::fs::read_to_string(output).unwrap(), "기존 결과");
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    ProgressEvent::Finished(Err(FileTranslationError::Cancelled))
                )
            })
            .count(),
        1
    );
}

#[test]
fn cancellation_cleans_temporary_output_and_preserves_existing_result() {
    let directory = TestDirectory::new();
    let input = directory.0.join("input.txt");
    let output = directory.0.join("result.txt");
    std::fs::write(&input, " \n \n").unwrap();
    std::fs::write(&output, "기존 결과").unwrap();
    let job = job(vec![input], vec![output.clone()]);
    let cancel = job.cancel_token.clone();
    let events = std::cell::RefCell::new(Vec::new());

    run(&job, |event| {
        if event == ProgressEvent::FileProgress(1) {
            cancel.store(true, Ordering::SeqCst);
        }
        events.borrow_mut().push(event);
    });
    let events = events.into_inner();

    assert_eq!(std::fs::read_to_string(output).unwrap(), "기존 결과");
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 2);
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    ProgressEvent::Finished(Err(FileTranslationError::Cancelled))
                )
            })
            .count(),
        1
    );
}

#[test]
fn supervisor_cancels_joins_and_rejects_new_work_during_shutdown() {
    let directory = TestDirectory::new();
    let input = directory.0.join("large.txt");
    let output = directory.0.join("result.txt");
    std::fs::write(&input, " \n".repeat(100_000)).unwrap();

    let supervisor = FileTranslationSupervisor::new();
    let task = supervisor
        .start(job(vec![input.clone()], vec![output.clone()]))
        .unwrap();
    let report = supervisor.shutdown(Duration::from_secs(2));

    assert_eq!(report.detached, 0);
    assert_eq!(supervisor.active_tasks(), 0);
    assert!(task.cancel_handle().is_cancelled());
    assert!(supervisor.start(job(vec![input], vec![output])).is_err());
}
