use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::FileTranslationError;

/// UTF-8/UTF-8 BOM 입력을 스트리밍으로 읽을 reader를 연다. 인코딩 전체 검증은
/// 소비자가 읽는 동안 수행하며, 여기서는 잘못된 BOM을 먼저 거른다.
pub(super) fn open_utf8_translation_input(path: &Path) -> Result<BufReader<File>, String> {
    let file = File::open(path)
        .map_err(|e| format!("입력 파일을 열 수 없습니다: {} ({e})", path.display()))?;
    let mut reader = BufReader::new(file);
    let prefix = reader
        .fill_buf()
        .map_err(|e| format!("입력 파일을 읽을 수 없습니다: {} ({e})", path.display()))?;
    if let Some(name) = detect_non_utf8_bom(prefix) {
        return Err(format!(
            "지원하지 않는 인코딩입니다: {name}. UTF-8 또는 UTF-8 BOM 파일만 사용할 수 있습니다. ({})",
            path.display()
        ));
    }
    Ok(reader)
}

/// 미리보기는 파일 크기와 무관한 고정 바이트 상한 안에서만 읽는다.
pub(crate) fn read_utf8_preview(
    path: &Path,
    max_lines: usize,
    max_bytes: u64,
) -> Result<String, String> {
    let reader = open_utf8_translation_input(path)?;
    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024) as usize);
    reader
        .take(max_bytes)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("미리보기를 읽을 수 없습니다: {e}"))?;
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    let valid = match std::str::from_utf8(body) {
        Ok(text) => text,
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&body[..error.valid_up_to()]).unwrap_or_default()
        }
        Err(error) => {
            return Err(format!(
                "UTF-8 디코딩 실패(byte {}): UTF-8 또는 UTF-8 BOM 파일만 사용할 수 있습니다. ({})",
                error.valid_up_to(),
                path.display()
            ));
        }
    };
    Ok(valid
        .lines()
        .take(max_lines)
        .collect::<Vec<_>>()
        .join("\r\n"))
}

fn detect_non_utf8_bom(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x00, 0x00, 0xFE, 0xFF]) {
        Some("UTF-32 BE")
    } else if bytes.starts_with(&[0xFF, 0xFE, 0x00, 0x00]) {
        Some("UTF-32 LE")
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Some("UTF-16 BE")
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        Some("UTF-16 LE")
    } else {
        None
    }
}

/// UTF-8 검증과 파일별 줄 수 계산을 결합한 사전 검사.
pub(super) fn preflight_inputs(
    files: &[PathBuf],
    cancel_token: &AtomicBool,
) -> Result<Vec<usize>, FileTranslationError> {
    let mut counts = Vec::with_capacity(files.len());
    for path in files {
        let reader = open_utf8_translation_input(path).map_err(FileTranslationError::input)?;
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
