//! `ring`을 사용하는 SHA-256.
//!
//! 업데이트 asset 무결성 검증에 쓴다. `ring`은 `rustls`가 이미 끌어오는
//! 의존성이라 이 모듈을 위해 새로 컴파일되는 크레이트는 없다.

use std::fmt;

use ring::digest::{Context, SHA256};

pub const DIGEST_LEN: usize = 32;

/// SHA-256 다이제스트. 16진 문자열과 상호 변환한다.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Digest([u8; DIGEST_LEN]);

impl Digest {
    /// 소문자 16진 64자를 파싱한다. 대문자도 허용한다.
    pub fn from_hex(text: &str) -> Result<Self, HashError> {
        let text = text.trim();
        if text.len() != DIGEST_LEN * 2 {
            return Err(HashError::InvalidHex);
        }
        let mut bytes = [0u8; DIGEST_LEN];
        for (index, chunk) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            let pair = std::str::from_utf8(chunk).map_err(|_| HashError::InvalidHex)?;
            bytes[index] = u8::from_str_radix(pair, 16).map_err(|_| HashError::InvalidHex)?;
        }
        Ok(Self(bytes))
    }

    /// 원시 다이제스트. 16진 표기를 거치지 않고 키 유도에 쓸 때 필요하다.
    /// 빌드 구성에 따라 호출부가 없을 수 있다.
    #[allow(dead_code)]
    pub fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// 다이제스트는 비밀이 아니지만 로그를 읽기 쉽게 16진으로 표시한다.
impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Digest({self})")
    }
}

/// 스트리밍 해시 계산기. 다운로드하면서 청크 단위로 먹인다.
pub struct Hasher(Context);

impl Hasher {
    pub fn new() -> Self {
        Self(Context::new(&SHA256))
    }

    pub fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    /// 다이제스트를 확정한다. 호출 후 이 hasher는 재사용할 수 없다.
    pub fn finish(self) -> Digest {
        let mut bytes = [0u8; DIGEST_LEN];
        // SHA-256 다이제스트는 항상 DIGEST_LEN이므로 길이가 어긋날 수 없다.
        bytes.copy_from_slice(self.0.finish().as_ref());
        Digest(bytes)
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

/// 한 번에 전체 버퍼를 해시한다.
///
/// 다운로드 검증(`download.rs`)은 스트리밍 `Hasher`를 쓰므로, 빌드 구성에
/// 따라서는 이 one-shot 헬퍼의 호출부가 없을 수 있다.
#[allow(dead_code)]
pub fn digest(data: &[u8]) -> Digest {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finish()
}

#[derive(Debug, thiserror::Error)]
pub enum HashError {
    #[error("SHA-256 값 형식이 올바르지 않습니다")]
    InvalidHex,
}

#[cfg(test)]
#[path = "../../tests/unit/update/sha256.rs"]
mod tests;
