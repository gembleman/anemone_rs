//! GitHub Releases API로 최신 릴리스를 조회한다.
//!
//! 응답 해석은 네트워크와 분리해 순수 함수로 두고 단위 테스트로 고정한다.

use serde::Deserialize;

use super::{
    CHECK_TIMEOUT, CHECKSUM_ASSET_NAME, GITHUB_OWNER, GITHUB_REPO, MAX_API_BODY, UPDATE_ASSET_NAME,
    UpdateError, Version, build_client, ensure_trusted_url,
};

/// 업데이트 확인 결과.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateCheck {
    /// 현재 버전이 최신이다.
    UpToDate,
    /// 새 버전을 받을 수 있다.
    Available(AvailableUpdate),
    /// 새 버전은 있지만 자동으로 적용할 수 없다.
    ///
    /// asset 이름이 어긋났거나 체크섬이 빠진 경우다. **절대 `UpToDate`로
    /// 뭉개지 않는다** — 조용히 넘기면 배포 실수를 오래 발견하지 못한다.
    Unsupported { version: Version, reason: String },
}

/// 내려받을 수 있는 새 버전.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub version: Version,
    pub asset_url: String,
    pub checksum_url: String,
    pub release_page_url: String,
}

/// 최신 릴리스를 조회해 현재 버전과 비교한다.
pub async fn fetch_latest(current: &Version) -> Result<UpdateCheck, UpdateError> {
    let url =
        format!("https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases?per_page=100");
    let parsed =
        reqwest::Url::parse(&url).map_err(|error| UpdateError::Parse(error.to_string()))?;
    ensure_trusted_url(&parsed)?;

    let client = build_client(CHECK_TIMEOUT)?;
    let response = client
        .get(parsed)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|error| UpdateError::Network(error.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        // 미인증 호출은 시간당 60회다. 소진 시 403 + remaining: 0으로 온다.
        let exhausted = response
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.trim() == "0");
        if status.as_u16() == 429 || (status.as_u16() == 403 && exhausted) {
            return Err(UpdateError::RateLimited);
        }
        return Err(UpdateError::Api {
            code: status.as_u16(),
        });
    }

    let body = read_limited(response, MAX_API_BODY).await?;
    parse_releases(&body, current)
}

/// 본문을 상한까지만 읽는다. 응답이 상한을 넘으면 즉시 중단한다.
async fn read_limited(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<String, UpdateError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(UpdateError::TooLarge {
            limit: limit as u64,
        });
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| UpdateError::Network(error.to_string()))?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(UpdateError::TooLarge {
                limit: limit as u64,
            });
        }
        bytes.extend_from_slice(&chunk);
    }

    String::from_utf8(bytes).map_err(|error| UpdateError::Parse(error.to_string()))
}

/// 릴리스 목록에서 정식 버전 태그 중 가장 높은 것을 골라 업데이트 가능 여부를 판단한다.
pub(super) fn parse_releases(body: &str, current: &Version) -> Result<UpdateCheck, UpdateError> {
    let releases: Vec<Release> =
        serde_json::from_str(body).map_err(|error| UpdateError::Parse(error.to_string()))?;

    let candidate = releases
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| {
            let version: Version = release.tag_name.parse().ok()?;
            (!version.is_prerelease()).then_some((release, version))
        })
        .max_by(|(_, left), (_, right)| left.cmp(right));

    let Some((release, version)) = candidate else {
        return Ok(UpdateCheck::UpToDate);
    };

    evaluate_release(release, version, current)
}

fn evaluate_release(
    release: Release,
    version: Version,
    current: &Version,
) -> Result<UpdateCheck, UpdateError> {
    if !version.is_upgrade_from(current) {
        return Ok(UpdateCheck::UpToDate);
    }

    let unsupported = |reason: &str| {
        Ok(UpdateCheck::Unsupported {
            version: version.clone(),
            reason: reason.to_string(),
        })
    };

    // 이름이 정확히 일치하는 asset만 고른다. 최초 사용자용 full zip을 실수로
    // 집지 않기 위해 부분 일치를 허용하지 않는다.
    let Some(asset) = release.asset(UPDATE_ASSET_NAME) else {
        return unsupported("이 릴리스에는 자동 업데이트 파일이 없습니다");
    };
    let Some(checksum) = release.asset(CHECKSUM_ASSET_NAME) else {
        return unsupported("이 릴리스에는 무결성 검증 파일이 없습니다");
    };

    for url in [&asset.browser_download_url, &checksum.browser_download_url] {
        let parsed =
            reqwest::Url::parse(url).map_err(|error| UpdateError::Parse(error.to_string()))?;
        ensure_trusted_url(&parsed)?;
    }

    Ok(UpdateCheck::Available(AvailableUpdate {
        version,
        asset_url: asset.browser_download_url.clone(),
        checksum_url: checksum.browser_download_url.clone(),
        release_page_url: release.html_url,
    }))
}

/// 응답에서 실제로 쓰는 필드만 받는다. 나머지는 serde가 무시한다.
#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

impl Release {
    fn asset(&self, name: &str) -> Option<&ReleaseAsset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    #[serde(default)]
    browser_download_url: String,
}

#[cfg(test)]
#[path = "../../tests/unit/update/check.rs"]
mod tests;
