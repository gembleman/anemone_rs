//! 공통 유틸리티 함수

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

/// UTF-8 문자열을 null-terminated UTF-16 `Vec<u16>`로 변환한다.
///
/// Win32 API에 문자열을 전달할 때 사용.
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-8/UTF-8 BOM 입력을 스트리밍으로 읽을 reader를 연다. 인코딩 전체 검증은
/// 소비자가 읽는 동안 수행하며, 여기서는 잘못된 BOM을 먼저 거른다.
pub fn open_utf8_translation_input(path: &Path) -> Result<BufReader<File>, String> {
    let file = File::open(path)
        .map_err(|e| format!("입력 파일을 열 수 없습니다: {} ({e})", path.display()))?;
    let mut reader = BufReader::new(file);
    let prefix = reader
        .fill_buf()
        .map_err(|e| format!("입력 파일을 읽을 수 없습니다: {} ({e})", path.display()))?;
    let kind = detect_non_utf8_bom(prefix);
    if let Some(name) = kind {
        return Err(format!(
            "지원하지 않는 인코딩입니다: {name}. UTF-8 또는 UTF-8 BOM 파일만 사용할 수 있습니다. ({})",
            path.display()
        ));
    }

    Ok(reader)
}

/// 미리보기는 파일 크기와 무관한 고정 바이트 상한 안에서만 읽는다.
pub fn read_utf8_preview(path: &Path, max_lines: usize, max_bytes: u64) -> Result<String, String> {
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
