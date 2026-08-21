use super::super::UpdateError;
use super::super::check::{AvailableUpdate, UpdateCheck};
use super::super::version::Version;
use super::{AUTO_CHECK_INTERVAL_SECS, should_auto_check, should_update_last_check};

fn version(text: &str) -> Version {
    text.parse().expect("유효한 버전이어야 한다")
}

fn available_update() -> AvailableUpdate {
    AvailableUpdate {
        version: version("9.9.9"),
        asset_url: "https://github.com/gembleman/anemone_rs/releases/download/v9.9.9/app.exe"
            .to_string(),
        checksum_url:
            "https://github.com/gembleman/anemone_rs/releases/download/v9.9.9/app.exe.sha256"
                .to_string(),
        release_page_url: "https://github.com/gembleman/anemone_rs/releases/tag/v9.9.9".to_string(),
    }
}

// -- should_auto_check --------------------------------------------------

#[test]
fn disabled_never_checks() {
    assert!(!should_auto_check(
        false,
        0,
        1_000_000,
        AUTO_CHECK_INTERVAL_SECS
    ));
    assert!(!should_auto_check(
        false,
        1,
        1_000_000_000,
        AUTO_CHECK_INTERVAL_SECS
    ));
}

#[test]
fn never_checked_before_always_checks() {
    assert!(should_auto_check(
        true,
        0,
        1_000_000,
        AUTO_CHECK_INTERVAL_SECS
    ));
}

#[test]
fn negative_last_check_is_treated_as_never_checked() {
    assert!(should_auto_check(
        true,
        -1,
        1_000_000,
        AUTO_CHECK_INTERVAL_SECS
    ));
}

#[test]
fn less_than_24_hours_elapsed_skips() {
    let last_check = 1_000_000;
    let now = last_check + AUTO_CHECK_INTERVAL_SECS - 1;
    assert!(!should_auto_check(
        true,
        last_check,
        now,
        AUTO_CHECK_INTERVAL_SECS
    ));
}

#[test]
fn exactly_24_hours_elapsed_checks() {
    let last_check = 1_000_000;
    let now = last_check + AUTO_CHECK_INTERVAL_SECS;
    assert!(should_auto_check(
        true,
        last_check,
        now,
        AUTO_CHECK_INTERVAL_SECS
    ));
}

#[test]
fn more_than_24_hours_elapsed_checks() {
    let last_check = 1_000_000;
    let now = last_check + AUTO_CHECK_INTERVAL_SECS + 1;
    assert!(should_auto_check(
        true,
        last_check,
        now,
        AUTO_CHECK_INTERVAL_SECS
    ));
}

#[test]
fn clock_moving_backwards_does_not_panic_and_skips() {
    let last_check = 1_000_000;
    let now = last_check - 10;
    assert!(!should_auto_check(
        true,
        last_check,
        now,
        AUTO_CHECK_INTERVAL_SECS
    ));
}

// -- should_update_last_check -------------------------------------------

#[test]
fn up_to_date_updates_last_check() {
    assert!(should_update_last_check(&Ok(UpdateCheck::UpToDate)));
}

#[test]
fn available_update_updates_last_check() {
    assert!(should_update_last_check(&Ok(UpdateCheck::Available(
        available_update()
    ))));
}

#[test]
fn unsupported_release_updates_last_check() {
    assert!(should_update_last_check(&Ok(UpdateCheck::Unsupported {
        version: version("9.9.9"),
        reason: "asset 없음".to_string(),
        release_page_url: String::new(),
    })));
}

#[test]
fn rate_limited_updates_last_check() {
    assert!(should_update_last_check(&Err(UpdateError::RateLimited)));
}

/// 가장 중요한 케이스: 네트워크 실패로 갱신하면 오프라인이었던 하루 때문에
/// 다음 24시간을 더 놓친다.
#[test]
fn network_failure_does_not_update_last_check() {
    assert!(!should_update_last_check(&Err(UpdateError::Network(
        "connection refused".to_string()
    ))));
}

#[test]
fn parse_failure_updates_last_check() {
    assert!(should_update_last_check(&Err(UpdateError::Parse(
        "bad json".to_string()
    ))));
}

#[test]
fn api_error_updates_last_check() {
    assert!(should_update_last_check(&Err(UpdateError::Api {
        code: 500
    })));
}
