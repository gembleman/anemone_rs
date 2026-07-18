use super::{
    LineEnding, PendingOutput, read_input_line, split_eztrans_batch, validate_and_count_reader,
    write_output,
};
use std::io::{BufReader, Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
    let temporary = pending.temp_path.clone();
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

    assert!(error.contains("invalid.txt"));
    assert!(error.contains("UTF-8 디코딩 실패"));
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

#[test]
fn cancellation_aborts_an_in_flight_file_http_request() {
    use crate::file_trans::FileTransJobData;
    use crate::translation::llm::{LlmCallParams, LlmProvider};
    use crate::translation::{EngineCredentials, Language, TranslationEngine};
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
        engine: TranslationEngine::Llm,
        source_lang: Language::Jpn,
        target_lang: Language::Kor,
        credentials: EngineCredentials::Llm(LlmCallParams {
            provider: LlmProvider::OpenAi,
            model: "test".into(),
            api_key: "test".into(),
            base_url: format!("http://{address}"),
            system_prompt: String::new(),
            temperature: 0.3,
            max_tokens: 10,
            glossary: Vec::new(),
        }),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = crate::translation::http_common::shared_client();
    let context = super::TranslationContext {
        runtime: &runtime,
        http_client: &client,
    };
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
