use std::io::{Read, Write};
use std::net::TcpListener;

use super::super::{CHECKSUM_ASSET_NAME, UPDATE_ASSET_NAME, UpdateError, Version};
use super::{UpdateCheck, parse_releases, read_limited};

fn v(text: &str) -> Version {
    text.parse().expect("유효한 버전이어야 한다")
}

/// 실제 GitHub 응답을 본뜬 픽스처. 앱이 쓰지 않는 필드도 함께 넣어
/// serde가 그것들을 무시하는지 확인한다.
fn release_json(tag: &str, assets: &[&str]) -> String {
    let assets: Vec<String> = assets
        .iter()
        .map(|name| {
            format!(
                r#"{{
                    "url": "https://api.github.com/repos/gembleman/anemone_rs/releases/assets/1",
                    "id": 1,
                    "name": "{name}",
                    "content_type": "application/octet-stream",
                    "size": 5534208,
                    "download_count": 0,
                    "browser_download_url": "https://github.com/gembleman/anemone_rs/releases/download/{tag}/{name}"
                }}"#
            )
        })
        .collect();

    format!(
        r#"{{
            "url": "https://api.github.com/repos/gembleman/anemone_rs/releases/1",
            "id": 1,
            "tag_name": "{tag}",
            "target_commitish": "master",
            "name": "{tag}",
            "draft": false,
            "prerelease": false,
            "created_at": "2026-07-25T00:00:00Z",
            "published_at": "2026-07-25T00:00:00Z",
            "html_url": "https://github.com/gembleman/anemone_rs/releases/tag/{tag}",
            "body": "릴리스 노트",
            "assets": [{}]
        }}"#,
        assets.join(",")
    )
}

fn release_list_json(releases: &[String]) -> String {
    format!("[{}]", releases.join(","))
}

fn single_release_json(tag: &str, assets: &[&str]) -> String {
    release_list_json(&[release_json(tag, assets)])
}

#[test]
fn a_newer_release_with_both_assets_is_offered() {
    let body = single_release_json("v0.2.0", &[UPDATE_ASSET_NAME, CHECKSUM_ASSET_NAME]);

    let UpdateCheck::Available(update) = parse_releases(&body, &v("0.1.0")).unwrap() else {
        panic!("새 버전을 제공해야 한다");
    };

    assert_eq!(update.version, v("0.2.0"));
    assert!(update.asset_url.ends_with(UPDATE_ASSET_NAME));
    assert!(update.checksum_url.ends_with(CHECKSUM_ASSET_NAME));
    assert_eq!(
        update.release_page_url,
        "https://github.com/gembleman/anemone_rs/releases/tag/v0.2.0"
    );
}

#[test]
fn the_same_or_older_release_is_up_to_date() {
    let body = single_release_json("v0.1.0", &[UPDATE_ASSET_NAME, CHECKSUM_ASSET_NAME]);
    assert_eq!(
        parse_releases(&body, &v("0.1.0")).unwrap(),
        UpdateCheck::UpToDate
    );
    assert_eq!(
        parse_releases(&body, &v("0.9.0")).unwrap(),
        UpdateCheck::UpToDate
    );
}

/// GitHub는 태그를 붙인 그대로 돌려준다. 실제로 `v` 없이 `15.2.0` 형태로
/// 배포하는 저장소가 있으므로(ripgrep 등), 양쪽 표기를 모두 받아야 한다.
#[test]
fn a_tag_without_the_v_prefix_is_handled() {
    let body = single_release_json("0.2.0", &[UPDATE_ASSET_NAME, CHECKSUM_ASSET_NAME]);

    let UpdateCheck::Available(update) = parse_releases(&body, &v("0.1.0")).unwrap() else {
        panic!("v 없는 태그도 새 버전으로 인식해야 한다");
    };
    assert_eq!(update.version, v("0.2.0"));
}

