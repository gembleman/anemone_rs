use super::{
    detect_non_utf8_bom, open_utf8_translation_input, preflight_inputs, read_utf8_preview,
};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "anemone-file-input-test-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn file(&self, name: &str, contents: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---- detect_non_utf8_bom ----

#[test]
fn detect_non_utf8_bom_identifies_utf16_and_utf32_prefixes() {
    assert_eq!(
        detect_non_utf8_bom(&[0x00, 0x00, 0xFE, 0xFF, b'x']),
        Some("UTF-32 BE")
    );
    assert_eq!(
        detect_non_utf8_bom(&[0xFF, 0xFE, 0x00, 0x00, b'x']),
        Some("UTF-32 LE")
    );
    assert_eq!(detect_non_utf8_bom(&[0xFE, 0xFF, b'x']), Some("UTF-16 BE"));
    assert_eq!(detect_non_utf8_bom(&[0xFF, 0xFE, b'x']), Some("UTF-16 LE"));
}

#[test]
fn detect_non_utf8_bom_allows_utf8_and_plain_text() {
    assert_eq!(detect_non_utf8_bom(&[0xEF, 0xBB, 0xBF, b'x']), None);
    assert_eq!(detect_non_utf8_bom(b"plain ascii"), None);
    assert_eq!(detect_non_utf8_bom(&[]), None);
}

// ---- open_utf8_translation_input ----

#[test]
fn open_utf8_translation_input_reads_a_plain_utf8_file() {
    let dir = TestDirectory::new();
    let path = dir.file("plain.txt", "안녕하세요\n".as_bytes());

    let mut reader = open_utf8_translation_input(&path).unwrap();
    let mut buf = String::new();
    reader.read_to_string(&mut buf).unwrap();
    assert_eq!(buf, "안녕하세요\n");
}

#[test]
fn open_utf8_translation_input_allows_a_utf8_bom() {
    let dir = TestDirectory::new();
    let mut contents = vec![0xEF, 0xBB, 0xBF];
    contents.extend_from_slice("안녕".as_bytes());
    let path = dir.file("bom.txt", &contents);

    let mut reader = open_utf8_translation_input(&path).unwrap();
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).unwrap();
    assert_eq!(buf, contents);
}

#[test]
fn open_utf8_translation_input_rejects_utf16_le() {
    let dir = TestDirectory::new();
    let path = dir.file("utf16le.txt", &[0xFF, 0xFE, b'a', 0x00]);

    let error = open_utf8_translation_input(&path).unwrap_err();
    assert!(error.contains("UTF-16 LE"), "{error}");
}

#[test]
fn open_utf8_translation_input_rejects_utf16_be() {
    let dir = TestDirectory::new();
    let path = dir.file("utf16be.txt", &[0xFE, 0xFF, 0x00, b'a']);

    let error = open_utf8_translation_input(&path).unwrap_err();
    assert!(error.contains("UTF-16 BE"), "{error}");
}

#[test]
fn open_utf8_translation_input_rejects_utf32_le() {
    let dir = TestDirectory::new();
    let path = dir.file("utf32le.txt", &[0xFF, 0xFE, 0x00, 0x00, b'a']);

    let error = open_utf8_translation_input(&path).unwrap_err();
    assert!(error.contains("UTF-32 LE"), "{error}");
}

#[test]
fn open_utf8_translation_input_reports_a_missing_file() {
    let dir = TestDirectory::new();
    let path = dir.0.join("does-not-exist.txt");

    let error = open_utf8_translation_input(&path).unwrap_err();
    assert!(error.contains("입력 파일을 열 수 없습니다"), "{error}");
}

// ---- read_utf8_preview ----

#[test]
fn read_utf8_preview_strips_the_utf8_bom() {
    let dir = TestDirectory::new();
    let mut contents = vec![0xEF, 0xBB, 0xBF];
    contents.extend_from_slice("첫줄\n둘째줄\n".as_bytes());
    let path = dir.file("preview.txt", &contents);

    let preview = read_utf8_preview(&path, 10, 1024).unwrap();
    assert_eq!(preview, "첫줄\r\n둘째줄");
}

#[test]
fn read_utf8_preview_truncates_to_max_lines() {
    let dir = TestDirectory::new();
    let path = dir.file("lines.txt", "1\n2\n3\n4\n5\n".as_bytes());

    let preview = read_utf8_preview(&path, 2, 1024).unwrap();
    assert_eq!(preview, "1\r\n2");
}

#[test]
fn read_utf8_preview_cleanly_truncates_a_char_boundary_at_the_byte_limit() {
    let dir = TestDirectory::new();
    // 각 글자가 3바이트인 한글 5개 -> 15바이트. 한도를 4바이트로 주면 글자 하나(3바이트)만 남고
    // 잘린 멀티바이트 꼬리는 버려져야 한다 (에러가 아니라 자연스러운 절단).
    let path = dir.file("cut.txt", "가나다라마".as_bytes());

    let preview = read_utf8_preview(&path, 10, 4).unwrap();
    assert_eq!(preview, "가");
}

#[test]
fn read_utf8_preview_rejects_genuinely_invalid_utf8() {
    let dir = TestDirectory::new();
    let path = dir.file("invalid.txt", &[b'a', 0xFF, b'b']);

    let error = read_utf8_preview(&path, 10, 1024).unwrap_err();
    assert!(error.contains("UTF-8 디코딩 실패"), "{error}");
}

// ---- preflight_inputs ----

#[test]
fn preflight_inputs_counts_lines_across_multiple_files() {
    let dir = TestDirectory::new();
    let a = dir.file("a.txt", b"one\ntwo\n");
    let b = dir.file("b.txt", b"only-one-line-no-trailing-newline");

    let cancel = AtomicBool::new(false);
    let counts = preflight_inputs(&[a, b], &cancel).unwrap();
    assert_eq!(counts, vec![2, 1]);
}

#[test]
fn preflight_inputs_reports_the_offending_file_on_bad_encoding() {
    let dir = TestDirectory::new();
    let good = dir.file("good.txt", b"ok\n");
    let bad = dir.file("bad.txt", &[0xFF, b'\n']);

    let cancel = AtomicBool::new(false);
    let error = preflight_inputs(&[good, bad.clone()], &cancel).unwrap_err();
    assert!(error.to_string().contains(&bad.display().to_string()));
}

#[test]
fn preflight_inputs_stops_immediately_when_already_cancelled() {
    let dir = TestDirectory::new();
    let path = dir.file("any.txt", b"line\n");

    let cancel = AtomicBool::new(true);
    let error = preflight_inputs(&[path], &cancel).unwrap_err();
    assert_eq!(error, crate::file_trans::FileTranslationError::Cancelled);
}
