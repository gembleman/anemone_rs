use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::super::UpdateError;
use super::{backup_path, can_write_dir, cleanup_backup, is_transient, replace_running_executable};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 실행 중인 exe로는 자동 테스트를 할 수 없으므로 같은 이름 규칙의 더미
/// 파일로 파일시스템 상태 전이를 검증한다.
struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "anemone-update-apply-{label}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("테스트 디렉터리를 만들 수 있어야 한다");
        Self(path)
    }

    fn file(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).expect("파일을 쓸 수 있어야 한다");
        path
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("파일을 읽을 수 있어야 한다")
}

#[test]
fn replacing_moves_the_old_file_aside_and_installs_the_new_one() {
    let dir = TestDir::new("ok");
    let current = dir.file("anemone_rs.exe", "구버전");
    let staged = dir.file("anemone_rs.exe.new", "신버전");

    replace_running_executable(&current, &staged).expect("교체에 성공해야 한다");

    assert_eq!(read(&current), "신버전");
    assert_eq!(
        read(&backup_path(&current)),
        "구버전",
        "이전 버전은 롤백을 위해 남아 있어야 한다"
    );
    assert!(!staged.exists(), "staged 파일은 제자리로 옮겨져야 한다");
}

#[test]
fn a_leftover_backup_from_a_previous_update_is_overwritten() {
    let dir = TestDir::new("leftover");
    let current = dir.file("anemone_rs.exe", "구버전");
    let staged = dir.file("anemone_rs.exe.new", "신버전");
    dir.file("anemone_rs.exe.old", "아주 오래된 버전");

    replace_running_executable(&current, &staged).expect("교체에 성공해야 한다");

    assert_eq!(read(&current), "신버전");
    assert_eq!(read(&backup_path(&current)), "구버전");
}

/// 서명 없는 exe가 백신에 격리되면 실제로 밟히는 경로다. 예외가 아니라
/// 정상 경로로 다뤄야 한다.
#[test]
fn a_missing_staged_file_rolls_back_to_the_previous_executable() {
    let dir = TestDir::new("rollback");
    let current = dir.file("anemone_rs.exe", "구버전");
    let staged = dir.0.join("사라진-파일.exe");

    let error = replace_running_executable(&current, &staged)
        .expect_err("staged 파일이 없으면 실패해야 한다");

    assert!(
        !matches!(error, UpdateError::RollbackFailed { .. }),
        "되돌리기는 성공했어야 한다: {error}"
    );
    assert_eq!(
        read(&current),
        "구버전",
        "실행 파일이 제자리에 복구되어야 한다"
    );
    assert!(
        !backup_path(&current).exists(),
        "되돌린 뒤에는 백업이 남지 않아야 한다"
    );
}

/// 대상 폴더에 쓸 수 없으면(Program Files 등) 1단계로 넘어가지 않고 즉시
/// 실패해야 한다 — 부분적으로 파일을 옮겨놓고 실패하는 것보다 안전하다.
#[test]
fn replacing_in_an_unwritable_directory_fails_before_touching_any_file() {
    let dir = TestDir::new("unwritable");
    let missing_parent = dir.0.join("존재하지-않는-폴더").join("anemone_rs.exe");
    let staged = dir.file("anemone_rs.exe.new", "신버전");

    let error = replace_running_executable(&missing_parent, &staged)
        .expect_err("쓸 수 없는 폴더는 실패해야 한다");

    assert!(matches!(error, UpdateError::NotWritable(_)));
    assert!(
        staged.exists(),
        "실패 전이므로 staged 파일은 그대로 남아 있어야 한다"
    );
}

