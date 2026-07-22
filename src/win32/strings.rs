/// UTF-8 문자열을 null-terminated UTF-16 `Vec<u16>`로 변환한다.
///
/// raw 포인터, mutable 버퍼 또는 고정 길이 배열을 요구하는 Win32 경계에 사용한다.
pub(crate) fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
