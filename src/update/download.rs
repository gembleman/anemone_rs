//! 업데이트 asset을 내려받고 SHA-256으로 검증한다.
//!
//! 코드 서명이 없으므로 이 해시는 **전송 무결성**만 보증한다. 출처 신뢰는
//! TLS와 호스트 고정(`ensure_trusted_url`)에 의존한다.

use std::io::Write;
use std::path::{Path, PathBuf};

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

/// asset을 받아 `destination`에 저장하고 해시를 검증한다.
///
/// `destination`은 교체 대상 exe와 **같은 볼륨**이어야 한다. 그래야 이후 교체가
/// 볼륨 간 복사 없이 rename으로 끝난다.
pub async fn download(
    update: &AvailableUpdate,
    destination: PathBuf,
) -> Result<StagedUpdate, UpdateError> {
    download_with_asset_name(update, destination, UPDATE_ASSET_NAME).await
}

/// 체크섬 매니페스트에서 찾을 파일명을 지정할 수 있는 형태.
/// 실제 릴리스는 항상 `UPDATE_ASSET_NAME`을 쓰고, 이 인자는 테스트용이다.
pub(super) async fn download_with_asset_name(
    update: &AvailableUpdate,
    destination: PathBuf,
    asset_name: &str,
) -> Result<StagedUpdate, UpdateError> {
    let expected = fetch_expected_digest(&update.checksum_url, asset_name).await?;

    let url = reqwest::Url::parse(&update.asset_url)
        .map_err(|error| UpdateError::Parse(error.to_string()))?;
    ensure_trusted_url(&url)?;

    let client = build_client(DOWNLOAD_TIMEOUT)?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| UpdateError::Network(error.to_string()))?;

    if !response.status().is_success() {
        return Err(UpdateError::Api {
            code: response.status().as_u16(),
        });
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ASSET_BYTES)
    {
        return Err(UpdateError::TooLarge {
            limit: MAX_ASSET_BYTES,
        });
    }

    // 파일을 만든 직후부터 Drop이 정리를 책임지도록 staged를 먼저 세운다.
    let staged = StagedUpdate {
        path: destination,
        applied: false,
    };
    let mut file = std::fs::File::create(&staged.path)?;
    let mut hasher = Hasher::new()?;
    let mut written: u64 = 0;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| UpdateError::Network(error.to_string()))?
    {
        written = written.saturating_add(chunk.len() as u64);
        if written > MAX_ASSET_BYTES {
            return Err(UpdateError::TooLarge {
                limit: MAX_ASSET_BYTES,
            });
        }
        hasher.update(&chunk)?;
        file.write_all(&chunk)?;
    }

    file.flush()?;
    file.sync_all()?;
    drop(file);

    let actual = hasher.finish()?;
    if actual != expected {
        tracing::warn!(
            "업데이트 해시 불일치: 기대 {expected}, 실제 {actual}. 내려받은 파일을 폐기합니다."
        );
        return Err(UpdateError::ChecksumMismatch);
    }

    Ok(staged)
}

/// `.sha256` asset을 받아 기대 다이제스트를 얻는다.
async fn fetch_expected_digest(url: &str, asset_name: &str) -> Result<Digest, UpdateError> {
    let parsed = reqwest::Url::parse(url).map_err(|error| UpdateError::Parse(error.to_string()))?;
    ensure_trusted_url(&parsed)?;

    let client = build_client(CHECK_TIMEOUT)?;
    let response = client
        .get(parsed)
        .send()
        .await
        .map_err(|error| UpdateError::Network(error.to_string()))?;

    if !response.status().is_success() {
        return Err(UpdateError::Api {
            code: response.status().as_u16(),
        });
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_CHECKSUM_BODY as u64)
    {
        return Err(UpdateError::TooLarge {
            limit: MAX_CHECKSUM_BODY as u64,
        });
    }

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
