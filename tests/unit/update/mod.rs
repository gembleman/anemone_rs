use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use super::{
    CHECKSUM_ASSET_NAME, UPDATE_ASSET_NAME, UpdateError, build_client, ensure_trusted_url,
};

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

// --- build_client의 리다이렉트 정책: 매 홉마다 신뢰 검사 ---------------------
//
// `ensure_trusted_url`은 최초 URL만 막는다 — CDN을 거치는 실제 다운로드는
// 리다이렉트를 반드시 따라가야 하므로, 각 홉을 검사하는 책임은
// `build_client`의 커스텀 리다이렉트 정책에 있다. 로컬 스텁 서버로 그 정책
// 자체를 검증한다(호스트 화이트리스트를 우회하려는 것이 아니라, 정책이 실제로
// 작동하는지 확인하는 것이 목적이다).

#[tokio::test]
async fn a_redirect_to_an_untrusted_host_is_not_followed() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("로컬 리스너");
    let address = listener.local_addr().unwrap();
    let response = b"HTTP/1.1 302 Found\r\nLocation: https://evil.example.com/payload\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("연결 수락");
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        let _ = stream.write_all(&response);
    });

    let client = build_client(Duration::from_secs(5)).expect("클라이언트 생성");
    let result = client.get(format!("http://{address}/")).send().await;

    assert!(
        result.is_err(),
        "신뢰할 수 없는 호스트로의 리다이렉트는 정책이 거부해야 한다"
    );
}

#[tokio::test]
async fn a_redirect_chain_longer_than_five_hops_is_rejected() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("로컬 리스너");
    let address = listener.local_addr().unwrap();
    let url = format!("http://{address}/");
    let redirect_response = format!(
        "HTTP/1.1 302 Found\r\nLocation: {url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .into_bytes();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            if stream.write_all(&redirect_response).is_err() {
                break;
            }
        }
    });

    let client = build_client(Duration::from_secs(5)).expect("클라이언트 생성");
    let result = client.get(url).send().await;

    assert!(
        result.is_err(),
        "자기 자신으로 무한히 리다이렉트하면 5홉 제한에서 거부되어야 한다"
    );
}
