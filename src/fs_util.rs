use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 같은 디렉터리에 완전한 임시 파일을 만든 뒤 원자적으로 교체한다.
///
/// 데이터와 디렉터리를 가능한 범위에서 동기화하므로, 프로세스 종료나 전원
/// 장애가 저장 도중 발생해도 기존 파일 또는 새 파일 중 하나가 남는다.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let temp = unique_temp_path(path);
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);

        atomic_replace(&temp, path)?;
        if let Ok(directory) = OpenOptions::new().read(true).open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn unique_temp_path(path: &Path) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("anemone-data");
    path.with_file_name(format!(".{name}.tmp-{}-{sequence}", std::process::id()))
}

/// 같은 볼륨의 임시 파일을 목적지에 원자적으로 덮어쓰고 디스크 반영을 요청한다.
///
/// Windows의 기존 파일 overwrite 의미를 보존하기 위한 단일 플랫폼 경계다.
pub(crate) fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::HSTRING;

    let source = HSTRING::from(source.as_os_str());
    let destination = HSTRING::from(destination.as_os_str());
    unsafe {
        MoveFileExW(
            &source,
            &destination,
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(|error| io::Error::other(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_existing_file_without_temp_residue() {
        let root = std::env::temp_dir().join(format!(
            "anemone-atomic-write-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).expect("create test directory");
        let path = root.join("config.toml");
        fs::write(&path, "old").expect("seed old file");

        atomic_write(&path, b"new contents").expect("atomic replace");

        assert_eq!(fs::read_to_string(&path).unwrap(), "new contents");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).expect("remove test directory");
    }
}
