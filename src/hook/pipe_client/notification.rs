//! hook → host 알림 wire 디코딩.
//!
//! wire 바이트를 곧장 `RawHookParam`으로 재구성하지 않는다 — 그 안의 `JitType`
//! 은 무효 판별값일 수 있는(`#[repr(u32)]` 필드리스 enum) 신뢰할 수 없는 입력이고,
//! 무효한 enum 값을 만드는 것은 UB다. 필요한 정수/문자열 필드만 offset_of 기반으로
//! 안전하게 꺼낸다.

use std::mem::offset_of;

use lunahook_rs::protocol::{HostInfo, RpcHeader, TextOutputHeader, rpc_id};

/// hook → host 방향으로 받은 알림 하나.
#[derive(Debug)]
pub enum Notification {
    Text(TextNotification),
    /// DLL이 탐지한 엔진 이름.
    EngineDetected(String),
    FoundHook(Box<FoundHook>),
    /// 후크 제거 통지 (address).
    Removed(u64),
    /// DLL이 후크 설치를 진행 중 (주소 + hookcode).
    Inserting {
        address: u64,
        hook_code: String,
    },
    /// DLL 측 안내/경고 문자열.
    Info {
        warning: bool,
        message: String,
    },
    /// 형식을 알 수 없거나 무시해도 되는 알림.
    Ignored(u32),
}

