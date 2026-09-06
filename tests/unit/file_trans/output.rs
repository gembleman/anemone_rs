use super::{PendingOutput, write_output};
use crate::file_trans::WriteType;
use crate::file_trans::input::LineEnding;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "anemone-file-output-unit-test-{}-{sequence}",
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

// ---- write_output 내용 검증 ----

#[test]
fn write_output_original_and_trans_includes_both_lines_in_order() {
    let mut buffer = Vec::new();
    write_output(
        &mut buffer,
        "원문",
        "번역문",
        WriteType::OriginalAndTrans,
        LineEnding::Lf,
        false,
    )
    .unwrap();
    assert_eq!(String::from_utf8(buffer).unwrap(), "원문\n번역문\n");
}

#[test]
fn write_output_translation_only_omits_the_original_line() {
    let mut buffer = Vec::new();
    write_output(
        &mut buffer,
        "원문",
        "번역문",
        WriteType::TranslationOnly,
        LineEnding::CrLf,
        false,
    )
    .unwrap();
    assert_eq!(String::from_utf8(buffer).unwrap(), "번역문\r\n");
}

#[test]
fn write_output_with_more_lines_inserts_a_blank_separator_line() {
    let mut buffer = Vec::new();
    write_output(
        &mut buffer,
        "원문",
        "번역문",
        WriteType::OriginalTransNewline,
        LineEnding::Lf,
        true,
    )
    .unwrap();
    // has_more일 때는 줄 끝 대신 빈 줄 하나를 더 넣어 다음 원문/번역 쌍과 구분한다.
    assert_eq!(String::from_utf8(buffer).unwrap(), "원문\n번역문\n\n");
}

#[test]
fn write_output_without_more_lines_ends_with_the_line_ending_only() {
    let mut buffer = Vec::new();
    write_output(
        &mut buffer,
        "원문",
        "번역문",
        WriteType::OriginalTransNewline,
        LineEnding::Lf,
        false,
    )
    .unwrap();
    assert_eq!(String::from_utf8(buffer).unwrap(), "원문\n번역문\n");
}

#[test]
fn write_output_propagates_a_write_failure() {
    struct FailingWriter;
    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("디스크가 가득 찼습니다"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let error = write_output(
        &mut FailingWriter,
        "원문",
        "번역문",
        WriteType::TranslationOnly,
        LineEnding::Lf,
        false,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("디스크가 가득 찼습니다"),
        "{error}"
    );
}

// ---- PendingOutput ----

#[test]
fn pending_output_create_fails_when_the_parent_directory_is_missing() {
    let dir = TestDirectory::new();
    let path = dir.0.join("missing-parent").join("out.txt");

    let error = match PendingOutput::create(&path) {
        Ok(_) => panic!("부모 디렉터리가 없으면 생성에 실패해야 합니다"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("임시 출력 파일을 생성할 수 없습니다")
    );
}

#[test]
fn write_after_persist_state_returns_an_error_instead_of_panicking() {
    // persist()는 self를 소비하므로 정상 경로에서는 이 상태에 도달할 수 없다.
    // Drop이 부분 이동을 막기 때문에 Option<Writer>로 불변식을 옮긴 방어 코드를
    // 직접 재현해 패닉 대신 오류를 반환하는지 검증한다.
    let dir = TestDirectory::new();
    let mut already_persisted = PendingOutput {
        final_path: dir.0.join("final.txt"),
        temp_path: PathBuf::new(),
        writer: None,
    };

    let write_error = already_persisted.write_all(b"more data").unwrap_err();
    assert!(write_error.to_string().contains("이미 저장(persist)되어"));

    let flush_error = already_persisted.flush().unwrap_err();
    assert!(flush_error.to_string().contains("이미 저장(persist)되어"));
}

#[test]
fn persisting_an_already_persisted_instance_returns_an_error_instead_of_panicking() {
    // 위 테스트와 마찬가지로, 이 상태는 공개 API로는 도달할 수 없는 방어 코드다.
    // `persist` 자체가 이 상태를 방어하는지(패닉 대신 오류) 직접 검증한다.
    let dir = TestDirectory::new();
    let already_persisted = PendingOutput {
        final_path: dir.0.join("final.txt"),
        temp_path: dir.0.join("temp.tmp"),
        writer: None,
    };

    let error = match already_persisted.persist() {
        Ok(()) => panic!("writer가 없는 상태에서 persist가 성공하면 안 됩니다"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("이미 저장(persist)되었습니다"));
}