/// 현재 실행 파일이 이미 없으면(경합 등으로) 1단계에서 즉시 실패해야 한다 —
/// 백업으로 옮길 대상 자체가 없으므로 롤백을 시도할 필요도 없다.
#[test]
fn a_missing_current_executable_fails_at_the_first_step_without_a_rollback() {
    let dir = TestDir::new("missing-current");
    let current = dir.0.join("이미-사라진.exe");
    let staged = dir.file("anemone_rs.exe.new", "신버전");

    let error = replace_running_executable(&current, &staged)
        .expect_err("현재 실행 파일이 없으면 실패해야 한다");

    assert!(
        matches!(error, UpdateError::Io(_)),
        "RollbackFailed가 아닌 단순 Io 오류여야 한다: {error}"
    );
    assert!(
        staged.exists(),
        "1단계 실패 시 staged 파일은 건드리지 않아야 한다"
    );
}

#[test]
fn a_writable_directory_is_detected_and_leaves_no_probe_file() {
    let dir = TestDir::new("probe");

    assert!(can_write_dir(dir.path()));
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "권한 확인용 임시 파일이 남으면 안 된다"
    );
}

#[test]
fn a_missing_directory_is_not_writable() {
    let dir = TestDir::new("missing");
    assert!(!can_write_dir(&dir.0.join("존재하지-않는-폴더")));
}

#[test]
fn the_backup_keeps_the_executable_name() {
    let backup = backup_path(Path::new(r"D:\Apps\Anemone\anemone_rs.exe"));
    assert_eq!(backup, Path::new(r"D:\Apps\Anemone\anemone_rs.exe.old"));
}

// --- is_transient: 재시도 가능한 오류 판별 ----------------------------------

#[test]
fn known_transient_os_error_codes_are_retried() {
    for code in [5, 32, 33] {
        assert!(
            is_transient(&io::Error::from_raw_os_error(code)),
            "OS 오류 코드 {code}는 재시도 대상이어야 한다"
        );
    }
}

#[test]
fn unrelated_os_error_codes_are_not_retried() {
    // 2 = ERROR_FILE_NOT_FOUND. 잠금 문제가 아니므로 재시도해도 소용없다.
    assert!(!is_transient(&io::Error::from_raw_os_error(2)));
}

/// `atomic_replace`가 `MoveFileExW` 실패를 `io::Error::other`로 감싸면
/// `raw_os_error()`가 비어, 메시지에서 HRESULT 16진 코드를 직접 찾아야 한다.
#[test]
fn transient_hresult_codes_embedded_in_the_message_are_detected() {
    for hresult in ["0x80070005", "0x80070020", "0x80070021"] {
        let error = io::Error::other(format!("MoveFileExW failed: {hresult}"));
        assert!(
            is_transient(&error),
            "{hresult}가 포함된 메시지는 재시도 대상이어야 한다"
        );
    }
}

#[test]
fn messages_without_a_known_hresult_are_not_retried() {
    let error = io::Error::other("MoveFileExW failed: 0x80004005");
    assert!(!is_transient(&error));
}

// --- cleanup_backup: 이전 실행에서 남은 백업 정리 ---------------------------
//
// `cleanup_backup`은 `current_exe()` 옆 고정 경로만 다루므로, 두 시나리오
// (있음/없음)를 같은 테스트 함수 안에서 순서대로 검증한다 — 별도 테스트로
// 나누면 병렬 실행 중인 다른 테스트와 같은 경로를 두고 경합할 수 있다.
#[test]
fn cleanup_backup_removes_a_leftover_backup_and_is_a_no_op_once_gone() {
    let current = std::env::current_exe().expect("테스트 실행 파일 경로를 얻을 수 있어야 한다");
    let backup = backup_path(&current);
    // 실제 실행 파일 옆에 더미 백업을 만들어 정리 대상을 준비한다. 진짜 실행
    // 파일 자체는 건드리지 않는다.
    std::fs::write(&backup, "stale backup").expect("더미 백업을 쓸 수 있어야 한다");
    assert!(backup.exists());

    cleanup_backup();
    assert!(!backup.exists(), "정리 후에는 백업이 남아 있으면 안 된다");

    // NotFound 분기가 패닉하지 않고 조용히 넘어가는지 이어서 확인한다.
    cleanup_backup();
}