#[derive(Debug, Clone)]
pub struct TextNotification {
    pub process_id: u32,
    pub thread_addr: u64,
    pub thread_ctx: u64,
    pub thread_ctx2: u64,
    pub hook_address: u64,
    pub hook_type_flags: u64,
    pub detected_codepage: u32,
    pub hook_name: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct FoundHook {
    pub hook_type_flags: u64,
    pub hook_address: u64,
    /// 신뢰할 수 없는 wire enum은 읽지 않고 `JitType::PC`로 재구성한 설치용 값.
    pub hook_param: lunahook_rs::params::RawHookParam,
    pub text: String,
}

/// 신뢰할 수 없는 wire 바이트 → [`Notification`].
pub(super) fn parse_notification(bytes: &[u8]) -> Option<Notification> {
    let (id, args) = parse_rpc_args(bytes)?;
    match id {
        rpc_id::OUTPUT_TEXT => args
            .first()
            .and_then(|blob| parse_text_output(blob))
            .map(Notification::Text),
        rpc_id::NOTIFY_TEXT => {
            let kind = args
                .first()
                .and_then(|arg| (arg.len() >= 4).then(|| read_u32(arg, 0)))?;
            let message = args
                .get(2)
                .map(|arg| String::from_utf8_lossy(arg).into_owned())?;
            Some(Notification::Info {
                warning: kind != HostInfo::Console as u32,
                message,
            })
        }
        rpc_id::NOTIFY_TEXT_W => {
            let kind = args
                .first()
                .and_then(|arg| (arg.len() >= 4).then(|| read_u32(arg, 0)))?;
            let message = args
                .get(1)
                .map(|arg| read_fixed_utf16(arg, 0, arg.len() / 2))?;
            Some(Notification::Info {
                warning: kind != HostInfo::Console as u32,
                message,
            })
        }
        rpc_id::NOTIFY_HOOK_FOUND => {
            parse_found_hook_args(&args).map(|found| Notification::FoundHook(Box::new(found)))
        }
        rpc_id::NOTIFY_HOOK_REMOVED => args
            .first()
            .and_then(|arg| (arg.len() >= 8).then(|| Notification::Removed(read_u64(arg, 0)))),
        rpc_id::NOTIFY_HOOK_INSERTING => {
            let address = args
                .first()
                .and_then(|arg| (arg.len() >= 8).then(|| read_u64(arg, 0)))?;
            let code_bytes = args.get(1)?;
            Some(Notification::Inserting {
                address,
                hook_code: read_fixed_utf16(code_bytes, 0, code_bytes.len() / 2),
            })
        }
        rpc_id::NOTIFY_ENGINE_DETECTED => args
            .first()
            .map(|name| Notification::EngineDetected(String::from_utf8_lossy(name).into_owned())),
        // 알고도 쓰지 않는 것(`NOTIFY_EMU_GAME_INFO`, `REQUEST_I18N`,
        // `NOTIFY_PREPARED_OK`)과 모르는 것을 같이 흘린다. 나눠 적어도 값이
        // 같아 분기가 하는 일이 없다.
        _ => Some(Notification::Ignored(id)),
    }
}

fn parse_rpc_args(bytes: &[u8]) -> Option<(u32, Vec<&[u8]>)> {
    if bytes.len() < size_of::<RpcHeader>() {
        return None;
    }
    let id = read_u32(bytes, 0);
    let payload_len = read_u32(bytes, 4) as usize;
    let payload_end = size_of::<RpcHeader>().checked_add(payload_len)?;
    if payload_end > bytes.len() {
        return None;
    }
    let payload = &bytes[size_of::<RpcHeader>()..payload_end];
    let mut args = Vec::new();
    let mut cursor = 0;
    while cursor < payload.len() {
        let end_len = cursor.checked_add(4)?;
        if end_len > payload.len() {
            return None;
        }
        let len = read_u32(payload, cursor) as usize;
        cursor = end_len;
        let end = cursor.checked_add(len)?;
        if end > payload.len() {
            return None;
        }
        args.push(&payload[cursor..end]);
        cursor = end;
    }
    Some((id, args))
}

fn parse_text_output(bytes: &[u8]) -> Option<TextNotification> {
    let header_len = size_of::<TextOutputHeader>();
    if bytes.len() <= header_len {
        return None;
    }
    // ThreadParam은 정수 필드만 있어 임의 비트 패턴이 전부 유효하다.
    let tp_base = offset_of!(TextOutputHeader, tp);
    let hp_base = offset_of!(TextOutputHeader, hp);

    Some(TextNotification {
        process_id: read_u32(bytes, tp_base),
        thread_addr: read_u64(
            bytes,
            tp_base + offset_of!(lunahook_rs::protocol::ThreadParam, addr),
        ),
        thread_ctx: read_u64(
            bytes,
            tp_base + offset_of!(lunahook_rs::protocol::ThreadParam, ctx),
        ),
        thread_ctx2: read_u64(
            bytes,
            tp_base + offset_of!(lunahook_rs::protocol::ThreadParam, ctx2),
        ),
        hook_address: read_u64(
            bytes,
            hp_base + offset_of!(lunahook_rs::params::RawHookParam, address),
        ),
        // `hp.hook_type`이 아니라 header의 `kind`를 읽는다. DLL이 이 이벤트에
        // 실제로 적용한 종류를 싣는 자리라, embed 왕복을 포기한 텍스트는
        // `EMBED_ABLE`이 지워진 채로 온다. `hp` 쪽은 후크 등록 당시 값 그대로다.
        hook_type_flags: read_u64(bytes, offset_of!(TextOutputHeader, kind)),
        detected_codepage: read_u32(
            bytes,
            hp_base + offset_of!(lunahook_rs::params::RawHookParam, detected_codepage),
        ),
        hook_name: read_fixed_utf8(
            bytes,
            hp_base + offset_of!(lunahook_rs::params::RawHookParam, name),
            lunahook_rs::params::RawHookParam::default().name.len(),
        ),
        payload: bytes[header_len..].to_vec(),
    })
}

fn parse_found_hook_args(args: &[&[u8]]) -> Option<FoundHook> {
    let hp = args.first()?;
    let text = args.get(1)?;
    if hp.len() < size_of::<lunahook_rs::params::RawHookParam>() {
        return None;
    }
    type Raw = lunahook_rs::params::RawHookParam;
    // 나머지 필드(함수 포인터 등)는 이 프로세스에서 의미가 없으므로 zeroed 기본값을 쓴다.
    let hook_param = Raw {
        address: read_u64(hp, offset_of!(Raw, address)),
        offset: read_i32(hp, offset_of!(Raw, offset)),
        index: read_i32(hp, offset_of!(Raw, index)),
        split: read_i32(hp, offset_of!(Raw, split)),
        split_index: read_i32(hp, offset_of!(Raw, split_index)),
        module: read_u16_array(hp, offset_of!(Raw, module)),
        function: read_u8_array(hp, offset_of!(Raw, function)),
        hook_type: read_u64(hp, offset_of!(Raw, hook_type)),
        codepage: read_u32(hp, offset_of!(Raw, codepage)),
        length_offset: read_i16(hp, offset_of!(Raw, length_offset)),
        padding: read_u64(hp, offset_of!(Raw, padding)),
        user_value: read_u64(hp, offset_of!(Raw, user_value)),
        line_separator: read_u64(hp, offset_of!(Raw, line_separator)),
        name: read_u8_array(hp, offset_of!(Raw, name)),
        hookcode: read_u16_array(hp, offset_of!(Raw, hookcode)),
        detected_codepage: read_u32(hp, offset_of!(Raw, detected_codepage)),
        emu_addr: read_u32(hp, offset_of!(Raw, emu_addr)),
        ..Default::default()
    };
    Some(FoundHook {
        hook_address: hook_param.address,
        hook_type_flags: hook_param.hook_type,
        hook_param,
        text: read_fixed_utf16(text, 0, text.len() / 2),
    })
}

// --- 안전한 little-endian 필드 추출 helpers ------------------------------
//
// 슬라이스 범위가 정확히 N바이트임을 `get(offset..offset+N)`으로 미리
// 확인하므로, 이어지는 배열 리터럴 조립은 항상 성공한다(패닉 경로 없음).

pub(super) fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let Some(chunk) = bytes.get(offset..offset + 4) else {
        return 0;
    };
    u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
}

