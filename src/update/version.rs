//! 릴리스 태그 버전 파싱과 비교.
//!
//! GitHub 태그는 `v0.1.0` 형태이고 `CARGO_PKG_VERSION`은 `0.1.0` 형태다.
//! 두 표기를 같은 값으로 다루기 위해 선행 `v` 하나를 허용한다.
//!
//! semver crate를 쓰지 않는다. 필요한 것은 `major.minor.patch` 비교뿐이고
//! comparator 문법(`^1.2`, `>=1.0, <2.0`)은 전혀 사용하지 않으므로,
//! `docs/EXTERNAL_CRATE_AUDIT.md`의 "작은 편의를 위해 새 의존성을 넣지
//! 않는다"는 기준에 따라 직접 파싱한다.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

/// 릴리스 버전. 프리릴리스는 존재 여부만 기억하고 내부 순서는 비교하지 않는다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    major: u32,
    minor: u32,
    patch: u32,
    /// `-` 뒤 식별자. `0.2.0-rc1`의 `rc1`.
    pre: Option<String>,
}

impl Version {
    /// 현재 실행 중인 빌드의 버전.
    ///
    /// `CARGO_PKG_VERSION`은 빌드 시점에 Cargo가 검증한 값이라 파싱이 실패할 수
    /// 없다. 그럼에도 실패한다면 빌드 구성이 깨진 것이므로 즉시 드러내야 한다.
    pub fn current() -> Self {
        env!("CARGO_PKG_VERSION")
            .parse()
            .expect("CARGO_PKG_VERSION은 항상 유효한 버전이어야 한다")
    }

    pub fn is_prerelease(&self) -> bool {
        self.pre.is_some()
    }

    /// `candidate`로 업데이트할 수 있는지 판단한다.
    ///
    /// 프리릴리스는 후보에서 제외한다. `GET /releases/latest`가 이미
    /// prerelease와 draft를 걸러내지만, 태그를 잘못 붙였을 때 베타가 사용자에게
    /// 밀려나가는 사고를 앱에서도 이중으로 막는다.
    ///
    /// 반대로 현재 실행 중인 쪽이 프리릴리스면 같은 수치의 정식판으로 올라갈 수
    /// 있어야 한다 (`0.2.0-rc1` → `0.2.0`).
    pub fn is_upgrade_from(&self, current: &Version) -> bool {
        !self.is_prerelease() && self > current
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
            // 수치가 같으면 프리릴리스가 정식판보다 낮다. 프리릴리스끼리는
            // 식별자 순서를 정의하지 않고 같은 것으로 취급한다.
            .then(match (&self.pre, &other.pre) {
                (None, None) | (Some(_), Some(_)) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

impl FromStr for Version {
    type Err = VersionParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let text = value.trim();
        // 태그(`v0.1.0`)와 Cargo 버전(`0.1.0`)을 모두 받는다.
        let text = text.strip_prefix('v').unwrap_or(text);

        // 빌드 메타데이터(`+build`)는 순서에 영향을 주지 않으므로 버린다.
        let text = text.split('+').next().unwrap_or(text);

        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) if !pre.is_empty() => (core, Some(pre.to_string())),
            Some(_) => return Err(VersionParseError::new(value)),
            None => (text, None),
        };

        let mut parts = core.split('.');
        let mut next = || {
            parts
                .next()
                .filter(|part| !part.is_empty())
                .and_then(|part| part.parse::<u32>().ok())
                .ok_or_else(|| VersionParseError::new(value))
        };
        let major = next()?;
        let minor = next()?;
        let patch = next()?;
        if parts.next().is_some() {
            return Err(VersionParseError::new(value));
        }

        Ok(Self {
            major,
            minor,
            patch,
            pre,
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("버전 형식을 해석할 수 없습니다: {value}")]
pub struct VersionParseError {
    value: String,
}

impl VersionParseError {
    fn new(value: &str) -> Self {
        Self {
            value: value.to_string(),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/update/version.rs"]
mod tests;
