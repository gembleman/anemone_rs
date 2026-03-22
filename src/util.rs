//! 공통 유틸리티 함수

/// UTF-8 문자열을 null-terminated UTF-16 `Vec<u16>`로 변환한다.
///
/// Win32 API에 문자열을 전달할 때 사용.
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
