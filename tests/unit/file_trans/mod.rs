use super::{
    CancelHandle, FileTransJobData, FileTransRunner, FileTransTask, ProgressEvent, WriteType,
    default_output_paths, run, validate_job_paths,
};
use crate::translation::{EngineCredentials, Language, TranslationEngine};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
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
        engine: TranslationEngine::Google,
        source_lang: Language::Jpn,
        target_lang: Language::Kor,
        credentials: EngineCredentials::None,
    }
}

fn receive_through_terminal(task: &FileTransTask) -> Vec<ProgressEvent> {
    let mut events = Vec::new();
    loop {
        let event = task
            .events
            .recv_timeout(Duration::from_secs(5))
            .expect("file translation terminal event");
        let terminal = matches!(event, ProgressEvent::Complete | ProgressEvent::Error(_));
        events.push(event);
        if terminal {
            events.extend(task.events.try_iter());
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

#[test]
fn runner_delivers_ordered_progress_and_one_complete() {
    let directory = TestDirectory::new();
    let input = directory.0.join("empty.txt");
    let output = directory.0.join("result.txt");
    std::fs::write(&input, "").unwrap();

    let task = FileTransRunner::start(job(vec![input], vec![output]));
    let events = receive_through_terminal(&task);

    assert_eq!(
        events,
        [
            ProgressEvent::TotalFiles(1),
            ProgressEvent::TotalLines(0),
            ProgressEvent::FileIndex(1),
            ProgressEvent::FileName("empty.txt".into()),
            ProgressEvent::FileLines(0),
            ProgressEvent::Complete,
        ]
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ProgressEvent::Complete))
            .count(),
        1
    );
}

#[test]
fn runner_error_has_one_terminal_error_and_no_complete() {
    let directory = TestDirectory::new();
    let input = directory.0.join("input.txt");
    std::fs::write(&input, "").unwrap();

    let task = FileTransRunner::start(job(vec![input], vec![]));
    let events = receive_through_terminal(&task);

    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ProgressEvent::Error(_)))
            .count(),
        1
    );
    assert!(!events.contains(&ProgressEvent::Complete));
}

#[test]
fn runner_cancellation_has_one_terminal_error_and_no_complete() {
    let directory = TestDirectory::new();
    let input = directory.0.join("large.txt");
    let output = directory.0.join("result.txt");
    std::fs::write(&input, " \n".repeat(100_000)).unwrap();
    std::fs::write(&output, "기존 결과").unwrap();

    let task = FileTransRunner::start(job(vec![input], vec![output.clone()]));
    task.cancel();
    let events = receive_through_terminal(&task);

    assert_eq!(std::fs::read_to_string(output).unwrap(), "기존 결과");
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ProgressEvent::Error(_)))
            .count(),
        1
    );
    assert!(!events.contains(&ProgressEvent::Complete));
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
            .filter(|event| matches!(event, ProgressEvent::Error(_)))
            .count(),
        1
    );
    assert!(!events.contains(&ProgressEvent::Complete));
}

#[test]
#[ignore = "long-running test that translates all 1,000 sample lines with bundled EzTrans"]
fn translates_japanese_translation_sample_with_eztrans() {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let input = project_root
        .join("tests")
        .join("japanese_translation_sample.txt");
    let dll_path = project_root.join("eztrans_dll").join("J2KEngine.dll");
    let dat_path = project_root.join("eztrans_dll").join("Dat");
    let directory = TestDirectory::new();
    let output = directory.0.join("japanese_translation_sample_ko.txt");
    let expected_lines = std::fs::read_to_string(&input).unwrap().lines().count() as i32;
    let mut sample_job = job(vec![input.clone()], vec![output.clone()]);
    sample_job.engine = TranslationEngine::EzTrans;

    crate::translation::prepare_eztrans(
        dll_path.to_str().expect("EzTrans DLL path is valid UTF-8"),
        dat_path.to_str().expect("EzTrans DAT path is valid UTF-8"),
    )
    .expect("bundled EzTrans initializes");

    let task = FileTransRunner::start(sample_job);
    let events = receive_through_terminal(&task);
    assert_eq!(events.last(), Some(&ProgressEvent::Complete));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProgressEvent::Error(_)))
    );
    assert!(events.contains(&ProgressEvent::TotalFiles(1)));
    assert!(events.contains(&ProgressEvent::TotalLines(expected_lines)));
    assert!(events.contains(&ProgressEvent::FileProgress(expected_lines)));
    assert!(events.contains(&ProgressEvent::TotalProgress(expected_lines)));

    let output_bytes = std::fs::read(output).unwrap();
    let translated = output_bytes
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .expect("translated file has a UTF-8 BOM");
    let translated = std::str::from_utf8(translated).unwrap();

    assert_eq!(translated.lines().count() as i32, expected_lines);
    assert!(!translated.contains("[번역 실패:"));
    assert!(
        translated
            .chars()
            .any(|character| ('가'..='힣').contains(&character))
    );
}
