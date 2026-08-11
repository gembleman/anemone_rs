use super::{FileTransJobData, FileTransTask, FileTranslationSupervisor, ProgressEvent, WriteType};
use super::mem::MemSnapshot;
use crate::translation::{Language, PreparedJob};
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

/// EzTrans DLL 벤치 테스트는 병렬 실행 시 공유 DLL 상태가 서로 충돌해 랜덤
/// 실패한다(변경 전 상태에서 재현 확인). 실제 사용 경로인 전체 파이프라인
/// 테스트 2건이 lock을 공유해 순차로 돌게 한다.
static EZTRANS_BENCH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
#[ignore = "benchmark that translates all 1,000 sample lines with bundled EzTrans"]
fn translates_japanese_translation_sample_with_eztrans() {
    let _guard = EZTRANS_BENCH_LOCK.lock().unwrap();
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
    sample_job.translation = PreparedJob::eztrans(
        dll_path.to_string_lossy().into_owned(),
        dat_path.to_string_lossy().into_owned(),
        4,
        Language::Jpn,
        Language::Kor,
    )
    .unwrap();

    let supervisor = FileTranslationSupervisor::new();
    let task = supervisor.start(sample_job).unwrap();
    let events = receive_through_terminal(&task);
    assert!(matches!(
        events.last(),
        Some(ProgressEvent::Finished(Ok(_)))
    ));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProgressEvent::Finished(Err(_))))
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
    let _guard = EZTRANS_BENCH_LOCK.lock().unwrap();
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
        sample_job.translation = PreparedJob::eztrans(
            dll_path.to_string_lossy().into_owned(),
            dat_path.to_string_lossy().into_owned(),
            4,
            Language::Jpn,
            Language::Kor,
        )
        .unwrap();

        let started = Instant::now();
        let mem_before = MemSnapshot::now();
        let supervisor = FileTranslationSupervisor::new();
        let task = supervisor.start(sample_job).unwrap();
        let events = receive_through_terminal(&task);
        let elapsed = started.elapsed();
        MemSnapshot::now().delta(mem_before).report_per(
            &format!("eztrans-{label}"),
            expected_lines,
        );

        assert!(matches!(
            events.last(),
            Some(ProgressEvent::Finished(Ok(_)))
        ));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ProgressEvent::Finished(Err(_))))
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
