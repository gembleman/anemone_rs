use super::{classify_common_dialog_result, parse_selection};

#[test]
fn common_dialog_zero_error_is_cancel_but_nonzero_is_failure() {
    assert!(classify_common_dialog_result("GetOpenFileNameW", 0).is_ok());
    let error = classify_common_dialog_result("GetOpenFileNameW", 0x3003)
        .expect_err("nonzero CommDlgExtendedError must be surfaced");
    assert!(error.to_string().contains("12291"));
}

#[test]
fn parses_cancelled_or_empty_selection_as_no_paths() {
    assert!(parse_selection(&[0], false).is_empty());
    assert!(parse_selection(&[], false).is_empty());
}

#[test]
fn parses_single_and_multi_file_selections() {
    let single = "C:\\input.txt"
        .encode_utf16()
        .chain([0])
        .collect::<Vec<_>>();
    assert_eq!(
        parse_selection(&single, false),
        vec![std::path::PathBuf::from("C:\\input.txt")]
    );

    let multi = "C:\\data"
        .encode_utf16()
        .chain([0])
        .chain("one.txt".encode_utf16())
        .chain([0])
        .chain("two.txt".encode_utf16())
        .chain([0, 0])
        .collect::<Vec<_>>();
    assert_eq!(
        parse_selection(&multi, true),
        vec![
            std::path::PathBuf::from("C:\\data\\one.txt"),
            std::path::PathBuf::from("C:\\data\\two.txt"),
        ]
    );
}
