//! 업데이트 asset을 내려받고 SHA-256으로 검증한다.
//!
//! 코드 서명이 없으므로 이 해시는 **전송 무결성**만 보증한다. 출처 신뢰는
//! TLS와 호스트 고정(`ensure_trusted_url`)에 의존한다.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{
    AvailableUpdate, CHECK_TIMEOUT, DOWNLOAD_TIMEOUT, MAX_ASSET_BYTES, MAX_CHECKSUM_BODY,
    UPDATE_ASSET_NAME, UpdateError, build_client, ensure_trusted_url,
    sha256::{Digest, Hasher},
};

/// 검증까지 끝나 교체를 기다리는 파일.
///
/// 적용되지 않은 채 버려지면 `Drop`이 임시 파일을 지운다.
#[derive(Debug)]
pub struct StagedUpdate {
    path: PathBuf,
    applied: bool,
}

impl StagedUpdate {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 교체에 사용했음을 표시해 `Drop`의 삭제를 막는다.
    pub fn into_applied(mut self) -> PathBuf {
        self.applied = true;
        self.path.clone()
    }
}

impl Drop for StagedUpdate {
    fn drop(&mut self) {
        if self.applied {
            return;
        }
        if let Err(error) = std::fs::remove_file(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("업데이트 임시 파일을 지우지 못했습니다: {error}");
        }
    }
}

/// 다운로드 진행 상황. `total`은 서버가 Content-Length를 주지 않으면 `None`이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadProgress {
    pub received: u64,
    pub total: Option<u64>,
}

/// asset을 받아 `destination`에 저장하고 해시를 검증한다.
///
/// `destination`은 교체 대상 exe와 **같은 볼륨**이어야 한다. 그래야 이후 교체가
/// 볼륨 간 복사 없이 rename으로 끝난다.
///
/// `on_progress`는 청크를 받을 때마다 호출된다. 다운로드 스레드에서 동기로
/// 불리므로 **블로킹하면 안 된다** — 호출자는 값만 남기고 즉시 반환해야 한다.
pub async fn download(
    update: &AvailableUpdate,
    destination: PathBuf,
    on_progress: impl FnMut(DownloadProgress),
) -> Result<StagedUpdate, UpdateError> {
    download_with_asset_name(update, destination, UPDATE_ASSET_NAME, on_progress).await
}

/// 체크섬 매니페스트에서 찾을 파일명을 지정할 수 있는 형태.
/// 실제 릴리스는 항상 `UPDATE_ASSET_NAME`을 쓰고, 이 인자는 테스트용이다.
pub(super) async fn download_with_asset_name(
    update: &AvailableUpdate,
    destination: PathBuf,
    asset_name: &str,
    mut on_progress: impl FnMut(DownloadProgress),
) -> Result<StagedUpdate, UpdateError> {
    let expected = fetch_expected_digest(&update.checksum_url, asset_name).await?;
    let (mut response, total) = open_asset_response(&update.asset_url).await?;

    // 파일을 만든 직후부터 Drop이 정리를 책임지도록 staged를 먼저 세운다.
    let staged = StagedUpdate {
        path: destination,
        applied: false,
    };
    let file = std::fs::File::create(&staged.path)?;
    let actual = stream_to_file(
        &mut response,
        file,
        total,
        MAX_ASSET_BYTES,
        &mut on_progress,
    )
    .await?;

    finalize_staged(staged, actual, expected)
}

/// 스트리밍이 끝난 뒤 해시를 대조한다. 불일치하면 `staged`가 스코프를 벗어나며
/// `Drop`이 임시 파일을 지운다.
///
/// 네트워크와 분리해 둔 순수한 갈림길이라 로컬 파일만으로 양쪽 분기(일치/불일치)를
/// 결정적으로 테스트할 수 있다.
fn finalize_staged(
    staged: StagedUpdate,
    actual: Digest,
    expected: Digest,
) -> Result<StagedUpdate, UpdateError> {
    if actual != expected {
        tracing::warn!(
            "업데이트 해시 불일치: 기대 {expected}, 실제 {actual}. 내려받은 파일을 폐기합니다."
        );
        return Err(UpdateError::ChecksumMismatch);
    }
    Ok(staged)
}

