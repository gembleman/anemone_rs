use super::{FileTransJobData, FileTransRunner, FileTransTask, ProgressEvent, WriteType};
use crate::translation::{EngineCredentials, Language, TranslationEngine};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

static BENCH_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct BenchDirectory(PathBuf);

impl BenchDirectory {
    fn new() -> Self {
        let sequence = BENCH_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "anemone-file-translation-bench-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for BenchDirectory {
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
        eztrans_process: None,
    }
}

fn receive_through_terminal(task: &FileTransTask) -> Vec<ProgressEvent> {
    let mut events = Vec::new();
    loop {
        let event = task
            .recv_event_timeout(Duration::from_secs(5))
            .expect("file translation terminal event");
        let terminal = matches!(event, ProgressEvent::Complete | ProgressEvent::Error(_));
        events.push(event);
        if terminal {
            events.extend(task.drain_events());
            return events;
        }
    }
}

#[test]
#[ignore = "benchmark that translates all 1,000 sample lines with bundled EzTrans"]
fn translates_japanese_translation_sample_with_eztrans() {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let input = project_root
        .join("benchmark")
        .join("japanese_translation_sample.txt");
    let dll_path = project_root.join("eztrans_dll").join("J2KEngine.dll");
    let dat_path = project_root.join("eztrans_dll").join("Dat");
    let directory = BenchDirectory::new();
    let output = directory.0.join("japanese_translation_sample_ko.txt");
    let expected_lines = std::fs::read_to_string(&input).unwrap().lines().count() as i32;
    let mut sample_job = job(vec![input], vec![output.clone()]);
    sample_job.engine = TranslationEngine::EzTrans;
    sample_job.eztrans_process = Some(crate::translation::EzTransProcessConfig {
        dll_path: dll_path.to_string_lossy().into_owned(),
        dat_path: dat_path.to_string_lossy().into_owned(),
        process_count: 4,
    });

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

#[test]
#[ignore = "performance benchmark that uses the bundled EzTrans DLL"]
fn measures_repeated_and_unique_sample_translation_performance() {
    assert!(
        !std::hint::black_box(cfg!(debug_assertions)),
        "performance measurements must run with cargo test --release"
    );
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dll_path = project_root.join("eztrans_dll").join("J2KEngine.dll");
    let dat_path = project_root.join("eztrans_dll").join("Dat");
    let directory = BenchDirectory::new();

    for (label, filename, expected_lines) in [
        (
            "repeated-200k",
            "large_japanese_translation_sample.txt",
            200_000,
        ),
        (
            "unique-10k",
            "unique_japanese_translation_sample.txt",
            10_000,
        ),
    ] {
        let input = project_root.join("benchmark").join(filename);
        let output = directory.0.join(format!("{label}_ko.txt"));
        let mut sample_job = job(vec![input], vec![output.clone()]);
        sample_job.engine = TranslationEngine::EzTrans;
        sample_job.eztrans_process = Some(crate::translation::EzTransProcessConfig {
            dll_path: dll_path.to_string_lossy().into_owned(),
            dat_path: dat_path.to_string_lossy().into_owned(),
            process_count: 4,
        });

        let started = Instant::now();
        let task = FileTransRunner::start(sample_job);
        let events = receive_through_terminal(&task);
        let elapsed = started.elapsed();

        assert_eq!(events.last(), Some(&ProgressEvent::Complete));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ProgressEvent::Error(_)))
        );
        let translated = std::fs::read_to_string(output).unwrap();
        let translated = translated.strip_prefix('\u{feff}').unwrap_or(&translated);
        assert_eq!(translated.lines().count(), expected_lines);
        assert!(!translated.contains("[번역 실패:"));

        eprintln!(
            "{label}: {expected_lines} lines in {:.3}s ({:.0} lines/s)",
            elapsed.as_secs_f64(),
            expected_lines as f64 / elapsed.as_secs_f64()
        );
    }
}
