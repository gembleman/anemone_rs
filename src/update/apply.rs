//! 실행 중인 exe를 새 버전으로 교체한다.
//!
//! Windows에서 실행 중인 이미지는 삭제하거나 덮어쓸 수 없지만 **rename은 된다**.
//! 현재 exe를 `.old`로 밀어내고 그 자리에 새 파일을 놓는 방식이다.
//!
//! 이 모듈은 실패 경로가 본체다. 교체 도중 실패하면 앱이 통째로 사라질 수
//! 있으므로 모든 단계에 복구를 붙인다.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::UpdateError;
use crate::fs_util::atomic_replace;

/// 교체 후 남는 이전 버전의 확장자.
const BACKUP_EXTENSION: &str = "exe.old";

/// 백신 실시간 검사나 셸 확장이 파일을 잠깐 잡고 있을 때를 위한 재시도.
///
/// Defender는 새로 쓰인 `.exe`를 스캔하는 동안 핸들을 유지하고, 탐색기의
/// 썸네일·미리보기 핸들러도 간헐적으로 같은 증상을 만든다. 몇백 ms면 풀린다.
const REPLACE_RETRIES: usize = 5;
const REPLACE_RETRY_DELAY: Duration = Duration::from_millis(50);

/// 교체를 끝낸 뒤 호출자가 해야 할 일.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// 새 exe가 제자리에 놓였다. 재시작하면 새 버전으로 뜬다.
    ReadyToRestart,
}

/// 실행 파일이 있는 디렉터리에 쓸 수 있는지 실제로 만들어 보고 판단한다.
///
/// Windows에서 `metadata().permissions().readonly()`는 디렉터리 ACL을 반영하지
/// 않으므로 신뢰할 수 없다. Program Files 아래나 읽기 전용 미디어에 압축을 푼
/// 경우를 걸러내려면 직접 시도하는 수밖에 없다.
pub fn can_write_dir(dir: &Path) -> bool {
    let probe = dir.join(format!(".anemone-write-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 현재 실행 파일을 `staged`로 교체한다.
///
/// 성공하면 이전 버전이 `<이름>.exe.old`로 남는다. 실행 중인 이미지라 지금은
/// 지울 수 없고, 다음 실행에서 [`cleanup_backup`]이 치운다.
pub fn replace_running_executable(
    current_exe: &Path,
    staged: &Path,
) -> Result<Applied, UpdateError> {
    let directory = current_exe
        .parent()
        .ok_or_else(|| UpdateError::Io(io::Error::other("실행 파일 경로에 부모가 없습니다")))?;
    if !can_write_dir(directory) {
        return Err(UpdateError::NotWritable(directory.to_path_buf()));
    }

    let backup = backup_path(current_exe);

    // 지난 교체에서 남은 백업이 있으면 먼저 치운다. 실패해도 다음 단계의
    // MOVEFILE_REPLACE_EXISTING이 덮어쓰므로 계속 진행한다.
    if let Err(error) = std::fs::remove_file(&backup)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::debug!("이전 백업을 미리 지우지 못했습니다(계속 진행): {error}");
    }

    // 1단계: 실행 중인 exe를 백업 이름으로 밀어낸다.
    replace_with_retry(current_exe, &backup).map_err(|error| {
        tracing::error!("현재 실행 파일을 백업으로 옮기지 못했습니다: {error}");
        UpdateError::Io(error)
    })?;

    // 2단계: 새 파일을 원래 자리에 놓는다. 여기서 실패하면 exe가 없는 상태이므로
    // 반드시 되돌린다.
    if let Err(error) = replace_with_retry(staged, current_exe) {
        tracing::error!("새 실행 파일을 배치하지 못했습니다: {error}");
        return Err(rollback(&backup, current_exe, error));
    }

    Ok(Applied::ReadyToRestart)
}

/// 새 파일 배치에 실패했을 때 백업을 제자리로 되돌린다.
///
/// 되돌리기까지 실패하면 실행 파일이 사라진 상태다. 로그만으로는 사용자가
/// 복구할 수 없으므로 백업 경로를 직접 알려준다.
fn rollback(backup: &Path, current_exe: &Path, cause: io::Error) -> UpdateError {
    match replace_with_retry(backup, current_exe) {
        Ok(()) => {
            tracing::info!("이전 실행 파일로 되돌렸습니다.");
            UpdateError::Io(cause)
        }
        Err(rollback_error) => {
            tracing::error!(
                "롤백에 실패했습니다. 실행 파일이 {}에 남아 있습니다: {rollback_error}",
                backup.display()
            );
            UpdateError::RollbackFailed {
                backup: backup.to_path_buf(),
                cause: cause.to_string(),
            }
        }
    }
}

/// 파일 잠금이 풀릴 때까지 짧게 재시도하며 교체한다.
fn replace_with_retry(source: &Path, destination: &Path) -> io::Result<()> {
    let mut attempt = 0;
    loop {
        match atomic_replace(source, destination) {
            Ok(()) => return Ok(()),
            Err(error) if attempt < REPLACE_RETRIES && is_transient(&error) => {
                attempt += 1;
                tracing::debug!(
                    "파일이 잠겨 있어 재시도합니다 ({attempt}/{REPLACE_RETRIES}): {error}"
                );
                std::thread::sleep(REPLACE_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
}

/// 잠깐 기다리면 풀릴 수 있는 오류인지 판단한다.
///
/// `atomic_replace`는 `MoveFileExW` 실패를 `io::Error::other`로 감싸므로
/// `raw_os_error()`가 비어 있다. 메시지에서 Windows 오류 코드를 찾는다.
fn is_transient(error: &io::Error) -> bool {
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;

    if let Some(code) = error.raw_os_error() {
        return matches!(
            code,
            ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION
        );
    }
    // windows crate가 만드는 메시지에는 HRESULT가 16진으로 들어간다.
    // 0x80070005 / 0x80070020 / 0x80070021.
    let text = error.to_string().to_ascii_lowercase();
    ["0x80070005", "0x80070020", "0x80070021"]
        .iter()
        .any(|code| text.contains(code))
}

/// `<실행파일>.exe.old` 경로.
pub fn backup_path(current_exe: &Path) -> PathBuf {
    current_exe.with_extension(BACKUP_EXTENSION)
}

/// 이전 실행에서 남은 백업을 지운다.
///
/// **GUI 시작 경로에서 프로세스당 한 번만 호출한다.** `runtime::initialize()`에
/// 넣으면 안 된다. 그 함수는 데이터 경로를 얻을 때마다 다시 실행되고, EzTrans
/// helper 프로세스(`cli::run`의 숨은 서브커맨드)도 같은 경로를 지나므로 helper가
/// 뜰 때마다 삭제를 시도하게 된다.
///
/// 실패는 정상 상황이다. 직전 인스턴스나 그 helper가 아직 `.old` 이미지를
/// 실행 중이면 잠겨 있다. 다음 실행에서 다시 시도하면 된다.
pub fn cleanup_backup() {
    let Ok(current) = std::env::current_exe() else {
        return;
    };
    let backup = backup_path(&current);
    match std::fs::remove_file(&backup) {
        Ok(()) => tracing::info!("이전 버전 파일을 정리했습니다: {}", backup.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!("이전 버전 파일을 지우지 못했습니다(다음 실행에서 다시 시도): {error}");
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/update/apply.rs"]
mod tests;
