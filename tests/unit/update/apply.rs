use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::super::UpdateError;
use super::{backup_path, can_write_dir, replace_running_executable};

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
