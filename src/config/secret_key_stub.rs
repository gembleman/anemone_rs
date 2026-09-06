//! `secret_key.rs`가 없는 빌드의 대체 원재료.
//!
//! 값이 공개되어 있으므로 이 빌드의 `config.toml` 암호화는 난독화 이상이
//! 아니다. 그래도 형식은 같아야 한다 — 설정 파일을 읽고 쓰는 경로는 MyS
//! 엔진이 빠진 빌드에서도 그대로 돌아야 하기 때문이다.

/// 비공개 빌드와 같은 길이의 고정 원재료.
pub(super) fn material() -> [u8; 48] {
    *b"anemone.config.secret.v1.public-build-material.."
}
