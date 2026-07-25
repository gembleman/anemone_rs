use super::{CHECKSUM_ASSET_NAME, UPDATE_ASSET_NAME, UpdateError, ensure_trusted_url};

fn url(text: &str) -> reqwest::Url {
    reqwest::Url::parse(text).expect("유효한 URL이어야 한다")
}

#[test]
fn github_hosts_are_trusted_over_https() {
    for text in [
        "https://api.github.com/repos/gembleman/anemone_rs/releases/latest",
        "https://github.com/gembleman/anemone_rs/releases/download/v0.1.0/app.exe",
        "https://objects.githubusercontent.com/some/path",
        "https://release-assets.githubusercontent.com/some/path",
    ] {
        assert!(
            ensure_trusted_url(&url(text)).is_ok(),
            "{text}는 허용해야 한다"
        );
    }
}

#[test]
fn plain_http_is_rejected() {
    assert!(matches!(
        ensure_trusted_url(&url("http://github.com/gembleman/anemone_rs")),
        Err(UpdateError::UntrustedUrl(_))
    ));
}

/// 호스트를 문자열 `contains`로 비교하면 통과해버리는 URL들이다.
#[test]
fn lookalike_hosts_do_not_pass_the_check() {
    for text in [
        "https://evil.example.com/api.github.com/releases",
        "https://api.github.com.evil.example.com/releases",
        "https://notgithub.com/gembleman",
        "https://githubusercontent.com/path",
    ] {
        assert!(
            matches!(
                ensure_trusted_url(&url(text)),
                Err(UpdateError::UntrustedUrl(_))
            ),
            "{text}는 거부해야 한다"
        );
    }
}

/// 체크섬 asset 이름은 exe 이름에서 파생된다. CI가 두 파일을 함께 올리므로
/// 규칙이 어긋나면 앱이 업데이트를 적용하지 못한다.
#[test]
fn the_checksum_asset_name_derives_from_the_update_asset_name() {
    assert_eq!(CHECKSUM_ASSET_NAME, format!("{UPDATE_ASSET_NAME}.sha256"));
    assert!(UPDATE_ASSET_NAME.ends_with(".exe"));
}