#[test]
fn a_non_version_release_does_not_hide_the_latest_version() {
    let body = release_list_json(&[
        release_json("starter_kit", &["anemone_rs_starter_kit_v0.1.1.zip"]),
        release_json("v0.2.0", &[UPDATE_ASSET_NAME, CHECKSUM_ASSET_NAME]),
    ]);

    let UpdateCheck::Available(update) = parse_releases(&body, &v("0.1.0")).unwrap() else {
        panic!("비버전 릴리스를 건너뛰고 최신 버전을 제공해야 한다");
    };
    assert_eq!(update.version, v("0.2.0"));
}

#[test]
fn the_highest_valid_version_is_selected_regardless_of_api_order() {
    let body = release_list_json(&[
        release_json("v0.2.0", &[UPDATE_ASSET_NAME, CHECKSUM_ASSET_NAME]),
        release_json("v0.3.0", &[UPDATE_ASSET_NAME, CHECKSUM_ASSET_NAME]),
        release_json("v0.1.9", &[UPDATE_ASSET_NAME, CHECKSUM_ASSET_NAME]),
    ]);

    let UpdateCheck::Available(update) = parse_releases(&body, &v("0.1.0")).unwrap() else {
        panic!("가장 높은 유효 버전을 제공해야 한다");
    };
    assert_eq!(update.version, v("0.3.0"));
}

/// 최초 사용자용 full zip을 업데이트 asset으로 착각하면 안 된다.
#[test]
fn a_release_with_only_the_full_zip_is_unsupported_not_up_to_date() {
    let body = single_release_json("v0.2.0", &["anemone_rs-v0.2.0-full.zip"]);

    let result = parse_releases(&body, &v("0.1.0")).unwrap();

    assert!(
        matches!(result, UpdateCheck::Unsupported { .. }),
        "조용히 최신으로 처리하면 배포 실수를 발견하지 못한다: {result:?}"
    );
}

/// asset 전환 과도기 등으로 자동 적용이 막힌 경우에도 수동 내려받기 경로를
/// 잃지 않아야 한다.
#[test]
fn an_unsupported_result_carries_the_release_page_url() {
    let body = single_release_json("v0.2.0", &[UPDATE_ASSET_NAME]);

    match parse_releases(&body, &v("0.1.0")).unwrap() {
        UpdateCheck::Unsupported {
            release_page_url, ..
        } => {
            assert_eq!(
                release_page_url,
                "https://github.com/gembleman/anemone_rs/releases/tag/v0.2.0"
            );
        }
        other => panic!("Unsupported여야 한다: {other:?}"),
    }
}

#[test]
fn a_release_without_the_checksum_is_unsupported() {
    let body = single_release_json("v0.2.0", &[UPDATE_ASSET_NAME]);
    assert!(matches!(
        parse_releases(&body, &v("0.1.0")).unwrap(),
        UpdateCheck::Unsupported { .. }
    ));
}

#[test]
fn partial_name_matches_are_not_accepted() {
    // 이름이 접두사만 같은 asset은 고르지 않는다.
    let body = single_release_json("v0.2.0", &["anemone_rs-x86_64-pc-windows-msvc.exe.bak"]);
    assert!(matches!(
        parse_releases(&body, &v("0.1.0")).unwrap(),
        UpdateCheck::Unsupported { .. }
    ));
}

#[test]
fn draft_and_prerelease_flags_suppress_the_update() {
    for flag in ["draft", "prerelease"] {
        let body = single_release_json("v0.2.0", &[UPDATE_ASSET_NAME, CHECKSUM_ASSET_NAME])
            .replace(&format!("\"{flag}\": false"), &format!("\"{flag}\": true"));
        assert_eq!(
            parse_releases(&body, &v("0.1.0")).unwrap(),
            UpdateCheck::UpToDate,
            "{flag} 릴리스는 제공하지 않아야 한다"
        );
    }
}

#[test]
fn asset_urls_outside_the_allowed_hosts_are_rejected() {
    let body = single_release_json("v0.2.0", &[UPDATE_ASSET_NAME, CHECKSUM_ASSET_NAME]).replace(
        "https://github.com/gembleman",
        "https://evil.example.com/gembleman",
    );

    assert!(matches!(
        parse_releases(&body, &v("0.1.0")),
        Err(UpdateError::UntrustedUrl(_))
    ));
}

