//! 공통 유틸리티 함수

use std::path::Path;

/// UTF-8 문자열을 null-terminated UTF-16 `Vec<u16>`로 변환한다.
///
/// Win32 API에 문자열을 전달할 때 사용.
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 파일 번역 입력에 허용되는 인코딩. UTF-8 또는 UTF-8 BOM 만 허용한다.
///
/// 검증 성공 시 BOM 을 제외한 본문 바이트(`Vec<u8>`) 를 돌려준다. 호출부는
/// 이 바이트를 그대로 `BufReader::new(Cursor::new(...))` 등으로 다시 줄 단위로
/// 읽으면 된다. 파일을 두 번 열지 않기 위해 본문 전체를 메모리에 올리는 방식.
pub fn read_utf8_translation_input(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("입력 파일을 열 수 없습니다: {} ({e})", path.display()))?;

    // 비-UTF-8 BOM 부터 빠르게 거른다. UTF-16/UTF-32 가 가장 흔한 오인코딩 후보.
    let kind = detect_non_utf8_bom(&bytes);
    if let Some(name) = kind {
        return Err(format!(
            "지원하지 않는 인코딩입니다: {name}. UTF-8 또는 UTF-8 BOM 파일만 사용할 수 있습니다. ({})",
            path.display()
        ));
    }

    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    if let Err(e) = std::str::from_utf8(body) {
        return Err(format!(
            "UTF-8 디코딩 실패(byte {}): UTF-8 또는 UTF-8 BOM 파일만 사용할 수 있습니다. ({})",
            e.valid_up_to(),
            path.display()
        ));
    }
    Ok(body.to_vec())
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
