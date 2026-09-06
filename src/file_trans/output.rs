use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
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

    #[cfg(test)]
    pub fn temp_path(&self) -> &Path {
        &self.temp_path
    }

    pub fn persist(mut self) -> Result<(), FileTranslationError> {
        // `writer`는 이 메서드가 self를 소비할 때만 비워지므로, `persist`가 두 번
        // 불릴 수 없는 이상 항상 Some이다. 그래도 패닉 대신 오류로 처리해 타입이
        // 이 불변식을 강제하지 못하는 경우에도 복구 가능하게 한다.
        let mut writer = self.writer.take().ok_or_else(|| {
            FileTranslationError::output(format!(
                "임시 출력 파일 writer가 이미 저장(persist)되었습니다: {}",
                self.temp_path.display()
            ))
        })?;
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

/// `write_output` 등 호출부가 내부 `Option<BufWriter<File>>`을 직접 벗기지
/// 않고도 쓸 수 있도록 `Write`를 위임한다. `persist` 이후(즉 writer가 이미
/// 저장을 마친 뒤)에 호출되면 패닉 대신 오류를 반환한다 — `PendingOutput`은
/// `Drop`을 구현하므로 `writer` 필드를 부분 이동시킬 수 없어, 접근자가
/// `Option`을 노출하지 않는 쪽으로 불변식을 옮겼다.
impl Write for PendingOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.writer.as_mut() {
            Some(writer) => writer.write(buf),
            None => Err(io::Error::other(
                "임시 출력 파일 writer가 이미 저장(persist)되어 더 이상 쓸 수 없습니다",
            )),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.writer.as_mut() {
            Some(writer) => writer.flush(),
            None => Err(io::Error::other(
                "임시 출력 파일 writer가 이미 저장(persist)되어 더 이상 쓸 수 없습니다",
            )),
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

#[cfg(test)]
#[path = "../../tests/unit/file_trans/output.rs"]
mod tests;
