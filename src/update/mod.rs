//! GitHub 릴리스 기반 반자동 업데이트.
//!
//! 새 버전 확인과 다운로드·검증은 앱이 하지만, 적용 시점은 항상 사용자가
//! 결정한다. 앱이 완전 포터블이라(`crate::runtime`) `config.toml`, `logs/`,
//! 캐시 DB가 실행 파일과 같은 폴더에 있으므로 **exe 파일 하나만** 교체한다.
//! `eztrans_dll/`은 사용자가 수정하는 사전을 담고 있어 대상에서 제외한다.

// 이 모듈은 아직 앱에 배선되지 않았다. check/download/apply와 UI가 붙으면
// 이 허용을 제거한다.
#![allow(dead_code)]

pub mod sha256;
pub mod version;

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
