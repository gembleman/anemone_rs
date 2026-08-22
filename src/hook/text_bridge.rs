//! TextOutput payload → 표시 가능한 UTF-8 문자열 변환과 갱신 병합.
//!
//! 디코딩 규칙은 lunahook_rs의 `HookType` 비트플래그를 따른다:
//! - `CODEC_UTF16` → UTF-16LE
//! - `CODEC_UTF8`  → UTF-8
//! - `CODEC_UTF32` → UTF-32LE
//! - 그 외(ANSI/Shift-JIS 계열) → codepage 932 가정. OS 변환
//!   (`MultiByteToWideChar`)을 쓴다 — dyncodec은 게임 글리프 리매핑 특수
//!   케이스용이고 일본어 게임 ANSI 텍스트는 CP932로 충분하다.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use lunahook_rs::params::HookType;

/// 후킹 텍스트 이벤트 하나가 UI로 넘어갈 준비가 된 형태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookText {
    /// `(thread_addr, thread_ctx)` — 텍스트 출처 식별자(후크+컨텍스트).
    pub source: (u64, u64),
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("문자열 후크가 아닌 payload입니다")]
    NotString,
}

/// payload 바이트를 hook_type 플래그에 따라 UTF-8로 디코딩한다.
pub fn decode_payload(
    flags: u64,
    detected_codepage: u16,
    payload: &[u8],
) -> Result<String, DecodeError> {
    let flags = HookType::from_bits_truncate(flags);
    if !flags.contains(HookType::USING_STRING) {
        return Err(DecodeError::NotString);
    }

    if flags.contains(HookType::CODEC_UTF16) {
        let units: Vec<u16> = payload
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .take_while(|unit| *unit != 0)
            .collect();
        return Ok(normalize(&String::from_utf16_lossy(&units)));
    }
    if flags.contains(HookType::CODEC_UTF8) {
        let end = payload
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(payload.len());
        return Ok(normalize(&String::from_utf8_lossy(&payload[..end])));
    }
    if flags.contains(HookType::CODEC_UTF32) {
        let chars: Vec<char> = payload
            .chunks_exact(4)
            .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .take_while(|value| *value != 0)
            .filter_map(char::from_u32)
            .collect();
        return Ok(normalize(&chars.iter().collect::<String>()));
    }

    // ANSI 계열: codepage 932(Shift-JIS) 기본. detected_codepage가 있으면 우선.
    let codepage = if detected_codepage != 0 {
        detected_codepage
    } else {
        932
    };
    Ok(normalize(&decode_ansi(payload, u32::from(codepage))))
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
    out.trim().to_string()
}

/// MultiByteToWideChar로 코드페이지 변환. 실패 시 lossy ASCII 폴백.
fn decode_ansi(bytes: &[u8], codepage: u32) -> String {
    use windows::Win32::Globalization::{MB_ERR_INVALID_CHARS, MultiByteToWideChar};

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
        let len = MultiByteToWideChar(codepage, MB_ERR_INVALID_CHARS, bytes, None);
        if len > 0 {
            let mut wide = vec![0u16; len as usize];
            let written =
                MultiByteToWideChar(codepage, MB_ERR_INVALID_CHARS, bytes, Some(&mut wide));
            if written > 0 {
                return String::from_utf16_lossy(&wide[..written as usize]);
            }
        }
    }
    // 변환 불가 바이트가 섞였으면 느슨하게라도 보여준다.
    String::from_utf8_lossy(bytes).into_owned()
}

/// 같은 출처(`source`)의 연속 이벤트를 짧은 시간 창 안에서 병합해,
/// 매 프레임 텍스트를 다시 보내는 갱신형 엔진이 번역 요청을 홍수내지 않게 한다.
///
/// - 창 안에 같은 출처의 새 이벤트가 오면 "마지막 값"으로 덮어쓴다 (최신 상태만 의미 있음)
/// - 창이 지나간 뒤 drain하면 출처별 최신 값만 나온다
#[derive(Debug, Default)]
pub struct TextMerger {
    window_ms: u64,
    pending: HashMap<(u64, u64), PendingText>,
}

