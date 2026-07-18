use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::input::LineEnding;
use super::{FileTranslationError, WriteType};

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 성공 시에만 최종 경로로 교체되는 임시 출력 파일.
pub struct PendingOutput {
    final_path: PathBuf,
    temp_path: PathBuf,
    writer: Option<BufWriter<File>>,
}

impl PendingOutput {
    pub fn create(final_path: &Path) -> Result<Self, FileTranslationError> {
        let parent = final_path.parent().unwrap_or(Path::new(""));
        let name = final_path.file_name().unwrap_or_default().to_string_lossy();

        for _ in 0..100 {
            let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let temp_path = parent.join(format!(
                ".{name}.anemone-{}-{sequence}.tmp",
                std::process::id()
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
            {
                Ok(file) => {
                    return Ok(Self {
                        final_path: final_path.to_path_buf(),
                        temp_path,
                        writer: Some(BufWriter::new(file)),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(FileTranslationError::output(format!(
                        "임시 출력 파일을 생성할 수 없습니다: {}\n{error}",
                        temp_path.display()
                    )));
                }
            }
        }

        Err(FileTranslationError::output(format!(
            "고유한 임시 출력 파일을 생성할 수 없습니다: {}",
            final_path.display()
        )))
    }

    pub fn writer(&mut self) -> &mut BufWriter<File> {
        self.writer.as_mut().expect("writer exists until persist")
    }

    #[cfg(test)]
    pub fn temp_path(&self) -> &Path {
        &self.temp_path
    }

    pub fn persist(mut self) -> Result<(), FileTranslationError> {
        let mut writer = self.writer.take().expect("writer exists until persist");
        writer.flush().map_err(|error| {
            FileTranslationError::output(format!(
                "임시 출력 파일을 저장할 수 없습니다: {}\n{error}",
                self.temp_path.display()
            ))
        })?;
        writer.get_ref().sync_all().map_err(|error| {
            FileTranslationError::output(format!(
                "임시 출력 파일을 디스크에 반영할 수 없습니다: {}\n{error}",
                self.temp_path.display()
            ))
        })?;
        drop(writer);

        crate::fs_util::atomic_replace(&self.temp_path, &self.final_path).map_err(|error| {
            FileTranslationError::output(format!(
                "완성된 출력 파일을 최종 경로로 옮길 수 없습니다: {}\n{error}",
                self.final_path.display()
            ))
        })?;

        self.temp_path.clear();
        Ok(())
    }
}

impl Drop for PendingOutput {
    fn drop(&mut self) {
        if !self.temp_path.as_os_str().is_empty()
            && let Err(error) = std::fs::remove_file(&self.temp_path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                "임시 출력 파일 삭제 실패 ({}): {error}",
                self.temp_path.display()
            );
        }
    }
}

/// 출력 형식에 따라 쓰기.
pub fn write_output<W: Write>(
    writer: &mut W,
    original: &str,
    translated: &str,
    write_type: WriteType,
    ending: LineEnding,
    has_more: bool,
) -> Result<(), FileTranslationError> {
    let write = |result: std::io::Result<()>| {
        result.map_err(|error| FileTranslationError::output(error.to_string()))
    };
    let separator = ending.separator();
    match write_type {
        WriteType::TranslationOnly => {
            write(writer.write_all(translated.as_bytes()))?;
            write(writer.write_all(ending.bytes()))?;
        }
        WriteType::OriginalAndTrans => {
            write(writer.write_all(original.as_bytes()))?;
            write(writer.write_all(separator))?;
            write(writer.write_all(translated.as_bytes()))?;
            write(writer.write_all(ending.bytes()))?;
        }
        WriteType::OriginalTransNewline => {
            write(writer.write_all(original.as_bytes()))?;
            write(writer.write_all(separator))?;
            write(writer.write_all(translated.as_bytes()))?;
            if has_more {
                write(writer.write_all(separator))?;
                write(writer.write_all(separator))?;
            } else {
                write(writer.write_all(ending.bytes()))?;
            }
        }
    }

    Ok(())
}
