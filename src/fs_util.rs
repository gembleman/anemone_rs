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

    let (mut file, temp) = create_unique_temp(path)?;
    let result = (|| {
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

fn create_unique_temp(path: &Path) -> io::Result<(fs::File, PathBuf)> {
    const MAX_ATTEMPTS: usize = 100;
    create_unique_temp_from(std::iter::repeat_with(|| unique_temp_path(path)).take(MAX_ATTEMPTS))
}

fn create_unique_temp_from(
    candidates: impl IntoIterator<Item = PathBuf>,
) -> io::Result<(fs::File, PathBuf)> {
    for temp in candidates {
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => return Ok((file, temp)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "고유한 원자 저장 임시 파일명을 할당할 수 없습니다",
    ))
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
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain([0]).collect();
    let destination: Vec<u16> = destination.as_os_str().encode_wide().chain([0]).collect();
    let succeeded = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/fs_util.rs"]
mod tests;
