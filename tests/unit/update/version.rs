use super::Version;

fn v(text: &str) -> Version {
    text.parse().expect("유효한 버전이어야 한다")
}

#[test]
fn tag_and_cargo_version_notations_parse_to_the_same_value() {
    assert_eq!(v("v0.1.0"), v("0.1.0"));
    assert_eq!(v("  v1.2.3  "), v("1.2.3"));
}

#[test]
fn version_order_compares_numbers_not_strings() {
    // 문자열 비교였다면 "0.10.0" < "0.2.0"으로 뒤집힌다.
    assert!(v("0.10.0") > v("0.2.0"));
    assert!(v("0.2.0") > v("0.1.9"));
    assert!(v("1.0.0") > v("0.99.99"));
    assert_eq!(v("0.1.0"), v("0.1.0"));
}

#[test]
fn prerelease_sorts_below_the_matching_release() {
    assert!(v("0.2.0-rc1") < v("0.2.0"));
    assert!(v("0.2.0-rc1") > v("0.1.0"));
}

#[test]
fn upgrades_require_a_higher_release_version() {
    assert!(v("0.2.0").is_upgrade_from(&v("0.1.0")));
    assert!(!v("0.1.0").is_upgrade_from(&v("0.1.0")));
    assert!(!v("0.1.0").is_upgrade_from(&v("0.2.0")));
}

#[test]
fn prereleases_are_never_offered_as_updates() {
    // 태그를 잘못 붙여 latest가 프리릴리스여도 사용자에게 밀려나가면 안 된다.
    assert!(!v("0.2.0-rc1").is_upgrade_from(&v("0.1.0")));
    assert!(!v("9.9.9-beta").is_upgrade_from(&v("0.1.0")));
}

#[test]
fn a_prerelease_build_can_move_to_the_matching_release() {
    assert!(v("0.2.0").is_upgrade_from(&v("0.2.0-rc1")));
}

#[test]
fn malformed_versions_are_rejected_instead_of_silently_accepted() {
    for text in [
        "", "0.1", "0.1.0.0", "a.b.c", "0.1.x", "v", "0..0", "0.1.0-", "-1.0.0",
    ] {
        assert!(
            text.parse::<Version>().is_err(),
            "{text:?}는 오류로 처리해야 한다"
        );
    }
}

#[test]
fn build_metadata_is_ignored() {
    assert_eq!(v("0.1.0+20260725"), v("0.1.0"));
}

#[test]
fn current_version_matches_the_crate_version() {
    assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn display_round_trips_through_parsing() {
    for text in ["0.1.0", "1.2.3", "0.2.0-rc1"] {
        assert_eq!(v(text).to_string(), text);
    }
}