/// asset URL을 검증하고 요청을 보낸 뒤, 상태 코드와 `Content-Length` 상한까지
/// 확인한 응답을 돌려준다. 본문은 아직 읽지 않은 상태다.
async fn open_asset_response(
    asset_url: &str,
) -> Result<(reqwest::Response, Option<u64>), UpdateError> {
    let url =
        reqwest::Url::parse(asset_url).map_err(|error| UpdateError::Parse(error.to_string()))?;
    ensure_trusted_url(&url)?;
    fetch_checked(url, DOWNLOAD_TIMEOUT, MAX_ASSET_BYTES).await
}

/// URL 신뢰 검사를 통과한 뒤 요청을 보내고, 상태 코드와 `Content-Length` 상한을
/// 확인한다. 호스트 화이트리스트 검사(`ensure_trusted_url`)는 호출자 책임이다 —
/// 이 함수 자체는 그 검사를 하지 않으므로, 테스트는 로컬 스텁 서버로 상태
/// 코드/크기 상한 로직만 골라 검증할 수 있다.
async fn fetch_checked(
    url: reqwest::Url,
    timeout: Duration,
    max_bytes: u64,
) -> Result<(reqwest::Response, Option<u64>), UpdateError> {
    let client = build_client(timeout)?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| UpdateError::Network(error.to_string()))?;

    if !response.status().is_success() {
        return Err(UpdateError::Api {
            code: response.status().as_u16(),
        });
    }
    let total = response.content_length();
    if total.is_some_and(|length| length > max_bytes) {
        return Err(UpdateError::TooLarge { limit: max_bytes });
    }

    Ok((response, total))
}

/// 응답 본문을 청크 단위로 받아 `file`에 쓰면서 SHA-256을 함께 누적하고,
/// 청크마다 진행률을 알린다. 상한 초과 시 즉시 중단한다.
async fn stream_to_file(
    response: &mut reqwest::Response,
    mut file: std::fs::File,
    total: Option<u64>,
    max_bytes: u64,
    on_progress: &mut impl FnMut(DownloadProgress),
) -> Result<Digest, UpdateError> {
    let mut hasher = Hasher::new()?;
    let mut written: u64 = 0;

    // 0%를 먼저 알려 총량을 UI가 알 수 있게 한다. 첫 청크까지 시간이 걸려도
    // "다운로드 중..."에서 멈춰 있는 것처럼 보이지 않는다.
    on_progress(DownloadProgress { received: 0, total });

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| UpdateError::Network(error.to_string()))?
    {
        written = written.saturating_add(chunk.len() as u64);
        if written > max_bytes {
            return Err(UpdateError::TooLarge { limit: max_bytes });
        }
        hasher.update(&chunk)?;
        file.write_all(&chunk)?;
        on_progress(DownloadProgress {
            received: written,
            total,
        });
    }

    file.flush()?;
    file.sync_all()?;
    drop(file);

    Ok(hasher.finish()?)
}

/// `.sha256` asset을 받아 기대 다이제스트를 얻는다.
async fn fetch_expected_digest(url: &str, asset_name: &str) -> Result<Digest, UpdateError> {
    let parsed = reqwest::Url::parse(url).map_err(|error| UpdateError::Parse(error.to_string()))?;
    ensure_trusted_url(&parsed)?;
    let (response, _total) = fetch_checked(parsed, CHECK_TIMEOUT, MAX_CHECKSUM_BODY as u64).await?;

    let body = response
        .text()
        .await
        .map_err(|error| UpdateError::Network(error.to_string()))?;
    parse_checksum(&body, asset_name)
}

/// `<sha256 hex>  <파일명>` 형식에서 다이제스트를 뽑는다. 순수 함수다.
///
/// `scripts/verify-release.ps1`이 `eztrans_dll/checksums.sha256`에 쓰는 형식과
/// 같다. 파일명이 다르면 다른 asset의 해시를 잘못 적용하는 것이므로 거부한다.
pub(super) fn parse_checksum(text: &str, expected_name: &str) -> Result<Digest, UpdateError> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(hex) = parts.next() else {
            continue;
        };
        let name = parts.next().unwrap_or("");
        // 파일명이 없는 한 줄짜리 해시도 허용한다. 이름이 있으면 반드시 맞아야 한다.
        if !name.is_empty() && name != expected_name {
            continue;
        }
        return Digest::from_hex(hex).map_err(UpdateError::Hash);
    }
    Err(UpdateError::Parse(format!(
        "{expected_name}의 SHA-256 값을 찾을 수 없습니다"
    )))
}

#[cfg(test)]
#[path = "../../tests/unit/update/download.rs"]
mod tests;
