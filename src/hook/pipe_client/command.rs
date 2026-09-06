//! host → hook 명령 직렬화.

use std::mem::offset_of;

use lunahook_rs::protocol::{rpc_frame, rpc_id};

/// `RawHookParam`의 C wire ABI 이미지를 만든다.
///
/// Rust 구조체의 값 필드를 모두 채워도 `repr(C)` 패딩 바이트는 초기화되지
/// 않을 수 있다. 예전의 generic `from_raw_parts` 구현은 그 패딩을 읽었고,
/// Miri에서 NEW_HOOK/FIND_HOOK 모두 UB로 잡혔다. 따라서 wire 크기의 zero-filled
/// 버퍼에 각 필드를 명시적으로 기록한다. 이 방식은 C ABI의 offset/크기를
/// 유지하면서 패딩과 reserved 영역을 항상 0으로 보낸다.
fn serialize_raw_hook_param(value: &lunahook_rs::params::RawHookParam) -> Vec<u8> {
    use lunahook_rs::params::RawHookParam;

    let mut bytes = vec![0u8; size_of::<RawHookParam>()];
    write_u64(&mut bytes, offset_of!(RawHookParam, address), value.address);
    write_i32(&mut bytes, offset_of!(RawHookParam, offset), value.offset);
    write_i32(&mut bytes, offset_of!(RawHookParam, index), value.index);
    write_i32(&mut bytes, offset_of!(RawHookParam, split), value.split);
    write_i32(
        &mut bytes,
        offset_of!(RawHookParam, split_index),
        value.split_index,
    );
    write_u16_array(&mut bytes, offset_of!(RawHookParam, module), &value.module);
    write_bytes(
        &mut bytes,
        offset_of!(RawHookParam, function),
        &value.function,
    );
    write_u64(
        &mut bytes,
        offset_of!(RawHookParam, hook_type),
        value.hook_type,
    );
    write_u32(
        &mut bytes,
        offset_of!(RawHookParam, codepage),
        value.codepage,
    );
    write_i16(
        &mut bytes,
        offset_of!(RawHookParam, length_offset),
        value.length_offset,
    );
    write_u64(&mut bytes, offset_of!(RawHookParam, padding), value.padding);
    write_u64(
        &mut bytes,
        offset_of!(RawHookParam, user_value),
        value.user_value,
    );
    write_u64(
        &mut bytes,
        offset_of!(RawHookParam, text_fun),
        value.text_fun,
    );
    write_u64(
        &mut bytes,
        offset_of!(RawHookParam, filter_fun),
        value.filter_fun,
    );
    write_u64(
        &mut bytes,
        offset_of!(RawHookParam, embed_fun),
        value.embed_fun,
    );
    write_u64(
        &mut bytes,
        offset_of!(RawHookParam, embed_hook_font),
        value.embed_hook_font,
    );
    write_u64(
        &mut bytes,
        offset_of!(RawHookParam, line_separator),
        value.line_separator,
    );
    write_bytes(&mut bytes, offset_of!(RawHookParam, name), &value.name);
    write_u16_array(
        &mut bytes,
        offset_of!(RawHookParam, hookcode),
        &value.hookcode,
    );
    write_u32(
        &mut bytes,
        offset_of!(RawHookParam, detected_codepage),
        value.detected_codepage,
    );
    write_u32(
        &mut bytes,
        offset_of!(RawHookParam, emu_addr),
        value.emu_addr,
    );
    write_u32(
        &mut bytes,
        offset_of!(RawHookParam, jittype),
        value.jittype as u32,
    );
    bytes
}

/// `SearchParam`의 C wire ABI 이미지를 만든다. 패딩은 위와 같은 이유로
/// zero-filled 버퍼에 남겨 둔다.
fn serialize_search_param(value: &lunahook_rs::protocol::SearchParam) -> Vec<u8> {
    use lunahook_rs::protocol::SearchParam;

    let mut bytes = vec![0u8; size_of::<SearchParam>()];
    write_bytes(&mut bytes, offset_of!(SearchParam, pattern), &value.pattern);
    write_i32(
        &mut bytes,
        offset_of!(SearchParam, address_method),
        value.address_method,
    );
    write_i32(
        &mut bytes,
        offset_of!(SearchParam, search_method),
        value.search_method,
    );
    write_i32(&mut bytes, offset_of!(SearchParam, length), value.length);
    write_i32(&mut bytes, offset_of!(SearchParam, offset), value.offset);
    write_i32(
        &mut bytes,
        offset_of!(SearchParam, search_time_ms),
        value.search_time_ms,
    );
    write_i32(
        &mut bytes,
        offset_of!(SearchParam, max_records),
        value.max_records,
    );
    write_i32(
        &mut bytes,
        offset_of!(SearchParam, codepage),
        value.codepage,
    );
    write_u64(&mut bytes, offset_of!(SearchParam, padding), value.padding);
    write_u64(
        &mut bytes,
        offset_of!(SearchParam, min_address),
        value.min_address,
    );
    write_u64(
        &mut bytes,
        offset_of!(SearchParam, max_address),
        value.max_address,
    );
    write_u16_array(
        &mut bytes,
        offset_of!(SearchParam, boundary_module),
        &value.boundary_module,
    );
    write_u16_array(
        &mut bytes,
        offset_of!(SearchParam, export_module),
        &value.export_module,
    );
    write_u16_array(&mut bytes, offset_of!(SearchParam, text), &value.text);
    write_bytes(
        &mut bytes,
        offset_of!(SearchParam, is_jit_hook),
        std::slice::from_ref(&value.is_jit_hook),
    );
    write_u16_array(
        &mut bytes,
        offset_of!(SearchParam, share_mem_name),
        &value.share_mem_name,
    );
    write_u64(
        &mut bytes,
        offset_of!(SearchParam, share_mem_size),
        value.share_mem_size,
    );
    bytes
}

