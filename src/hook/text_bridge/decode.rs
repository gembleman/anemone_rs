//! TextOutput payload → 표시 가능한 UTF-8 문자열 변환.
//!
//! 디코딩 규칙은 lunahook_rs의 `HookType` 비트플래그를 따른다:
//! - `CODEC_UTF16` → UTF-16LE
//! - `CODEC_UTF8`  → UTF-8
//! - `CODEC_UTF32` → UTF-32LE
//! - 그 외(ANSI/Shift-JIS 계열) → codepage 932 가정. OS 변환
//!   (`MultiByteToWideChar`)을 쓴다 — dyncodec은 게임 글리프 리매핑 특수
//!   케이스용이고 일본어 게임 ANSI 텍스트는 CP932로 충분하다.
//!
//! `USING_STRING`이 없는 후크는 lunahook에서 "문자 단위" 후크다
//! (`host/wire.rs`의 `single_char` 판정과 같은 규칙). payload가 문자열 대신
//! 문자 하나라는 차이뿐이고 codec 해석은 완전히 같으므로 여기서 특별히 다루지
//! 않는다. 조각을 문장으로 잇는 일은 [`super::merger::TextMerger`]가 맡는다.
//! 이 payload를 버리면 KiriKiri1/KiriKiri2처럼 문자 단위로만 텍스트를 내보내는
//! 엔진에서 문장이 하나도 나오지 않는다.

use std::collections::HashMap;

use lunahook_rs::params::HookType;

use super::merger::HookSource;

/// 원본 `LunaHost/textthread.cpp`의 `TextThread::leadByte`다.
///
/// 한 글자씩 내는 후크는 payload가 한 바이트로 올 수 있다. 2바이트 글자의
/// 선행 바이트 하나만 풀면 어떤 코드페이지에서도 글자가 되지 않는다.
/// `MB_ERR_INVALID_CHARS` 때문에 변환이 실패하고 U+FFFD가 된다. 물고 있다가
/// 다음 바이트와 붙여 한 글자로 만든다. 출처마다 따로 문다.
#[derive(Debug, Default)]
pub struct LeadBytes {
    pending: HashMap<HookSource, u8>,
}

impl LeadBytes {
    /// payload를 디코딩한다. 선행 바이트를 물었으면 빈 문자열을 낸다.
    pub fn decode(
        &mut self,
        source: HookSource,
        flags: u64,
        detected_codepage: u16,
        payload: &[u8],
    ) -> String {
        let [byte] = payload else {
            return decode_payload(flags, detected_codepage, payload);
        };
        if !is_ansi_codec(flags) {
            return decode_payload(flags, detected_codepage, payload);
        }
        if let Some(lead) = self.pending.remove(&source) {
            return decode_payload(flags, detected_codepage, &[lead, *byte]);
        }
        if is_dbcs_lead_byte(ansi_codepage(detected_codepage), *byte) {
            self.pending.insert(source, *byte);
            return String::new();
        }
        decode_payload(flags, detected_codepage, payload)
    }
}

/// 바이트 단위 코드페이지 텍스트인지 본다. 원본 `HookParam::isAscii`와 같다.
fn is_ansi_codec(flags: u64) -> bool {
    !HookType::from_bits_truncate(flags).intersects(
        HookType::CODEC_UTF8
            .union(HookType::CODEC_UTF16)
            .union(HookType::CODEC_UTF32),
    )
}

fn is_dbcs_lead_byte(codepage: u32, byte: u8) -> bool {
    // SAFETY: 인자가 값 두 개뿐인 순수 조회 함수다.
    unsafe { windows_sys::Win32::Globalization::IsDBCSLeadByteEx(codepage, byte) != 0 }
}

/// ANSI 계열의 코드페이지. `detected_codepage`가 있으면 그것을, 없으면 932다.
fn ansi_codepage(detected_codepage: u16) -> u32 {
    if detected_codepage != 0 {
        u32::from(detected_codepage)
    } else {
        932
    }
}

/// payload 바이트를 hook_type 플래그에 따라 UTF-8로 디코딩한다.
pub fn decode_payload(flags: u64, detected_codepage: u16, payload: &[u8]) -> String {
    let flags = HookType::from_bits_truncate(flags);

    if flags.contains(HookType::CODEC_UTF16) {
        let units: Vec<u16> = payload
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(u16::from_le_bytes)
            .take_while(|unit| *unit != 0)
            .collect();
        return normalize(&String::from_utf16_lossy(&units));
    }
    if flags.contains(HookType::CODEC_UTF8) {
        let end = payload
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(payload.len());
        return normalize(&String::from_utf8_lossy(&payload[..end]));
    }
    if flags.contains(HookType::CODEC_UTF32) {
        let chars: Vec<char> = payload
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(u32::from_le_bytes)
            .take_while(|value| *value != 0)
            .filter_map(char::from_u32)
            .collect();
        return normalize(&chars.iter().collect::<String>());
    }

    // ANSI 계열: codepage 932(Shift-JIS) 기본. detected_codepage가 있으면 우선.
    normalize(&decode_ansi(payload, ansi_codepage(detected_codepage)))
}

/// 제어 문자 제거 + 개행 정규화. 게임 엔진마다 `\n`, 세로 탭 등이 섞인다.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\r' | '\u{b}' | '\u{c}' => {}
            '\n' => {
                // 연속 개행도 하나로 접는다 — 오버레이는 문장 단위 표시가 목적.
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    // 입력 payload는 문장 전체가 아니라 병합할 조각일 수 있다. 여기서
    // trim하면 공백 하나짜리 조각과 조각 경계의 공백이 사라진다.
    out
}

/// MultiByteToWideChar로 코드페이지 변환. 실패 시 lossy ASCII 폴백.
fn decode_ansi(bytes: &[u8], codepage: u32) -> String {
    use windows_sys::Win32::Globalization::{MB_ERR_INVALID_CHARS, MultiByteToWideChar};

    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let bytes = &bytes[..end];
    if bytes.is_empty() {
        return String::new();
    }
    // SAFETY: bytes는 유효한 읽기 버퍼이고, 첫 호출로 필요한 길이를 얻은 뒤
    // 두 번째 호출에서 같은 길이의 쓰기 가능 버퍼를 넘긴다.
    unsafe {
        let len = MultiByteToWideChar(
            codepage,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            bytes.len() as i32,
            std::ptr::null_mut(),
            0,
        );
        if len > 0 {
            let mut wide = vec![0u16; len as usize];
            let written = MultiByteToWideChar(
                codepage,
                MB_ERR_INVALID_CHARS,
                bytes.as_ptr(),
                bytes.len() as i32,
                wide.as_mut_ptr(),
                wide.len() as i32,
            );
            if written > 0 {
                return String::from_utf16_lossy(&wide[..written as usize]);
            }
        }
    }
    // 변환 불가 바이트가 섞였으면 느슨하게라도 보여준다.
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
#[path = "../../../tests/unit/hook/text_bridge/decode.rs"]
mod tests;
