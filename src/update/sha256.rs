//! Windows CNG(bcrypt)를 사용하는 SHA-256.
//!
//! 업데이트 asset 무결성 검증에만 쓴다. 해시 crate를 새로 추가하는 대신 이미
//! 의존하고 있는 `windows` crate의 CNG를 쓴다
//! (`docs/EXTERNAL_CRATE_AUDIT.md`의 의존성 최소화 기준).
//!
//! `Cargo.toml`의 release profile이 `panic = "abort"`라 unwind 정리가 없으므로,
//! 두 핸들 모두 `Drop`으로 해제를 보장한다.

use std::fmt;

use windows::Win32::Security::Cryptography::{
    BCRYPT_ALG_HANDLE, BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALGORITHM, BCryptCloseAlgorithmProvider,
    BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash, BCryptHashData,
    BCryptOpenAlgorithmProvider,
};

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
        for (index, chunk) in text.as_bytes().chunks_exact(2).enumerate() {
            let pair = std::str::from_utf8(chunk).map_err(|_| HashError::InvalidHex)?;
            bytes[index] = u8::from_str_radix(pair, 16).map_err(|_| HashError::InvalidHex)?;
        }
        Ok(Self(bytes))
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
pub struct Hasher {
    // 선언 순서가 drop 순서다. hash를 먼저 파괴한 뒤 provider를 닫아야 한다.
    hash: HashHandle,
    _algorithm: AlgorithmHandle,
}

impl Hasher {
    pub fn new() -> Result<Self, HashError> {
        // SAFETY: 출력 핸들 포인터가 유효하고, 실패 시 핸들은 null로 남는다.
        let algorithm = unsafe {
            let mut handle = BCRYPT_ALG_HANDLE::default();
            BCryptOpenAlgorithmProvider(
                &mut handle,
                BCRYPT_SHA256_ALGORITHM,
                None,
                Default::default(),
            )
            .ok()
            .map_err(|error| HashError::Cng(error.to_string()))?;
            AlgorithmHandle(handle)
        };

        // SAFETY: algorithm은 위에서 성공적으로 열렸다. hash object 버퍼로 None을
        // 넘기면 CNG가 직접 할당한다(Win8+). 타깃이 Win10이라 문제없다.
        let hash = unsafe {
            let mut handle = BCRYPT_HASH_HANDLE::default();
            BCryptCreateHash(algorithm.0, &mut handle, None, None, 0)
                .ok()
                .map_err(|error| HashError::Cng(error.to_string()))?;
            HashHandle(handle)
        };

        Ok(Self {
            hash,
            _algorithm: algorithm,
        })
    }

    pub fn update(&mut self, data: &[u8]) -> Result<(), HashError> {
        if data.is_empty() {
            return Ok(());
        }
        // SAFETY: hash 핸들이 유효하고 data 슬라이스 길이가 함께 전달된다.
        unsafe {
            BCryptHashData(self.hash.0, data, 0)
                .ok()
                .map_err(|error| HashError::Cng(error.to_string()))
        }
    }

    /// 다이제스트를 확정한다. 호출 후 이 hasher는 재사용할 수 없다.
    pub fn finish(self) -> Result<Digest, HashError> {
        let mut digest = [0u8; DIGEST_LEN];
        // SAFETY: 출력 버퍼가 SHA-256 다이제스트 길이와 정확히 같다.
        unsafe {
            BCryptFinishHash(self.hash.0, &mut digest, 0)
                .ok()
                .map_err(|error| HashError::Cng(error.to_string()))?;
        }
        Ok(Digest(digest))
    }
}

/// 한 번에 전체 버퍼를 해시한다.
pub fn digest(data: &[u8]) -> Result<Digest, HashError> {
    let mut hasher = Hasher::new()?;
    hasher.update(data)?;
    hasher.finish()
}

struct AlgorithmHandle(BCRYPT_ALG_HANDLE);

impl Drop for AlgorithmHandle {
    fn drop(&mut self) {
        // SAFETY: 생성에 성공한 핸들만 여기 도달하며 한 번만 닫는다.
        unsafe {
            let _ = BCryptCloseAlgorithmProvider(self.0, 0);
        }
    }
}

struct HashHandle(BCRYPT_HASH_HANDLE);

impl Drop for HashHandle {
    fn drop(&mut self) {
        // SAFETY: 생성에 성공한 핸들만 여기 도달하며 한 번만 파괴한다.
        unsafe {
            let _ = BCryptDestroyHash(self.0);
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HashError {
    #[error("해시 계산에 실패했습니다: {0}")]
    Cng(String),
    #[error("SHA-256 값 형식이 올바르지 않습니다")]
    InvalidHex,
}

#[cfg(test)]
#[path = "../../tests/unit/update/sha256.rs"]
mod tests;
