use super::*;

#[test]
fn decodes_utf16_with_nul_terminator() {
    let flags = (HookType::USING_STRING | HookType::CODEC_UTF16).bits();
    let text: Vec<u8> = "こんにちは"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .chain([0, 0])
        .collect();
    let decoded = decode_payload(flags, 0, &text);
    assert_eq!(decoded, "こんにちは");
}

#[test]
fn decodes_utf8_and_strips_control_chars() {
    let flags = (HookType::USING_STRING | HookType::CODEC_UTF8).bits();
    // 0x93 0x00 뒤의 쓰레기는 NUL에서 끊긴다. 0x93은 단독으로는 유효하지
    // 않은 UTF-8이므로 replacement char가 된다.
    let decoded = decode_payload(flags, 0, b"\x93\x00garbage");
    assert_eq!(decoded, "\u{fffd}");
}

/// USING_STRING이 없는 후크는 lunahook의 문자 단위 후크다. KiriKiri1/2가
/// 이 형태이므로 버리면 KiriKiri 게임에서 텍스트가 하나도 나오지 않는다.
#[test]
fn decodes_single_char_hooks_without_the_string_flag() {
    // KiriKiri1: CODEC_UTF16만. payload는 UTF-16 코드 유닛 하나.
    let decoded = decode_payload(HookType::CODEC_UTF16.bits(), 0, &0x3042u16.to_le_bytes());
    assert_eq!(decoded, "あ");

    // 명시적 USING_CHAR도 같은 문자 단위 규칙을 따른다.
    let decoded = decode_payload(HookType::USING_CHAR.bits(), 0, &[0x41]);
    assert_eq!(decoded, "A");
}

#[test]
fn decode_preserves_whitespace_payloads() {
    let flags = (HookType::USING_STRING | HookType::CODEC_UTF8).bits();
    let decoded = decode_payload(flags, 0, b" \0");
    assert_eq!(decoded, " ");
}

#[test]
fn decodes_utf32_with_nul_terminator() {
    let flags = (HookType::USING_STRING | HookType::CODEC_UTF32).bits();
    let mut bytes: Vec<u8> = "こんにちは"
        .chars()
        .flat_map(|ch| (ch as u32).to_le_bytes())
        .collect();
    bytes.extend_from_slice(&0u32.to_le_bytes());
    let decoded = decode_payload(flags, 0, &bytes);
    assert_eq!(decoded, "こんにちは");
}

/// codec 플래그가 전혀 없으면(ANSI 계열) codepage 932(Shift-JIS)를 기본으로
/// MultiByteToWideChar 경로를 탄다.
#[test]
fn decodes_shift_jis_ansi_payload_by_default() {
    let flags = HookType::USING_STRING.bits();
    // "あ"(U+3042)의 Shift-JIS 인코딩.
    let decoded = decode_payload(flags, 0, &[0x82, 0xA0, 0x00]);
    assert_eq!(decoded, "あ");
}

/// detected_codepage가 있으면 기본 932보다 우선한다.
#[test]
fn detected_codepage_overrides_the_default_ansi_codepage() {
    let flags = HookType::USING_STRING.bits();
    // "é"의 codepage 1252(Latin-1 계열) 인코딩.
    let decoded = decode_payload(flags, 1252, &[0xE9, 0x00]);
    assert_eq!(decoded, "é");
}

fn source(address: u64) -> HookSource {
    HookSource {
        address,
        context: 1,
        subcontext: 0,
    }
}

/// 한 글자씩 내는 후크는 2바이트 글자를 바이트 둘로 나눠 보낼 수 있다.
/// 선행 바이트 하나만 풀면 U+FFFD가 되므로 물었다가 짝을 지어야 한다.
#[test]
fn a_split_double_byte_character_is_paired_before_decoding() {
    let mut lead = LeadBytes::default();
    let flags = HookType::USING_CHAR.bits();

    assert_eq!(lead.decode(source(1), flags, 0, &[0x82]), "");
    assert_eq!(lead.decode(source(1), flags, 0, &[0xa0]), "あ");
}

/// 물고 있는 선행 바이트는 출처마다 따로다. 후크 둘이 번갈아 들어와도
/// 서로의 짝을 가져가면 안 된다.
#[test]
fn pending_lead_bytes_do_not_cross_sources() {
    let mut lead = LeadBytes::default();
    let flags = HookType::USING_CHAR.bits();

    assert_eq!(lead.decode(source(1), flags, 0, &[0x82]), "");
    assert_eq!(lead.decode(source(2), flags, 0, &[0x82]), "");
    assert_eq!(lead.decode(source(1), flags, 0, &[0xa0]), "あ");
    assert_eq!(lead.decode(source(2), flags, 0, &[0xa2]), "い");
}

/// 선행 바이트가 아닌 한 바이트는 그대로 푼다.
#[test]
fn a_single_ascii_byte_is_decoded_as_is() {
    let mut lead = LeadBytes::default();
    assert_eq!(
        lead.decode(source(1), HookType::USING_CHAR.bits(), 0, &[0x41]),
        "A"
    );
}

/// 바이트 단위 코드페이지가 아닌 후크는 짝짓기 대상이 아니다. UTF-8에서
/// 0x82는 선행 바이트가 아니라 그 자체로 깨진 바이트다.
#[test]
fn wide_codec_payloads_skip_the_pairing() {
    let mut lead = LeadBytes::default();
    let flags = HookType::CODEC_UTF8.bits();
    assert_eq!(lead.decode(source(1), flags, 0, &[0x82]), "\u{fffd}");
}

#[test]
fn normalize_collapses_consecutive_newlines_and_strips_control_chars() {
    let flags = (HookType::USING_STRING | HookType::CODEC_UTF8).bits();
    let decoded = decode_payload(flags, 0, b"a\r\n\nb\x0b\x0cc\0");
    assert_eq!(decoded, "a\nbc");
}
