use super::{
    BoundedTranslationCache, InputLine, LineEnding, PendingOutput, partition_eztrans_batches,
    read_input_line, split_eztrans_batch, translate_eztrans_window, validate_and_count_reader,
    write_output,
};
use crate::translation::EzTransBatchTranslator;
use std::io::{BufReader, Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "anemone-file-output-test-{}-{sequence}",
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

#[test]
fn dropped_pending_output_preserves_existing_file() {
    let directory = TestDirectory::new();
    let output = directory.0.join("result.txt");
    std::fs::write(&output, "기존 결과").unwrap();

    {
        let mut pending = PendingOutput::create(&output).unwrap();
        pending.writer().write_all(b"incomplete").unwrap();
    }

    assert_eq!(std::fs::read_to_string(output).unwrap(), "기존 결과");
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
}

#[test]
fn persisted_output_replaces_existing_file() {
    let directory = TestDirectory::new();
    let output = directory.0.join("result.txt");
    std::fs::write(&output, "기존 결과").unwrap();

    let mut pending = PendingOutput::create(&output).unwrap();
    pending.writer().write_all("완성 결과".as_bytes()).unwrap();
    pending.persist().unwrap();

    assert_eq!(std::fs::read_to_string(output).unwrap(), "완성 결과");
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
}

#[test]
fn failed_persist_removes_the_temporary_output() {
    let directory = TestDirectory::new();
    let output = directory.0.join("occupied");
    std::fs::create_dir(&output).unwrap();

    let mut pending = PendingOutput::create(&output).unwrap();
    let temporary = pending.temp_path().to_path_buf();
    pending
        .writer()
        .write_all(b"complete but cannot replace")
        .unwrap();

    assert!(pending.persist().is_err());
    assert!(!temporary.exists());
    assert!(output.is_dir());
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
}

#[test]
fn line_read_error_is_returned_instead_of_skipped() {
    let reader = BufReader::new(Cursor::new(vec![0xFF, b'\n']));
    let cancel = std::sync::atomic::AtomicBool::new(false);

    let error = validate_and_count_reader(reader, Path::new("invalid.txt"), &cancel).unwrap_err();

    assert!(error.to_string().contains("invalid.txt"));
    assert!(error.to_string().contains("UTF-8 디코딩 실패"));
}

#[test]
fn input_line_preserves_lf_crlf_and_eof() {
    let mut reader = BufReader::new(Cursor::new(b"\xEF\xBB\xBFone\r\ntwo\nthree"));
    let one = read_input_line(&mut reader, Path::new("input.txt"), true)
        .unwrap()
        .unwrap();
    let two = read_input_line(&mut reader, Path::new("input.txt"), false)
        .unwrap()
        .unwrap();
    let three = read_input_line(&mut reader, Path::new("input.txt"), false)
        .unwrap()
        .unwrap();
    assert_eq!((one.text.as_str(), one.ending), ("one", LineEnding::CrLf));
    assert_eq!((two.text.as_str(), two.ending), ("two", LineEnding::Lf));
    assert_eq!(
        (three.text.as_str(), three.ending),
        ("three", LineEnding::None)
    );
}

#[test]
fn output_modes_preserve_final_newline_presence() {
    for write_type in [
        super::WriteType::TranslationOnly,
        super::WriteType::OriginalAndTrans,
        super::WriteType::OriginalTransNewline,
    ] {
        for ending in [LineEnding::None, LineEnding::Lf, LineEnding::CrLf] {
            let mut output = Vec::new();
            write_output(
                &mut output,
                "original",
                "translated",
                write_type,
                ending,
                false,
            )
            .unwrap();
            assert_eq!(output.ends_with(b"\n"), ending != LineEnding::None);
            if ending == LineEnding::CrLf {
                assert!(!output.windows(2).any(|window| window == b"d\n"));
            }
        }
    }
}

#[test]
fn eztrans_batch_split_removes_only_inserted_boundary_spaces() {
    let originals = [
        "これは一行目です。",
        "  これは二行目です。",
        "これは三行目です。  ",
        "「台詞です」",
    ];
    let translated =
        "이것은 1행째입니다. \n   이것은 2행째입니다. \n 이것은 3행째입니다.   \n 「대사입니다」";

    assert_eq!(
        split_eztrans_batch(translated, &originals),
        Some(vec![
            "이것은 1행째입니다.".into(),
            "   이것은 2행째입니다.".into(),
            "이것은 3행째입니다.   ".into(),
            "「대사입니다」".into(),
        ])
    );
}

#[test]
fn eztrans_batch_split_rejects_changed_line_boundaries() {
    assert_eq!(split_eztrans_batch("하나\n둘\n셋", &["一", "二"]), None);
}

struct MockBatchTranslator {
    process_count: usize,
    translated_lines: AtomicUsize,
    batches: Mutex<Vec<Vec<String>>>,
}

impl MockBatchTranslator {
    fn new(process_count: usize) -> Self {
        Self {
            process_count,
            translated_lines: AtomicUsize::new(0),
            batches: Mutex::new(Vec::new()),
        }
    }
}

impl EzTransBatchTranslator for MockBatchTranslator {
    fn process_count(&self) -> usize {
        self.process_count
    }

    fn translate_batches(
        &self,
        batches: Vec<Vec<Arc<str>>>,
        _cancelled: &Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Vec<Vec<String>>, String> {
        self.translated_lines.fetch_add(
            batches.iter().map(Vec::len).sum::<usize>(),
            Ordering::Relaxed,
        );
        self.batches.lock().unwrap().extend(
            batches
                .iter()
                .map(|batch| batch.iter().map(|text| text.to_string()).collect()),
        );
        Ok(batches
            .into_iter()
            .map(|batch| {
                batch
                    .into_iter()
                    .map(|text| format!("번역:{text}"))
                    .collect()
            })
            .collect())
    }
}

fn eztrans_job() -> crate::file_trans::FileTransJobData {
    use crate::translation::{Language, PreparedJob};
    crate::file_trans::FileTransJobData {
        input_files: Vec::new(),
        output_files: Vec::new(),
        write_type: super::WriteType::TranslationOnly,
        no_trans_linefeed: true,
        cancel_token: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        translation: PreparedJob::eztrans(
            "test.dll".into(),
            "test-dat".into(),
            1,
            Language::Jpn,
            Language::Kor,
        )
        .unwrap(),
    }
}

fn input_line(text: &str) -> InputLine {
    InputLine {
        text: text.into(),
        ending: LineEnding::Lf,
    }
}

#[test]
fn eztrans_window_deduplicates_and_reuses_bounded_cache() {
    let translator = MockBatchTranslator::new(4);
    let job = eztrans_job();
    let mut cache = BoundedTranslationCache::new(100);
    let first = [input_line("同じ"), input_line("別"), input_line("同じ")];
    assert_eq!(
        translate_eztrans_window(&first, &job, &translator, &mut cache).unwrap(),
        ["번역:同じ", "번역:別", "번역:同じ"]
    );
    let second = [input_line("同じ"), input_line("新規")];
    assert_eq!(
        translate_eztrans_window(&second, &job, &translator, &mut cache).unwrap(),
        ["번역:同じ", "번역:新規"]
    );
    assert_eq!(translator.translated_lines.load(Ordering::Relaxed), 3);
}

#[test]
fn eztrans_window_applies_postprocess_dictionary_before_caching() {
    let translator = MockBatchTranslator::new(1);
    let mut config = crate::config::TranslationConfig::default();
    config.eztrans_dll_path = "test.dll".into();
    config.eztrans_dat_path = "test-dat".into();
    config.eztrans_postprocess_dictionary = vec![crate::config::EzTransPostprocessEntry {
        source: "번역:".into(),
        target: "후처리:".into(),
    }];
    let mut job = eztrans_job();
    job.translation = crate::translation::PreparedJob::from_config(&config).unwrap();
    let mut cache = BoundedTranslationCache::new(10);

    assert_eq!(
        translate_eztrans_window(&[input_line("문장")], &job, &translator, &mut cache).unwrap(),
        ["후처리:문장"]
    );
    assert_eq!(
        translate_eztrans_window(&[input_line("문장")], &job, &translator, &mut cache).unwrap(),
        ["후처리:문장"]
    );
    assert_eq!(translator.translated_lines.load(Ordering::Relaxed), 1);
}

#[test]
fn eztrans_cache_keeps_a_reused_entry_during_a_unique_scan() {
    let translator = MockBatchTranslator::new(1);
    let job = eztrans_job();
    let mut cache = BoundedTranslationCache::new(4);

    let initial = [
        input_line("반복"),
        input_line("고유-1"),
        input_line("고유-2"),
        input_line("고유-3"),
    ];
    translate_eztrans_window(&initial, &job, &translator, &mut cache).unwrap();
    translate_eztrans_window(&[input_line("반복")], &job, &translator, &mut cache).unwrap();
    translate_eztrans_window(&[input_line("고유-4")], &job, &translator, &mut cache).unwrap();
    translate_eztrans_window(&[input_line("반복")], &job, &translator, &mut cache).unwrap();

    assert_eq!(translator.translated_lines.load(Ordering::Relaxed), 5);
}

#[test]
fn eztrans_batches_are_balanced_across_configured_processes_and_bounded() {
    let originals = (0..1_000)
        .map(|index| Arc::<str>::from(format!("문장 {index}")))
        .collect::<Vec<_>>();
    let batches = partition_eztrans_batches(&originals, 8);
    assert!(batches.len() >= 8);
    assert!(batches.iter().all(|batch| batch.len() <= 256));
    assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), 1_000);
}

#[test]
fn cancellation_aborts_an_in_flight_file_http_request() {
    use crate::config::{LlmConfig, TranslationConfig};
    use crate::file_trans::FileTransJobData;
    use crate::translation::PreparedJob;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = listener.accept();
        std::thread::sleep(Duration::from_secs(2));
    });

    let cancel_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let job = FileTransJobData {
        input_files: Vec::new(),
        output_files: Vec::new(),
        write_type: super::WriteType::TranslationOnly,
        no_trans_linefeed: false,
        cancel_token: cancel_token.clone(),
        translation: PreparedJob::from_config(&TranslationConfig {
            engine: "llm".into(),
            llm: LlmConfig {
                model: "test".into(),
                api_key: "test".into(),
                base_url: format!("http://{address}"),
                system_prompt: String::new(),
                temperature: 0.3,
                max_tokens: 10,
                glossary: Vec::new(),
                ..LlmConfig::default()
            },
            ..TranslationConfig::default()
        })
        .unwrap(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = crate::translation::http_common::create_client();
    let context = super::TranslationContext::new(&runtime, &client);
    let cancel = cancel_token.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        cancel.store(true, Ordering::SeqCst);
    });

    let started = Instant::now();
    let result = super::translate_line("source", &job, &context);

    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
}
