use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

pub fn data_dir() -> &'static PathBuf {
    static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
    DATA_DIR.get_or_init(executable_dir)
}

pub fn data_file(name: &str) -> PathBuf {
    data_dir().join(name)
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

fn executable_dir() -> PathBuf {
    std::env::current_exe()
        .expect("failed to determine the executable path")
        .parent()
        .expect("executable path has no parent directory")
        .to_path_buf()
}

pub fn ensure_data_directories() -> io::Result<()> {
    std::fs::create_dir_all(data_dir())?;
    std::fs::create_dir_all(logs_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_files_are_created_next_to_the_executable() {
        let expected = std::env::current_exe().unwrap();
        let expected = expected.parent().unwrap();

        assert_eq!(data_dir(), expected);
        assert_eq!(data_file("config.toml"), expected.join("config.toml"));
        assert_eq!(logs_dir(), expected.join("logs"));
    }
}