fn read_i16(bytes: &[u8], offset: usize) -> i16 {
    let Some(chunk) = bytes.get(offset..offset + 2) else {
        return 0;
    };
    i16::from_le_bytes([chunk[0], chunk[1]])
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    let Some(chunk) = bytes.get(offset..offset + 4) else {
        return 0;
    };
    i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
}

fn read_u8_array<const N: usize>(bytes: &[u8], offset: usize) -> [u8; N] {
    let mut out = [0; N];
    if let Some(slice) = bytes.get(offset..offset + N) {
        out.copy_from_slice(slice);
    }
    out
}

fn read_u16_array<const N: usize>(bytes: &[u8], offset: usize) -> [u16; N] {
    let mut out = [0; N];
    let Some(slice) = bytes.get(offset..offset + N * 2) else {
        return out;
    };
    for (unit, chunk) in out.iter_mut().zip(slice.as_chunks::<2>().0) {
        *unit = u16::from_le_bytes(*chunk);
    }
    out
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    let Some(chunk) = bytes.get(offset..offset + 8) else {
        return 0;
    };
    u64::from_le_bytes([
        chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
    ])
}

/// 고정 길이 NUL-종결 UTF-16 배열 필드를 문자열로.
fn read_fixed_utf16(bytes: &[u8], offset: usize, units: usize) -> String {
    let end = (offset + units * 2).min(bytes.len());
    let slice = &bytes[offset.min(end)..end];
    let collected: Vec<u16> = slice
        .as_chunks::<2>()
        .0
        .iter()
        .copied()
        .map(u16::from_le_bytes)
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&collected)
}

/// 고정 길이 NUL-종결 UTF-8(원본은 로케일 바이트일 수도 있지만 이름 필드는
/// ASCII 범위가 일반적) 배열 필드를 손실 허용 문자열로.
fn read_fixed_utf8(bytes: &[u8], offset: usize, len: usize) -> String {
    let end = (offset + len).min(bytes.len());
    let slice = &bytes[offset.min(end)..end];
    let stop = slice
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..stop]).into_owned()
}

#[cfg(test)]
#[path = "../../../tests/unit/hook/pipe_client/notification.rs"]
mod tests;
