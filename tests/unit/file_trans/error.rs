use super::FileTranslationError;
use std::path::PathBuf;

#[test]
fn constructor_helpers_wrap_the_message_in_the_matching_variant() {
    assert_eq!(
        FileTranslationError::path("경로 문제"),
        FileTranslationError::Path("경로 문제".to_string())
    );
    assert_eq!(
        FileTranslationError::input("입력 문제"),
        FileTranslationError::Input("입력 문제".to_string())
    );
    assert_eq!(
        FileTranslationError::encoding("인코딩 문제"),
        FileTranslationError::Encoding("인코딩 문제".to_string())
    );
    assert_eq!(
        FileTranslationError::output("출력 문제"),
        FileTranslationError::Output("출력 문제".to_string())
    );
    assert_eq!(
        FileTranslationError::backend("backend 문제"),
        FileTranslationError::Backend("backend 문제".to_string())
    );
}

#[test]
fn display_messages_include_the_wrapped_detail_for_each_variant() {
    assert_eq!(
        FileTranslationError::Cancelled.to_string(),
        "사용자가 취소했습니다."
    );
    assert_eq!(
        FileTranslationError::InvalidRequest("이유".to_string()).to_string(),
        "잘못된 파일 번역 요청: 이유"
    );
    assert_eq!(
        FileTranslationError::path("상세").to_string(),
        "파일 경로 오류: 상세"
    );
    assert_eq!(
        FileTranslationError::input("상세").to_string(),
        "입력 파일 오류: 상세"
    );
    assert_eq!(
        FileTranslationError::encoding("상세").to_string(),
        "입력 인코딩 오류: 상세"
    );
    assert_eq!(
        FileTranslationError::output("상세").to_string(),
        "출력 파일 오류: 상세"
    );
    assert_eq!(
        FileTranslationError::Runtime("상세".to_string()).to_string(),
        "파일 번역 실행기 오류: 상세"
    );
    assert_eq!(
        FileTranslationError::backend("상세").to_string(),
        "파일 번역 backend 오류: 상세"
    );
    assert_eq!(
        FileTranslationError::TooManyLines.to_string(),
        "입력 파일의 전체 줄 수가 너무 많습니다."
    );
}

#[test]
fn line_too_long_message_includes_the_path_and_byte_limit() {
    let error = FileTranslationError::LineTooLong {
        path: PathBuf::from("C:/input/very-long.txt"),
        limit: 8 * 1024 * 1024,
    };
    let message = error.to_string();
    assert!(message.contains("C:/input/very-long.txt"), "{message}");
    assert!(message.contains("8388608"), "{message}");
}
