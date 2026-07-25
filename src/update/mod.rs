//! GitHub 릴리스 기반 반자동 업데이트.
//!
//! 새 버전 확인과 다운로드·검증은 앱이 하지만, 적용 시점은 항상 사용자가
//! 결정한다. 앱이 완전 포터블이라(`crate::runtime`) `config.toml`, `logs/`,
//! 캐시 DB가 실행 파일과 같은 폴더에 있으므로 **exe 파일 하나만** 교체한다.
//! `eztrans_dll/`은 사용자가 수정하는 사전을 담고 있어 대상에서 제외한다.

// 이 모듈은 아직 앱에 배선되지 않았다. check/download/apply와 UI가 붙으면
// 이 허용을 제거한다.
#![allow(dead_code)]

pub mod apply;
pub mod check;
pub mod download;
pub(crate) mod schedule;
pub mod sha256;
pub mod version;
pub mod worker;

use std::time::Duration;

pub use check::AvailableUpdate;
pub use version::Version;

/// 릴리스를 조회할 저장소.
pub const GITHUB_OWNER: &str = "gembleman";
pub const GITHUB_REPO: &str = "anemone_rs";

/// 자동 업데이트용 asset 이름.
///
/// **CI(`.github/workflows/release.yml`)가 업로드하는 이름과 정확히 같아야
/// 한다.** 어긋나면 앱이 조용히 "업데이트 없음"으로 동작해 발견이 늦어지므로,
/// 규칙을 이 상수 하나로만 관리하고 CI가 여기서 값을 읽어 간다.
///
/// 최초 사용자용 full zip(exe + `eztrans_dll/`)은 사람이 직접 게시하며 앱은
/// 그것을 절대 내려받지 않는다.
pub const UPDATE_ASSET_NAME: &str = "anemone_rs-i686-pc-windows-msvc.exe";

/// 무결성 검증용 체크섬 asset. `<sha256 hex>  <파일명>` 한 줄이다.
/// `scripts/verify-release.ps1`이 쓰는 형식과 같다.
pub const CHECKSUM_ASSET_NAME: &str = "anemone_rs-i686-pc-windows-msvc.exe.sha256";

/// GitHub API는 User-Agent가 없으면 403을 반환한다.
/// `src/translation/deepl.rs`가 쓰는 형식과 같다.
const USER_AGENT: &str = concat!("AnemoneRS/", env!("CARGO_PKG_VERSION"));

/// 리다이렉트를 포함해 접속을 허용하는 호스트.
///
/// asset 다운로드는 CDN으로 302되므로 리다이렉트 자체를 막을 수는 없다.
/// 대신 각 홉의 호스트를 이 목록과 **정확히** 대조한다. 문자열 `contains`로
/// 비교하면 `evil.com/api.github.com` 같은 URL에 속는다.
const ALLOWED_HOSTS: &[&str] = &[
    "api.github.com",
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// 릴리스 조회 응답 본문 상한. 실제 응답은 수십 KB 수준이다.
const MAX_API_BODY: usize = 512 * 1024;
/// 체크섬 파일 상한. 한 줄이면 100바이트 미만이다.
const MAX_CHECKSUM_BODY: usize = 4 * 1024;
/// 업데이트 exe 상한. 현재 릴리스 빌드가 약 5.5 MB다.
const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 릴리스 조회는 짧게 끊는다.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// 느린 회선에서 수 MB를 받을 수 있도록 다운로드는 넉넉히 잡는다.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// 업데이트 확인·다운로드 과정에서 발생하는 오류.
///
/// 메시지는 그대로 사용자에게 보일 수 있어야 하므로 원인별로 나눈다.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    // 사용자에게는 앞 문장만 보이면 되지만, 원인을 잃으면 진단이 불가능하다.
    // UI는 이 오류를 그대로 노출하지 않고 짧은 안내 문구로 바꿔 표시한다.
    #[error("업데이트 서버에 연결할 수 없습니다. 인터넷 연결을 확인해주세요. ({0})")]
    Network(String),
    #[error("확인 요청이 많아 잠시 제한되었습니다. 1시간 뒤 다시 시도해주세요.")]
    RateLimited,
    #[error("업데이트 서버가 오류를 반환했습니다 (HTTP {code})")]
    Api { code: u16 },
    #[error("업데이트 정보를 해석할 수 없습니다: {0}")]
    Parse(String),
    #[error("허용되지 않은 주소로 연결을 시도했습니다: {0}")]
    UntrustedUrl(String),
    #[error("내려받을 데이터가 너무 큽니다 (상한 {limit} 바이트)")]
    TooLarge { limit: u64 },
    #[error("내려받은 파일이 손상되었습니다. 다시 시도해주세요.")]
    ChecksumMismatch,
    #[error("해시를 계산할 수 없습니다: {0}")]
    Hash(#[from] sha256::HashError),
    #[error(
        "이 폴더에 쓸 수 없어 자동 업데이트를 적용할 수 없습니다. \
         릴리스 페이지에서 직접 내려받아주세요. ({0})"
    )]
    NotWritable(std::path::PathBuf),
    /// 교체에 실패한 뒤 되돌리기까지 실패한 상태.
    ///
    /// 실행 파일이 백업 이름으로만 남아 있어 다음 실행이 불가능하다.
    /// UI는 이 경우 반드시 `backup` 경로를 사용자에게 그대로 보여줘야 한다.
    #[error(
        "업데이트에 실패했고 이전 버전으로 되돌리지도 못했습니다. \
         {backup} 파일의 확장자를 .exe로 바꿔주세요. (원인: {cause})"
    )]
    RollbackFailed {
        backup: std::path::PathBuf,
        cause: String,
    },
    #[error("파일을 저장할 수 없습니다: {0}")]
    Io(#[from] std::io::Error),
}

/// URL이 https이고 허용된 호스트인지 확인한다.
fn ensure_trusted_url(url: &reqwest::Url) -> Result<(), UpdateError> {
    if url.scheme() != "https" {
        return Err(UpdateError::UntrustedUrl(url.to_string()));
    }
    match url.host_str() {
        Some(host) if ALLOWED_HOSTS.contains(&host) => Ok(()),
        _ => Err(UpdateError::UntrustedUrl(url.to_string())),
    }
}

/// 업데이트 전용 HTTP 클라이언트.
///
/// 번역용 클라이언트(`translation::http_common`)와 타임아웃·리다이렉트 정책이
/// 달라 따로 만든다. rustls provider 설치는 프로세스 전역이고 `install_default`가
/// 멱등하므로, 번역 쪽이 먼저 설치했든 아니든 여기서 한 번 더 시도해도 안전하다.
/// 그 덕에 두 모듈 사이에 초기화 순서 의존이 생기지 않는다.
fn build_client(timeout: Duration) -> Result<reqwest::Client, UpdateError> {
    static INSTALL_PROVIDER: std::sync::Once = std::sync::Once::new();
    INSTALL_PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });

    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(USER_AGENT)
        // 리다이렉트마다 호스트를 검사한다. https 다운그레이드도 여기서 막힌다.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("리다이렉트가 너무 많습니다");
            }
            match ensure_trusted_url(attempt.url()) {
                Ok(()) => attempt.follow(),
                Err(error) => attempt.error(error.to_string()),
            }
        }))
        .build()
        .map_err(|error| UpdateError::Network(error.to_string()))
}

#[cfg(test)]
#[path = "../../tests/unit/update/mod.rs"]
mod tests;
