use super::{CancelHandle, FileTransTask, default_output_paths, validate_job_paths};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, mpsc};

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
    let token = Arc::new(AtomicBool::new(false));
    let (_sender, receiver) = mpsc::channel();
    let task = FileTransTask {
        cancel: CancelHandle(token.clone()),
        events: receiver,
        worker: None,
    };
    drop(task);
    assert!(token.load(std::sync::atomic::Ordering::SeqCst));
}
