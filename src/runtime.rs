use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use directories::BaseDirs;

const APP_DIRECTORY: &str = "Anemone";
const PORTABLE_MARKER: &str = "anemone.portable";

pub fn data_dir() -> &'static PathBuf {
    static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
    DATA_DIR.get_or_init(|| {
        let exe_dir = executable_dir().unwrap_or_else(|| PathBuf::from("."));
        let local_data_dir = BaseDirs::new().map(|dirs| dirs.data_local_dir().to_path_buf());
        choose_data_dir(&exe_dir, local_data_dir, std::env::temp_dir)
    })
}

pub fn data_file(name: &str) -> PathBuf {
    let destination = data_dir().join(name);
    migrate_legacy_file(name, &destination);
    destination
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

pub fn portable_mode() -> bool {
    executable_dir()
        .map(|directory| directory.join(PORTABLE_MARKER).is_file())
        .unwrap_or(false)
}

fn choose_data_dir(
    exe_dir: &Path,
    local_data_dir: Option<PathBuf>,
    fallback: impl FnOnce() -> PathBuf,
) -> PathBuf {
    if exe_dir.join(PORTABLE_MARKER).is_file() {
        return exe_dir.to_path_buf();
    }
    local_data_dir.unwrap_or_else(fallback).join(APP_DIRECTORY)
}

fn executable_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

/// 이전 버전이 실행 파일 옆에 저장한 파일은 목적지에 파일이 없을 때만 복사한다.
/// 원본은 사용자가 확인할 수 있도록 보존하고, 목적지 생성은 원자적으로 수행한다.
fn migrate_legacy_file(name: &str, destination: &Path) {
    if destination.exists() || portable_mode() {
        return;
    }
    let Some(source) = executable_dir().map(|directory| directory.join(name)) else {
        return;
    };
    if !source.is_file() || source == destination {
        return;
    }

    match copy_legacy_file(&source, destination) {
        Ok(()) => tracing::info!(
            "legacy runtime data migrated: {} -> {}",
            source.display(),
            destination.display()
        ),
        Err(error) => tracing::warn!(
            "legacy runtime data migration failed for {}: {error}",
            source.display()
        ),
    }
}

fn copy_legacy_file(source: &Path, destination: &Path) -> io::Result<()> {
    let contents = std::fs::read(source)?;
    crate::fs_util::atomic_write(destination, &contents)
}

pub fn ensure_data_directories() -> io::Result<()> {
    std::fs::create_dir_all(data_dir())?;
    std::fs::create_dir_all(logs_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_mode_prefers_local_app_data() {
        let exe = PathBuf::from(r"C:\Program Files\Anemone");
        let local = PathBuf::from(r"C:\Users\tester\AppData\Local");
        assert_eq!(
            choose_data_dir(&exe, Some(local.clone()), || panic!("fallback used")),
            local.join(APP_DIRECTORY)
        );
    }

    #[test]
    fn missing_known_folder_uses_temporary_directory_fallback() {
        let exe = PathBuf::from(r"C:\Program Files\Anemone");
        let fallback = PathBuf::from(r"C:\Temp");
        assert_eq!(
            choose_data_dir(&exe, None, || fallback.clone()),
            fallback.join(APP_DIRECTORY)
        );
    }

    #[test]
    fn portable_marker_explicitly_selects_executable_directory() {
        let root = std::env::temp_dir().join(format!(
            "anemone-portable-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(PORTABLE_MARKER), []).unwrap();
        assert_eq!(
            choose_data_dir(&root, Some(PathBuf::from(r"C:\Local")), || {
                panic!("fallback used")
            }),
            root
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_migration_preserves_source_and_creates_complete_destination() {
        let root = std::env::temp_dir().join(format!(
            "anemone-migration-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("install").join("config.toml");
        let destination = root.join("data").join("config.toml");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "complete-config").unwrap();

        copy_legacy_file(&source, &destination).unwrap();

        assert_eq!(std::fs::read_to_string(&source).unwrap(), "complete-config");
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "complete-config"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
