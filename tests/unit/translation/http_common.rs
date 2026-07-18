use super::*;

#[test]
fn parses_retry_after_delta_seconds() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::RETRY_AFTER, "17".parse().unwrap());
    assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(17)));
}

#[test]
fn ignores_invalid_retry_after_without_inventing_a_delay() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::RETRY_AFTER, "invalid".parse().unwrap());
    assert_eq!(parse_retry_after(&headers), None);
}

#[test]
fn bounds_retry_after_to_two_minutes() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::RETRY_AFTER, "9999".parse().unwrap());
    assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(120)));
}
