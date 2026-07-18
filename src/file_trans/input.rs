use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::FileTranslationError;

/// UTF-8 검증과 파일별 줄 수 계산을 결합한 사전 검사.
pub(super) fn preflight_inputs(
    files: &[PathBuf],
    cancel_token: &AtomicBool,
) -> Result<Vec<usize>, FileTranslationError> {
    let mut counts = Vec::with_capacity(files.len());
    for path in files {
        let reader =
            crate::util::open_utf8_translation_input(path).map_err(FileTranslationError::input)?;
        counts.push(validate_and_count_reader(reader, path, cancel_token)?);
    }
    Ok(counts)
}

pub fn validate_and_count_reader<R: Read>(
    mut reader: R,
    path: &Path,
    cancel_token: &AtomicBool,
) -> Result<usize, FileTranslationError> {
    const CHUNK_SIZE: usize = 64 * 1024;
    let mut buffer = [0u8; CHUNK_SIZE];
    let mut pending = Vec::with_capacity(4);
    let mut total_body_bytes = 0usize;
    let mut newline_count = 0usize;
    let mut last_was_newline = false;
    let mut first_chunk = true;

    loop {
        if cancel_token.load(Ordering::SeqCst) {
            return Err(FileTranslationError::Cancelled);
        }
        let read = reader.read(&mut buffer).map_err(|e| {
            FileTranslationError::input(format!(
                "입력 파일을 읽을 수 없습니다: {}\n{e}",
                path.display()
            ))
        })?;
        if read == 0 {
            break;
        }
        let mut chunk = &buffer[..read];
        if first_chunk {
            first_chunk = false;
            chunk = chunk.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(chunk);
        }
        total_body_bytes = total_body_bytes.saturating_add(chunk.len());
        newline_count = newline_count.saturating_add(chunk.iter().filter(|&&b| b == b'\n').count());
        if let Some(&last) = chunk.last() {
            last_was_newline = last == b'\n';
        }

        pending.extend_from_slice(chunk);
        match std::str::from_utf8(&pending) {
            Ok(_) => pending.clear(),
            Err(error) if error.error_len().is_none() => {
                let tail = pending.split_off(error.valid_up_to());
                pending = tail;
            }
            Err(error) => {
                let byte = total_body_bytes
                    .saturating_sub(pending.len())
                    .saturating_add(error.valid_up_to());
                return Err(FileTranslationError::encoding(format!(
                    "UTF-8 디코딩 실패(byte {byte}): UTF-8 또는 UTF-8 BOM 파일만 사용할 수 있습니다. ({})",
                    path.display()
                )));
            }
        }
    }
    if !pending.is_empty() {
        return Err(FileTranslationError::encoding(format!(
            "UTF-8 디코딩 실패(byte {}): UTF-8 또는 UTF-8 BOM 파일만 사용할 수 있습니다. ({})",
            total_body_bytes.saturating_sub(pending.len()),
            path.display()
        )));
    }
    Ok(newline_count + usize::from(total_body_bytes > 0 && !last_was_newline))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    None,
    Lf,
    CrLf,
}

impl LineEnding {
    pub(super) fn bytes(self) -> &'static [u8] {
        match self {
            Self::None => b"",
            Self::Lf => b"\n",
            Self::CrLf => b"\r\n",
        }
    }

    pub(super) fn separator(self) -> &'static [u8] {
        match self {
            Self::CrLf => b"\r\n",
            Self::None | Self::Lf => b"\n",
        }
    }
}

pub struct InputLine {
    pub text: String,
    pub ending: LineEnding,
}

pub fn read_input_line<R: BufRead>(
    reader: &mut R,
    path: &Path,
    first_line: bool,
) -> Result<Option<InputLine>, FileTranslationError> {
    const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|e| {
            FileTranslationError::input(format!(
                "입력 파일을 읽을 수 없습니다: {}\n{e}",
                path.display()
            ))
        })?;
        if available.is_empty() {
            break;
        }
        let take = available
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(take) > MAX_LINE_BYTES {
            return Err(FileTranslationError::LineTooLong {
                path: path.to_path_buf(),
                limit: MAX_LINE_BYTES,
            });
        }
        let found_newline = available[take - 1] == b'\n';
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if found_newline {
            break;
        }
    }
    if bytes.is_empty() {
        return Ok(None);
    }

    let ending = if bytes.ends_with(b"\r\n") {
        bytes.truncate(bytes.len() - 2);
        LineEnding::CrLf
    } else if bytes.ends_with(b"\n") {
        bytes.pop();
        LineEnding::Lf
    } else {
        LineEnding::None
    };
    if first_line && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        bytes.drain(..3);
    }
    if first_line && bytes.is_empty() && ending == LineEnding::None {
        return Ok(None);
    }
    let text = String::from_utf8(bytes).map_err(|error| {
        FileTranslationError::encoding(format!(
            "UTF-8 디코딩 실패: UTF-8 또는 UTF-8 BOM 파일만 사용할 수 있습니다. ({})\n{error}",
            path.display()
        ))
    })?;
    Ok(Some(InputLine { text, ending }))
}
