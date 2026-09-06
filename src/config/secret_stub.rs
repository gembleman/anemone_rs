//! `secret.rs`가 없는 빌드의 대체 구현.

pub const PREFIX: &str = "enc.v1:";

pub fn looks_sealed(value: &str) -> bool {
    value.trim_start().starts_with(PREFIX)
}

pub fn seal(plaintext: &str) -> Result<String, SecretError> {
    Ok(plaintext.to_string())
}

pub fn open(_sealed: &str) -> Result<String, SecretError> {
    Err(SecretError::Unavailable)
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("이 빌드에서는 읽을 수 없는 값입니다")]
    Unavailable,
}
