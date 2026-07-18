use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const LOG_FILE_BYTES: u64 = 1024 * 1024;
pub const LOG_FILE_COUNT: usize = 4;

pub struct RollingLogWriter {
    directory: PathBuf,
    file: Option<File>,
    length: u64,
}

impl RollingLogWriter {
    pub fn open(directory: &Path) -> io::Result<Self> {
        fs::create_dir_all(directory)?;
        let path = directory.join("anemone.log");
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let length = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        Ok(Self {
            directory: directory.to_path_buf(),
            file: Some(file),
            length,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.take();
        let oldest = self
            .directory
            .join(format!("anemone.log.{}", LOG_FILE_COUNT - 1));
        if oldest.exists() {
            fs::remove_file(oldest)?;
        }
        for generation in (1..LOG_FILE_COUNT - 1).rev() {
            let source = self.directory.join(format!("anemone.log.{generation}"));
            if source.exists() {
                fs::rename(
                    source,
                    self.directory
                        .join(format!("anemone.log.{}", generation + 1)),
                )?;
            }
        }
        let current = self.directory.join("anemone.log");
        if current.exists() {
            fs::rename(&current, self.directory.join("anemone.log.1"))?;
        }
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(current)?,
        );
        self.length = 0;
        Ok(())
    }
}

impl Write for RollingLogWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.length > 0 && self.length.saturating_add(buffer.len() as u64) > LOG_FILE_BYTES {
            self.rotate()?;
        }
        // 단일 이벤트가 상한보다 큰 경우에도 파일 상한을 지킨다. 애플리케이션
        // 이벤트는 원문을 기록하지 않으므로 정상적으로는 이 경로에 들어오지 않는다.
        let allowed = (LOG_FILE_BYTES - self.length).min(buffer.len() as u64) as usize;
        let written = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("log file is closed"))?
            .write(&buffer[..allowed])?;
        self.length += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("log file is closed"))?
            .flush()
    }
}

pub fn init() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    crate::runtime::ensure_data_directories()?;
    let writer = RollingLogWriter::open(&crate::runtime::logs_dir())?;
    tracing_subscriber::fmt()
        .with_max_level(configured_level())
        .with_target(false)
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(writer))
        .try_init()?;
    Ok(())
}

fn configured_level() -> tracing::Level {
    let default = if cfg!(debug_assertions) {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    match std::env::var("ANEMONE_LOG")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "warn" | "warning" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _ => default,
    }
}

pub fn report_init_failure(error: &dyn std::fmt::Display) {
    let message = format!("로그 초기화 실패(애플리케이션은 계속 실행): {error}");
    eprintln!("{message}");
    use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
    use windows::core::HSTRING;
    unsafe { OutputDebugStringW(&HSTRING::from(message)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_writer_bounds_total_log_storage() {
        let directory = std::env::temp_dir().join(format!(
            "anemone-log-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&directory);
        let mut writer = RollingLogWriter::open(&directory).unwrap();
        let block = vec![b'x'; 300 * 1024];
        for _ in 0..20 {
            writer.write_all(&block).unwrap();
        }
        writer.flush().unwrap();
        drop(writer);

        let files: Vec<_> = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect();
        let total: u64 = files
            .iter()
            .map(|entry| entry.metadata().unwrap().len())
            .sum();
        assert!(files.len() <= LOG_FILE_COUNT);
        assert!(total <= LOG_FILE_BYTES * LOG_FILE_COUNT as u64);
        fs::remove_dir_all(directory).unwrap();
    }
}
