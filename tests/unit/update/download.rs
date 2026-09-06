use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::super::{AvailableUpdate, UPDATE_ASSET_NAME, UpdateError, Version, sha256};
use super::{
    DownloadProgress, StagedUpdate, download_with_asset_name, fetch_checked, finalize_staged,
    parse_checksum, stream_to_file,
};

const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

// --- parse_checksum (순수 함수) ------------------------------------------

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

// --- 테스트 인프라: 로컬 스텁 서버 -----------------------------------------

/// 원시 HTTP 응답 바이트를 한 번 써주는 로컬 서버. `http://127.0.0.1:<port>/`를
/// 돌려준다. `ensure_trusted_url`은 github 호스트만 허용하므로, 여기서 만드는
/// URL은 신뢰 검사를 건너뛰는 저수준 함수(`fetch_checked`, `stream_to_file`)를
/// 직접 호출하는 테스트에만 쓴다 — 신뢰 검사 자체는 `tests/unit/update/mod.rs`가
/// 담당한다.
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

fn http_ok(body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

static DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempFile(PathBuf);

impl TempFile {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "anemone-update-download-{label}-{}-{}",
            std::process::id(),
            DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        Self(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
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

// --- fetch_checked: 상태 코드 / Content-Length 상한 -------------------------

#[tokio::test]
async fn fetch_checked_reports_the_response_status_code_on_failure() {
    let response =
        "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .as_bytes()
            .to_vec();
    let url = serve_once(response);
    let parsed = reqwest::Url::parse(&url).unwrap();

    let result = fetch_checked(parsed, Duration::from_secs(5), 1024).await;
    assert!(matches!(result, Err(UpdateError::Api { code: 500 })));
}

#[tokio::test]
async fn fetch_checked_rejects_a_content_length_over_the_limit_before_reading_the_body() {
    let response = http_ok(&[b'x'; 100]);
    let url = serve_once(response);
    let parsed = reqwest::Url::parse(&url).unwrap();

    let result = fetch_checked(parsed, Duration::from_secs(5), 10).await;
    assert!(matches!(result, Err(UpdateError::TooLarge { limit: 10 })));
}

#[tokio::test]
async fn fetch_checked_returns_the_body_and_total_length_on_success() {
    let body = b"hello world";
    let response = http_ok(body);
    let url = serve_once(response);
    let parsed = reqwest::Url::parse(&url).unwrap();

    let (response, total) = fetch_checked(parsed, Duration::from_secs(5), 1024)
        .await
        .expect("정상 응답이어야 한다");
    assert_eq!(total, Some(body.len() as u64));
    let text = response.text().await.unwrap();
    assert_eq!(text, "hello world");
}

// --- stream_to_file: 해시/진행률/상한 ---------------------------------------

#[tokio::test]
async fn stream_to_file_hashes_and_writes_the_full_body() {
    let body = b"the quick brown fox";
    let response = http_ok(body);
    let url = serve_once(response);
    let mut response = get_response(url).await;

    let temp = TempFile::new("ok");
    let file = std::fs::File::create(&temp.0).unwrap();

    let mut progress_calls: Vec<DownloadProgress> = Vec::new();
    let digest = stream_to_file(
        &mut response,
        file,
        Some(body.len() as u64),
        1024,
        &mut |progress| progress_calls.push(progress),
    )
    .await
    .expect("스트리밍이 성공해야 한다");

    assert_eq!(digest, sha256::digest(body).unwrap());
    assert_eq!(std::fs::read(&temp.0).unwrap(), body);
    // 0%를 먼저 알린 뒤, 적어도 한 번은 최종 진행률을 알린다.
    assert_eq!(progress_calls.first().unwrap().received, 0);
    assert_eq!(progress_calls.last().unwrap().received, body.len() as u64);
}

#[tokio::test]
async fn stream_to_file_aborts_once_received_bytes_exceed_the_limit() {
    let body = vec![b'x'; 64];
    let response = http_ok(&body);
    let url = serve_once(response);
    let mut response = get_response(url).await;

    let temp = TempFile::new("oversize");
    let file = std::fs::File::create(&temp.0).unwrap();

    // max_bytes를 실제 본문보다 훨씬 작게 둬서, 64MB를 실제로 보내지 않고도
    // 초과 중단 경로를 결정적으로 재현한다.
    let result = stream_to_file(&mut response, file, None, 8, &mut |_| {}).await;

    assert!(matches!(result, Err(UpdateError::TooLarge { limit: 8 })));
}

// --- finalize_staged: 해시 대조 후 StagedUpdate 정리 -------------------------

fn staged_at(temp: &TempFile, contents: &[u8]) -> StagedUpdate {
    std::fs::write(&temp.0, contents).expect("임시 파일을 쓸 수 있어야 한다");
    // 이 테스트 모듈은 #[path]로 download 모듈의 자식이라 비공개 필드에
    // 직접 접근할 수 있다 — transmute 등의 편법이 필요 없다.
    StagedUpdate {
        path: temp.0.clone(),
        applied: false,
    }
}

#[test]
fn finalize_staged_keeps_the_file_when_hashes_match() {
    let temp = TempFile::new("match");
    let body = b"matched contents";
    let staged = staged_at(&temp, body);
    let digest = sha256::digest(body).unwrap();

    let staged = finalize_staged(staged, digest, digest).expect("일치하면 성공해야 한다");
    assert!(staged.path().exists());
    // 여기서 drop되며 applied=false라 Drop이 파일을 지운다 — 실제 apply
    // 단계에서만 into_applied()로 생존시킨다.
}

#[test]
fn finalize_staged_deletes_the_file_and_reports_a_mismatch() {
    let temp = TempFile::new("mismatch");
    let staged = staged_at(&temp, b"corrupted contents");
    let expected = sha256::digest(b"original contents").unwrap();
    let actual = sha256::digest(b"corrupted contents").unwrap();
    assert_ne!(expected, actual);

    let path = temp.0.clone();
    let result = finalize_staged(staged, actual, expected);

    assert!(matches!(result, Err(UpdateError::ChecksumMismatch)));
    assert!(
        !path.exists(),
        "해시가 어긋나면 임시 파일이 Drop으로 지워져야 한다"
    );
}

/// `into_applied()`는 Drop의 삭제를 막는다 — 실제 적용(`apply::replace_running_executable`)
/// 단계로 넘어간 뒤에는 더 이상 임시 파일이 아니므로 지우면 안 된다.
#[test]
fn into_applied_prevents_drop_from_deleting_the_file() {
    let temp = TempFile::new("into-applied");
    let staged = staged_at(&temp, b"payload");
    let path = staged.into_applied();

    assert_eq!(path, temp.0);
    assert!(
        path.exists(),
        "into_applied 이후에는 파일이 남아 있어야 한다"
    );
    std::fs::remove_file(&path).expect("정리");
}

/// 파일이 이미 없어진 상태(예: 다른 코드가 먼저 지움)에서 Drop이 실행돼도
/// NotFound는 경고 없이 넘어가야 한다 — panic도, 새 오류 로그도 없어야 한다.
#[test]
fn dropping_a_staged_update_tolerates_a_file_that_is_already_gone() {
    let temp = TempFile::new("already-gone");
    let staged = staged_at(&temp, b"payload");
    std::fs::remove_file(&temp.0).expect("미리 지울 수 있어야 한다");
    // Drop이 NotFound를 조용히 넘기는지 확인 — 패닉하면 테스트가 실패한다.
    drop(staged);
}

// --- download_with_asset_name: 신뢰 경계 -----------------------------------

fn update_with(asset_url: &str, checksum_url: &str) -> AvailableUpdate {
    AvailableUpdate {
        version: Version::current(),
        asset_url: asset_url.to_string(),
        checksum_url: checksum_url.to_string(),
        release_page_url: String::new(),
    }
}

#[tokio::test]
async fn an_untrusted_checksum_host_is_rejected_before_any_network_call() {
    let update = update_with(
        "https://github.com/gembleman/anemone_rs/releases/download/v1/app.exe",
        "http://evil.example.com/app.exe.sha256",
    );
    let temp = TempFile::new("untrusted-checksum");

    let result = download_with_asset_name(&update, temp.0.clone(), UPDATE_ASSET_NAME, |_| {}).await;

    assert!(matches!(result, Err(UpdateError::UntrustedUrl(_))));
    assert!(
        !temp.0.exists(),
        "체크섬 URL이 거부되면 파일을 만들지도 않아야 한다"
    );
}

#[tokio::test]
async fn a_malformed_checksum_url_is_a_parse_error_before_any_network_call() {
    // checksum_url을 먼저 확인하므로, 여기서 거부되면 asset_url이 실제 github
    // 호스트를 가리켜도(=신뢰 검사를 통과해도) 네트워크로 나가지 않는다.
    let update = update_with(
        "https://github.com/gembleman/anemone_rs/releases/download/v1/app.exe",
        "not a url at all",
    );
    let temp = TempFile::new("malformed-checksum");

    let result = download_with_asset_name(&update, temp.0.clone(), UPDATE_ASSET_NAME, |_| {}).await;

    assert!(matches!(result, Err(UpdateError::Parse(_))));
    assert!(!temp.0.exists());
}