fn write_bytes(out: &mut [u8], offset: usize, value: &[u8]) {
    out[offset..offset + value.len()].copy_from_slice(value);
}

fn write_u16_array(out: &mut [u8], offset: usize, value: &[u16]) {
    for (index, unit) in value.iter().copied().enumerate() {
        let start = offset + index * 2;
        out[start..start + 2].copy_from_slice(&unit.to_le_bytes());
    }
}

fn write_i16(out: &mut [u8], offset: usize, value: i16) {
    out[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_i32(out: &mut [u8], offset: usize, value: i32) {
    out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(out: &mut [u8], offset: usize, value: u32) {
    out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(out: &mut [u8], offset: usize, value: u64) {
    out[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

/// `rpc_frame`은 payload가 파이프 버퍼 크기를 넘을 때만 실패한다. 이 모듈이
/// 만드는 모든 payload는 고정 크기 wire 구조체라 실제로는 결코 초과하지
/// 않지만, 프로덕션 경로에서 panic 대신 로그를 남기고 빈 프레임으로
/// 안전하게 폴백한다.
fn frame_or_log(id: u32, args: &[&[u8]], what: &str) -> Vec<u8> {
    rpc_frame(id, args).unwrap_or_else(|| {
        tracing::error!("{what} 프레임이 파이프 버퍼 크기를 초과했습니다 (있을 수 없는 상황)");
        Vec::new()
    })
}

/// NEW_HOOK 명령. `hp`는 lunahook_rs가 정의한 유효한 값이어야 한다.
pub fn build_new_hook(hp: &lunahook_rs::params::RawHookParam) -> Vec<u8> {
    let bytes = serialize_raw_hook_param(hp);
    frame_or_log(rpc_id::NEW_HOOK, &[&bytes], "NEW_HOOK")
}

/// REMOVE_HOOK 명령.
pub fn build_remove_hook(address: u64) -> Vec<u8> {
    frame_or_log(
        rpc_id::REMOVE_HOOK,
        &[&address.to_le_bytes()],
        "REMOVE_HOOK",
    )
}

/// FIND_HOOK 명령. `sp.text`가 비었으면 범용 탐색, 있으면 텍스트 검색이다.
pub fn build_find_hook(sp: &lunahook_rs::protocol::SearchParam) -> Vec<u8> {
    let bytes = serialize_search_param(sp);
    frame_or_log(rpc_id::FIND_HOOK, &[&bytes], "FIND_HOOK")
}

/// 텍스트 검색용 FIND_HOOK payload. `text`가 화면에 보이는 문장과 일치해야 한다.
pub fn build_text_search_param(text: &str) -> lunahook_rs::protocol::SearchParam {
    use lunahook_rs::protocol::{PATTERN_SIZE, SearchParam};
    // SAFETY: SearchParam은 정수/고정 배열 필드만 있어 zeroed가 유효하다.
    unsafe {
        let mut sp: SearchParam = std::mem::zeroed();
        sp.search_time_ms = 30_000;
        sp.max_records = 20;
        sp.codepage = 932; // SHIFT_JIS
        let units: Vec<u16> = text.encode_utf16().take(PATTERN_SIZE - 1).collect();
        sp.text[..units.len()].copy_from_slice(&units);
        sp
    }
}

/// 범용 탐색용 FIND_HOOK payload (`sp.text` 비움 → DLL의 후보 수집 런타임).
pub fn build_general_search_param() -> lunahook_rs::protocol::SearchParam {
    // SAFETY: 동일하다.
    unsafe {
        let mut sp: lunahook_rs::protocol::SearchParam = std::mem::zeroed();
        sp.search_time_ms = 30_000;
        sp.max_records = 50;
        sp.codepage = 932;
        sp
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/hook/pipe_client/command.rs"]
mod tests;
