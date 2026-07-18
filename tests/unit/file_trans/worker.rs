use super::{PendingOutput, count_reader_lines};
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
fn line_read_error_is_returned_instead_of_skipped() {
    let reader = BufReader::new(Cursor::new(vec![0xFF, b'\n']));

    let error = count_reader_lines(reader, Path::new("invalid.txt")).unwrap_err();

    assert!(error.contains("invalid.txt"));
    assert!(error.contains("입력 파일을 읽을 수 없습니다"));
}
