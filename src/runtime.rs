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
    let paths = match APP_PATHS.get() {
        Some(paths) => paths,
        None => {
            let discovered = AppPaths::discover()?;
            // 동시에 여러 스레드가 처음 호출해도 `get_or_init`이 하나만
            // 저장하고 나머지는 그 값을 돌려주므로, `set` 실패 후 `get`이
            // 비어 있을 수 있다는 가정(그리고 그로 인한 `unreachable!`)이
            // 필요 없어진다.
            APP_PATHS.get_or_init(|| discovered)
        }
    };
    ensure_directories(paths)?;
    Ok(paths)
}

pub(crate) fn paths() -> &'static AppPaths {
    match initialize() {
        Ok(paths) => paths,
        Err(error) => {
            // `paths()`는 프로세스 부트스트랩(`lib.rs::run()`)에서
            // `runtime::initialize()`가 이미 한 번 성공했다는 전제로 호출된다.
            // 그 이후에 실패한다면 데이터 디렉터리가 사라졌거나 손상된
            // 것이므로, 패닉으로 스택을 풀어내리는 대신 부트스트랩 최초
            // 실패와 동일한 방식(lib.rs::run() 참고)으로 즉시 종료한다.
            // 이 함수의 반환 타입을 `Result`로 바꿔 오류를 전파하려면
            // 호출부인 `config/app.rs::default_config_path()`도 함께 고쳐야
            // 하는데, 그 파일은 이 리팩토링 범위 밖이라 손대지 않았다.
            eprintln!("Anemone 데이터 경로를 사용할 수 없습니다: {error}");
            std::process::exit(1);
        }
    }
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
#[path = "../tests/unit/runtime.rs"]
mod tests;