#[derive(Debug)]
struct PendingText {
    text: String,
    first_seen: Instant,
}

impl TextMerger {
    pub fn new(window_ms: u64) -> Self {
        Self {
            window_ms,
            pending: HashMap::new(),
        }
    }

    /// 이벤트를 받아 창에 적립하고, 즉시 내보내도 되는 것들을 반환한다.
    ///
    /// 같은 출처가 이미 창에 있으면 값을 덮어쓰고(아직 내보내지 않음), 없으면
    /// 새로 적립한다. 반환값: 창이 만료된 출처들의 최신 텍스트.
    pub fn submit(&mut self, event: HookText) -> Vec<HookText> {
        self.flush_expired();
        let now = Instant::now();
        let entry = self
            .pending
            .entry(event.source)
            .or_insert_with(|| PendingText {
                text: event.text.clone(),
                first_seen: now,
            });
        entry.text = event.text;
        Vec::new()
    }

    /// 만료된 항목(첫 도착 후 window 경과)을 최신 값 하나로 방출한다.
    pub fn flush_expired(&mut self) -> Vec<HookText> {
        let now = Instant::now();
        let window = Duration::from_millis(self.window_ms);
        let expired: Vec<(u64, u64)> = self
            .pending
            .iter()
            .filter(|(_, pending)| now.duration_since(pending.first_seen) >= window)
            .map(|(source, _)| *source)
            .collect();
        expired
            .into_iter()
            .filter_map(|source| {
                self.pending.remove(&source).map(|pending| HookText {
                    source,
                    text: pending.text,
                })
            })
            .collect()
    }

    /// 종료/attach 해제 시 전부 방출한다.
    pub fn drain_all(&mut self) -> Vec<HookText> {
        self.pending
            .drain()
            .map(|(source, pending)| HookText {
                source,
                text: pending.text,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf16_with_nul_terminator() {
        let flags = (HookType::USING_STRING | HookType::CODEC_UTF16).bits();
        let text: Vec<u8> = "こんにちは"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .chain([0, 0])
            .collect();
        let decoded = decode_payload(flags, 0, &text).expect("utf16");
        assert_eq!(decoded, "こんにちは");
    }

    #[test]
    fn decodes_utf8_and_strips_control_chars() {
        let flags = (HookType::USING_STRING | HookType::CODEC_UTF8).bits();
        // 0x93 0x00 뒤의 쓰레기는 NUL에서 끊긴다. 0x93은 단독으로는 유효하지
        // 않은 UTF-8이므로 replacement char가 된다.
        let decoded = decode_payload(flags, 0, b"\x93\x00garbage").expect("utf8");
        assert_eq!(decoded, "\u{fffd}");
    }

    #[test]
    fn rejects_char_hooks() {
        assert!(matches!(
            decode_payload(HookType::USING_CHAR.bits(), 0, &[0x41]),
            Err(DecodeError::NotString)
        ));
    }

    #[test]
    fn merger_collapses_same_source_within_window() {
        let mut merger = TextMerger::new(10);
        assert!(
            merger
                .submit(HookText {
                    source: (1, 2),
                    text: "a".into()
                })
                .is_empty()
        );
        assert!(
            merger
                .submit(HookText {
                    source: (1, 2),
                    text: "ab".into()
                })
                .is_empty()
        );
        std::thread::sleep(std::time::Duration::from_millis(15));
        let flushed = merger.flush_expired();
        assert_eq!(flushed.len(), 1);
        assert_eq!(flushed[0].text, "ab");
        // flush 뒤에는 비어 있다.
        assert!(merger.drain_all().is_empty());
    }

    #[test]
    fn merger_keeps_distinct_sources_apart() {
        let mut merger = TextMerger::new(5);
        merger.submit(HookText {
            source: (1, 2),
            text: "a".into(),
        });
        merger.submit(HookText {
            source: (3, 4),
            text: "b".into(),
        });
        std::thread::sleep(std::time::Duration::from_millis(8));
        let mut texts: Vec<String> = merger
            .flush_expired()
            .into_iter()
            .map(|event| event.text)
            .collect();
        texts.sort();
        assert_eq!(texts, ["a".to_string(), "b".to_string()]);
    }
}
