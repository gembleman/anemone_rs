use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const APP_DIRECTORY: &str = "Anemone";
const PORTABLE_MARKER: &str = "portable.flag";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppMode {
    Installed,
    Portable,
}

#[derive(Debug)]
pub(crate) struct AppPaths {
    mode: AppMode,
    executable_dir: PathBuf,
    data_dir: PathBuf,
}

impl AppPaths {
    fn discover() -> Result<Self, AppPathsError> {
        let executable = std::env::current_exe().map_err(AppPathsError::Executable)?;
        let executable_dir = executable
            .parent()
            .ok_or(AppPathsError::ExecutableHasNoParent)?;
        let marker = executable_dir.join(PORTABLE_MARKER);
        let portable = match std::fs::metadata(&marker) {
            Ok(metadata) => metadata.is_file(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(source) => return Err(AppPathsError::InspectPortableMarker { marker, source }),
        };
        Self::resolve(
            executable_dir,
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
            portable,
        )
    }

    fn resolve(
        executable_dir: &Path,
        local_app_data: Option<PathBuf>,
        portable: bool,
    ) -> Result<Self, AppPathsError> {
        let executable_dir = executable_dir.to_path_buf();
        if portable {
            return Ok(Self {
                mode: AppMode::Portable,
                data_dir: executable_dir.clone(),
                executable_dir,
            });
        }

        let local_app_data = local_app_data
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or(AppPathsError::LocalAppDataUnavailable)?;
        Ok(Self {
            mode: AppMode::Installed,
            executable_dir,
            data_dir: local_app_data.join(APP_DIRECTORY),
        })
    }

    pub(crate) fn mode(&self) -> AppMode {
        self.mode
    }

    pub(crate) fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub(crate) fn config_file(&self) -> PathBuf {
        self.data_dir.join("config.toml")
    }

    pub(crate) fn secrets_file(&self) -> PathBuf {
        self.data_dir.join("secrets.dat")
    }

    pub(crate) fn legacy_file(&self, name: &str) -> Option<PathBuf> {
        (self.mode == AppMode::Installed).then(|| self.executable_dir.join(name))
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppPathsError {
    #[error("실행 파일 경로를 확인할 수 없습니다: {0}")]
    Executable(io::Error),
    #[error("실행 파일 경로에 부모 디렉터리가 없습니다")]
    ExecutableHasNoParent,
    #[error("LOCALAPPDATA를 확인할 수 없습니다; portable.flag를 사용하거나 환경을 복구하세요")]
    LocalAppDataUnavailable,
    #[error("portable marker를 확인할 수 없습니다: {marker}: {source}")]
    InspectPortableMarker {
        marker: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("데이터 디렉터리를 준비할 수 없습니다: {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("기존 데이터 파일을 이전할 수 없습니다: {source_path} -> {target_path}: {source}")]
    Migrate {
        source_path: PathBuf,
        target_path: PathBuf,
        #[source]
        source: io::Error,
    },
}

static APP_PATHS: OnceLock<AppPaths> = OnceLock::new();

/// 프로세스 시작 시 경로 정책을 확정하고 필요한 디렉터리/기존 데이터를 준비한다.
pub(crate) fn initialize() -> Result<&'static AppPaths, AppPathsError> {
    let paths = if let Some(paths) = APP_PATHS.get() {
        paths
    } else {
        let discovered = AppPaths::discover()?;
        let _ = APP_PATHS.set(discovered);
        APP_PATHS.get().expect("AppPaths was initialized")
    };
    ensure_directories(paths)?;
    migrate_legacy_file(paths, "config.toml")?;
    migrate_legacy_file(paths, "llm_usage.json")?;
    Ok(paths)
}

pub(crate) fn paths() -> &'static AppPaths {
    initialize().expect("AppPaths must be initialized during process bootstrap")
}

pub fn data_dir() -> &'static PathBuf {
    static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
    DATA_DIR.get_or_init(|| paths().data_dir().to_path_buf())
}

pub fn data_file(name: &str) -> PathBuf {
    data_dir().join(name)
}

pub(crate) fn is_portable() -> bool {
    paths().mode() == AppMode::Portable
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

pub fn ensure_data_directories() -> io::Result<()> {
    initialize().map(|_| ()).map_err(io::Error::other)
}

fn ensure_directories(paths: &AppPaths) -> Result<(), AppPathsError> {
    for path in [
        paths.data_dir().to_path_buf(),
        paths.data_dir().join("logs"),
    ] {
        std::fs::create_dir_all(&path)
            .map_err(|source| AppPathsError::CreateDirectory { path, source })?;
    }
    Ok(())
}

fn migrate_legacy_file(paths: &AppPaths, name: &str) -> Result<(), AppPathsError> {
    let target = paths.data_dir().join(name);
    if target.exists() {
        return Ok(());
    }
    let Some(source_path) = paths.legacy_file(name) else {
        return Ok(());
    };
    if !source_path.is_file() {
        return Ok(());
    }
    std::fs::copy(&source_path, &target).map_err(|source| AppPathsError::Migrate {
        source_path,
        target_path: target,
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_mode_uses_local_app_data() {
        let paths = AppPaths::resolve(
            Path::new(r"C:\Program Files\Anemone"),
            Some(PathBuf::from(r"C:\Users\tester\AppData\Local")),
            false,
        )
        .unwrap();

        assert_eq!(paths.mode(), AppMode::Installed);
        assert_eq!(
            paths.data_dir(),
            Path::new(r"C:\Users\tester\AppData\Local\Anemone")
        );
    }

    #[test]
    fn portable_marker_keeps_data_next_to_executable() {
        let executable = Path::new(r"D:\Portable\Anemone");
        let paths = AppPaths::resolve(executable, None, true).unwrap();

        assert_eq!(paths.mode(), AppMode::Portable);
        assert_eq!(paths.data_dir(), executable);
        assert_eq!(paths.config_file(), executable.join("config.toml"));
    }

    #[test]
    fn installed_mode_never_falls_back_when_local_app_data_is_missing() {
        let error = AppPaths::resolve(Path::new(r"C:\Anemone"), None, false).unwrap_err();
        assert!(matches!(error, AppPathsError::LocalAppDataUnavailable));
    }

    #[test]
    fn installed_bootstrap_copies_legacy_data_without_overwriting_target() {
        let root = std::env::temp_dir().join(format!(
            "anemone-paths-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let executable = root.join("portable-old");
        let local = root.join("local");
        std::fs::create_dir_all(&executable).unwrap();
        std::fs::write(executable.join("config.toml"), b"legacy").unwrap();
        let paths = AppPaths::resolve(&executable, Some(local), false).unwrap();

        ensure_directories(&paths).unwrap();
        migrate_legacy_file(&paths, "config.toml").unwrap();
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), b"legacy");

        std::fs::write(paths.config_file(), b"installed").unwrap();
        std::fs::write(executable.join("config.toml"), b"changed legacy").unwrap();
        migrate_legacy_file(&paths, "config.toml").unwrap();
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), b"installed");
        std::fs::remove_dir_all(root).unwrap();
    }
}