#[test]
fn malformed_payloads_are_reported_as_parse_errors() {
    for body in ["", "{", "{}", r#"{"tag_name": "not-a-version"}"#] {
        assert!(
            matches!(
                parse_releases(body, &v("0.1.0")),
                Err(UpdateError::Parse(_))
            ),
            "{body:?}는 파싱 오류여야 한다"
        );
    }
}

#[test]
fn a_list_with_only_non_version_releases_is_up_to_date() {
    let body = release_list_json(&[release_json("starter_kit", &[])]);
    assert_eq!(
        parse_releases(&body, &v("0.1.0")).unwrap(),
        UpdateCheck::UpToDate
    );
}

#[test]
fn unknown_fields_do_not_break_parsing() {
    let body = single_release_json("v0.2.0", &[UPDATE_ASSET_NAME, CHECKSUM_ASSET_NAME]).replace(
        r#""body": "릴리스 노트""#,
        r#""body": "노트", "future_field": {"nested": [1,2,3]}"#,
    );

    assert!(matches!(
        parse_releases(&body, &v("0.1.0")).unwrap(),
        UpdateCheck::Available(_)
    ));
}

// --- read_limited: 네트워크와 분리된 본문 상한 로직 --------------------------
//
// `read_limited`는 `ensure_trusted_url`을 호출하지 않는다 — 이미 검증된 응답의
// 본문만 다룬다. 그래서 로컬 스텁 서버로 상한/인코딩 오류 분기를 직접 재현할
// 수 있다 (신뢰 검사 자체는 `tests/unit/update/mod.rs`가 담당).

fn serve_once(response: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request);
        let _ = stream.write_all(&response);
    });
    format!("http://{address}/")
}

async fn get_response(url: String) -> reqwest::Response {
    reqwest::Client::builder()
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .expect("로컬 서버 응답을 받아야 한다")
}

#[tokio::test]
async fn read_limited_returns_the_body_when_within_the_limit() {
    let body = r#"[{"tag_name":"v1.0.0"}]"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes();
    let url = serve_once(response);
    let response = get_response(url).await;

    let text = read_limited(response, 4096)
        .await
        .expect("본문을 읽어야 한다");
    assert_eq!(text, body);
}

#[tokio::test]
async fn read_limited_rejects_a_content_length_header_over_the_limit() {
    let body = [b'x'; 200];
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    let url = serve_once(response);
    let response = get_response(url).await;

    let result = read_limited(response, 100).await;
    assert!(matches!(result, Err(UpdateError::TooLarge { limit: 100 })));
}

#[tokio::test]
async fn read_limited_rejects_actual_bytes_over_the_limit_even_without_content_length() {
    // chunked라 Content-Length 헤더 자체가 없다 — 사전 검사를 통과해도
    // 실제로 받은 바이트 총량으로 다시 걸러야 한다.
    let body = vec![b'y'; 200];
    let mut response =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(format!("{:X}\r\n", body.len()).as_bytes());
    response.extend_from_slice(&body);
    response.extend_from_slice(b"\r\n0\r\n\r\n");
    let url = serve_once(response);
    let response = get_response(url).await;

    let result = read_limited(response, 100).await;
    assert!(matches!(result, Err(UpdateError::TooLarge { limit: 100 })));
}

#[tokio::test]
async fn read_limited_reports_non_utf8_bodies_as_parse_errors() {
    let body: &[u8] = &[0xff, 0xfe, 0xfd];
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    let mut response = response;
    response.extend_from_slice(body);
    let url = serve_once(response);
    let response = get_response(url).await;

    let result = read_limited(response, 4096).await;
    assert!(matches!(result, Err(UpdateError::Parse(_))));
}

#[tokio::test]
async fn read_limited_accepts_an_empty_body() {
    let response = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
    let url = serve_once(response);
    let response = get_response(url).await;

    let text = read_limited(response, 4096)
        .await
        .expect("빈 본문도 허용해야 한다");
    assert_eq!(text, "");
}
