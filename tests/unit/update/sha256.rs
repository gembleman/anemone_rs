use super::{Digest, Hasher, digest};

/// NIST 표준 테스트 벡터. 구현이 실제 SHA-256인지 고정한다.
const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn known_vectors_match_the_reference_digests() {
    assert_eq!(digest(b"").to_string(), EMPTY);
    assert_eq!(digest(b"abc").to_string(), ABC);
}

#[test]
fn streaming_in_chunks_matches_a_single_pass() {
    let data: Vec<u8> = (0..4096u32).map(|value| (value % 251) as u8).collect();

    let one_shot = digest(&data);

    let mut hasher = Hasher::new();
    for chunk in data.chunks(97) {
        hasher.update(chunk);
    }
    assert_eq!(hasher.finish(), one_shot);
}

#[test]
fn empty_updates_do_not_change_the_digest() {
    let mut hasher = Hasher::new();
    hasher.update(b"");
    hasher.update(b"abc");
    hasher.update(b"");
    assert_eq!(hasher.finish().to_string(), ABC);
}

#[test]
fn hex_parsing_round_trips() {
    let parsed = Digest::from_hex(ABC).unwrap();
    assert_eq!(parsed.to_string(), ABC);
    assert_eq!(parsed, digest(b"abc"));
}

#[test]
fn uppercase_hex_is_accepted() {
    assert_eq!(
        Digest::from_hex(&ABC.to_uppercase()).unwrap().to_string(),
        ABC
    );
}

#[test]
fn malformed_hex_is_rejected() {
    for text in [
        "",
        "abc",
        &ABC[..62],
        &format!("{ABC}00"),
        &"z".repeat(64),
        &format!("{} ", &ABC[..63]),
    ] {
        assert!(
            Digest::from_hex(text).is_err(),
            "{text:?}는 오류로 처리해야 한다"
        );
    }
}

#[test]
fn different_inputs_produce_different_digests() {
    assert_ne!(digest(b"anemone"), digest(b"anemone "));
}
