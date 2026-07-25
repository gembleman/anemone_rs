use super::super::{UPDATE_ASSET_NAME, UpdateError, sha256};
use super::parse_checksum;

const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn the_release_manifest_format_is_parsed() {
    // verify-release.ps1이 쓰는 `<hex>  <파일명>` 두 칸 구분 형식.
    let text = format!("{ABC}  {UPDATE_ASSET_NAME}\n");
    assert_eq!(
        parse_checksum(&text, UPDATE_ASSET_NAME).unwrap(),
        sha256::digest(b"abc").unwrap()
    );
}

#[test]
fn a_bare_hash_without_a_filename_is_accepted() {
    assert_eq!(
        parse_checksum(&format!("{ABC}\n"), UPDATE_ASSET_NAME).unwrap(),
        sha256::digest(b"abc").unwrap()
    );
}

#[test]
fn the_matching_filename_is_selected_from_a_multi_entry_manifest() {
    let other = "0".repeat(64);
    let text = format!("{other}  some-other-file.zip\n{ABC}  {UPDATE_ASSET_NAME}\n");

    assert_eq!(
        parse_checksum(&text, UPDATE_ASSET_NAME).unwrap(),
        sha256::digest(b"abc").unwrap()
    );
}

#[test]
fn a_manifest_without_the_expected_file_is_an_error() {
    let text = format!("{ABC}  some-other-file.zip\n");
    assert!(matches!(
        parse_checksum(&text, UPDATE_ASSET_NAME),
        Err(UpdateError::Parse(_))
    ));
}

#[test]
fn malformed_hashes_are_rejected() {
    for hex in ["", "abc", &"z".repeat(64), &ABC[..63]] {
        let text = format!("{hex}  {UPDATE_ASSET_NAME}\n");
        assert!(
            parse_checksum(&text, UPDATE_ASSET_NAME).is_err(),
            "{hex:?}는 거부해야 한다"
        );
    }
}

#[test]
fn blank_lines_are_skipped() {
    let text = format!("\n\n  \n{ABC}  {UPDATE_ASSET_NAME}\n\n");
    assert!(parse_checksum(&text, UPDATE_ASSET_NAME).is_ok());
}
