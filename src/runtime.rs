use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug)]
pub(crate) struct AppPaths {
    data_dir: PathBuf,
}

impl AppPaths {
    fn discover() -> Result<Self, AppPathsError> {
        let executable = std::env::current_exe().map_err(AppPathsError::Executable)?;
        let executable_dir = executable
            .parent()
            .ok_or(AppPathsError::ExecutableHasNoParent)?;
        Ok(Self::for_executable_dir(executable_dir))
    }

    fn for_executable_dir(executable_dir: &Path) -> Self {
        Self {
            data_dir: executable_dir.to_path_buf(),
        }
    }

    pub(crate) fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub(crate) fn config_file(&self) -> PathBuf {
        self.data_dir.join("config.toml")
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppPathsError {
    #[error("실행 파일 경로를 확인할 수 없습니다: {0}")]
    Executable(io::Error),
    #[error("실행 파일 경로에 부모 디렉터리가 없습니다")]
    ExecutableHasNoParent,
    #[error("데이터 디렉터리를 준비할 수 없습니다: {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
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

pub fn cache_db_file() -> PathBuf {
    data_file("translation_cache.sqlite3")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_data_stays_next_to_executable() {
        let executable = Path::new(r"D:\Apps\Anemone");
        let paths = AppPaths::for_executable_dir(executable);

        assert_eq!(paths.data_dir(), executable);
        assert_eq!(paths.config_file(), executable.join("config.toml"));
        assert_eq!(paths.data_dir().join("logs"), executable.join("logs"));
    }
}
