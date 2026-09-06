use super::{BatchWrite, PendingOutput, collect_eztrans_window, collect_http_batch, write_batch};
use crate::file_trans::input::{InputLine, LineEnding, read_input_line};
use crate::file_trans::{
    FileTranslationError, FileTranslationProgress, FileTranslationRequest, WriteType,
};
use crate::translation::{Language, PreparedJob};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "anemone-file-batch-test-{}-{sequence}",
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

fn reader_over(lines: &[&str]) -> BufReader<std::io::Cursor<Vec<u8>>> {
    BufReader::new(std::io::Cursor::new(lines.join("\n").into_bytes()))
}

fn job_data() -> FileTranslationRequest {
    FileTranslationRequest {
        input_files: Vec::new(),
        output_files: Vec::new(),
        write_type: WriteType::TranslationOnly,
        no_trans_linefeed: false,
        cancel_token: Arc::new(AtomicBool::new(false)),
        translation: PreparedJob::eztrans(
            "dict".into(),
            "ehnd".into(),
            1,
            Language::Jpn,
            Language::Kor,
        )
        .unwrap(),
    }
}

// ---- collect_http_batch ----

#[test]
fn collect_http_batch_reads_every_line_until_eof_when_under_the_batch_size() {
    let mut reader = reader_over(&["a", "b", "c"]);
    let path = Path::new("in.txt");
    let first = read_input_line(&mut reader, path, true).unwrap().unwrap();

    let (lines, remaining) = collect_http_batch(&mut reader, path, first).unwrap();
    assert_eq!(
        lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert!(remaining.is_none());
}

#[test]
fn collect_http_batch_stops_at_the_configured_batch_size() {
    let source: Vec<String> = (0..40).map(|i| format!("l{i}")).collect();
    let refs: Vec<&str> = source.iter().map(String::as_str).collect();
    let mut reader = reader_over(&refs);
    let path = Path::new("in.txt");
    let first = read_input_line(&mut reader, path, true).unwrap().unwrap();

    let (lines, remaining) = collect_http_batch(&mut reader, path, first).unwrap();
    let batch_limit = super::FILE_HTTP_BATCH_LINES;
    assert_eq!(lines.len(), batch_limit);
    assert_eq!(remaining.unwrap().text, format!("l{batch_limit}"));
}

// ---- collect_eztrans_window ----

#[test]
fn collect_eztrans_window_reads_every_line_until_eof_when_small() {
    let mut reader = reader_over(&["가", "나", "다"]);
    let path = Path::new("in.txt");
    let first = read_input_line(&mut reader, path, true).unwrap().unwrap();

    let (lines, remaining) = collect_eztrans_window(&mut reader, path, first).unwrap();
    assert_eq!(
        lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
        ["가", "나", "다"]
    );
    assert!(remaining.is_none());
}

#[test]
fn collect_eztrans_window_stops_at_the_configured_line_count_limit() {
    // 창(window)은 아무리 짧은 줄이라도 EZTRANS_WINDOW_MAX_LINES개를 넘으면
    // 잘라서 다음 창으로 넘겨야 한다.
    let max_lines = super::EZTRANS_WINDOW_MAX_LINES;
    let source: Vec<String> = (0..=max_lines).map(|i| format!("l{i}")).collect();
    let refs: Vec<&str> = source.iter().map(String::as_str).collect();
    let mut reader = reader_over(&refs);
    let path = Path::new("in.txt");
    let first = read_input_line(&mut reader, path, true).unwrap().unwrap();

    let (lines, remaining) = collect_eztrans_window(&mut reader, path, first).unwrap();
    assert_eq!(lines.len(), max_lines);
    assert_eq!(remaining.unwrap().text, format!("l{max_lines}"));
}

// ---- write_batch ----

fn batch_write<'a>(
    lines: Vec<InputLine>,
    translated: Vec<&str>,
    has_more_after: bool,
    line_count: usize,
    job_data: &'a FileTranslationRequest,
) -> BatchWrite<'a> {
    BatchWrite {
        lines,
        translated_lines: translated.into_iter().map(String::from).collect(),
        has_more_after,
        line_count,
        job_data,
    }
}

fn input_line(text: &str) -> InputLine {
    InputLine {
        text: text.to_string(),
        ending: LineEnding::Lf,
    }
}

#[test]
fn write_batch_reports_progress_on_first_last_and_every_interval_line_only() {
    let dir = TestDirectory::new();
    let output_path = dir.0.join("out.txt");
    let mut pending = PendingOutput::create(&output_path).unwrap();
    let job = job_data();

    let batch = batch_write(
        vec![input_line("one"), input_line("two"), input_line("three")],
        vec!["하나", "둘", "셋"],
        false,
        3,
        &job,
    );
    let mut line_index = 0usize;
    let mut global_current = 0i32;
    let events = std::cell::RefCell::new(Vec::new());
    let report = |event: FileTranslationProgress| events.borrow_mut().push(event);

    write_batch(
        &mut pending,
        batch,
        &mut line_index,
        &mut global_current,
        &report,
    )
    .unwrap();

    let recorded = events.into_inner();
    // 첫 줄(1)과 마지막 줄(line_count=3)에서만 보고해야 하고, 중간 줄(2)에서는
    // 보고하지 않아야 한다. 보고 하나마다 FileProgress+TotalProgress 쌍이 온다.
    assert_eq!(
        recorded,
        vec![
            FileTranslationProgress::FileProgress(1),
            FileTranslationProgress::TotalProgress(1),
            FileTranslationProgress::FileProgress(3),
            FileTranslationProgress::TotalProgress(3),
        ]
    );
    assert_eq!(line_index, 3);
    assert_eq!(global_current, 3);
}

#[test]
fn write_batch_stops_immediately_on_cancellation_without_writing_remaining_lines() {
    let dir = TestDirectory::new();
    let output_path = dir.0.join("out.txt");
    let mut pending = PendingOutput::create(&output_path).unwrap();
    let job = job_data();

    let batch = batch_write(
        vec![input_line("one"), input_line("two")],
        vec!["하나", "둘"],
        false,
        2,
        &job,
    );
    let mut line_index = 0usize;
    let mut global_current = 0i32;

    // write_output 이후, 다음 줄로 넘어가기 전에 취소를 확인하므로 두 번째 줄을
    // 쓰기 전에 첫 번째 줄만 반영된 채로 멈춰야 한다. cancel_token을 콜백 안에서
    // 첫 줄 처리 직후 켠다.
    let job_ref = &job;
    let report = |_event: FileTranslationProgress| {
        job_ref.cancel_token.store(true, Ordering::SeqCst);
    };

    let result = write_batch(
        &mut pending,
        batch,
        &mut line_index,
        &mut global_current,
        &report,
    );

    assert_eq!(result, Err(FileTranslationError::Cancelled));
    assert_eq!(line_index, 1, "취소 전까지 처리한 줄만 반영되어야 합니다");
}
