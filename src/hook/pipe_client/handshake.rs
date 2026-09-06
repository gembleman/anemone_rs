//! LUNA_HOST 파이프 핸드셰이크 — `communication_initialize`의 호스트 측 절차.

use std::ffi::OsString;
use std::fs;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use lunahook_rs::protocol::{
    COMPATIBLE_SIG_BYTES, PIPE_BUFFER_SIZE, RpcHeader, VERSION_WIRE_SIZE, rpc_frame, rpc_id,
};

use super::PipeError;
use super::notification::read_u32;
use super::server::PipeServer;

/// cwd 하나의 파일 목록을 지나치게 크게 만들지 않도록 제한한다. 이 값은
/// `lunahook_rs::host::dispatch`의 수신 제한(1 Mi UTF-16 code unit)보다 작고,
/// 파일명 하나를 한 번의 message-mode WriteFile로 보낼 수 있는 파이프 버퍼에도
/// 맞는다.
const MAX_CHECK_FILE_UNITS: usize = (PIPE_BUFFER_SIZE - size_of::<i32>()) / size_of::<u16>();

/// 게임 폴더가 게임 데이터가 아닌 수십만 개의 파일을 담고 있을 때 핸드셰이크가
/// 끝없이 오래 걸리지 않도록 전체 목록에도 제한을 둔다. 제한에 도달하면 지금까지
/// 수집한 항목만 보내고 -1 센티널로 정상 종료한다.
const MAX_CHECK_FILES: usize = 16_384;
const MAX_CHECK_FILE_TOTAL_UNITS: usize = 1 << 20;
const MAX_CHECK_DIRECTORY_ENTRIES: usize = 65_536;

/// version+서명 확인 → 게임 cwd 수신 → cwd 기준 상대 파일 목록 전송 →
/// i18n 요청 응답 및 PreparedOk 확인. 성공하면 게임 작업 폴더를 돌려준다(로깅용).
pub(super) fn perform_handshake(server: &PipeServer) -> Result<String, PipeError> {
    let mut buffer = vec![0u8; 2048];

    // 1. LUNA_VERSION 4×u16 — DLL은 전부 0으로 보낸다(값 미사용).
    server.read_exact(&mut buffer[..VERSION_WIRE_SIZE])?;
    // 2. COMPATIBLE_SIG.
    server.read_exact(&mut buffer[..COMPATIBLE_SIG_BYTES.len()])?;
    if buffer[..COMPATIBLE_SIG_BYTES.len()] != COMPATIBLE_SIG_BYTES {
        return Err(PipeError::Handshake(
            "lunahook 버전 시그니처가 일치하지 않습니다".to_string(),
        ));
    }
    // 3. cwd: 길이(u32) → UTF-16 본문.
    server.read_exact(&mut buffer[..4])?;
    let cwd_units = read_u32(&buffer, 0);
    if cwd_units > 1024 {
        return Err(PipeError::Handshake(format!(
            "비정상적인 작업 폴더 길이: {cwd_units}"
        )));
    }
    let mut cwd_bytes = vec![0u8; cwd_units as usize * 2];
    server.read_exact(&mut cwd_bytes)?;
    let cwd_units: Vec<u16> = cwd_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .copied()
        .map(u16::from_le_bytes)
        .collect();
    let cwd = String::from_utf16_lossy(&cwd_units);

    // DLL이 보낸 cwd는 호스트 프로세스의 cwd와 다를 수 있다. 특히 게임 런처가
    // 다른 폴더에서 실행된 경우 호스트의 current_dir()를 사용하면 RAILLORE의
    // RIO.INI와 *.WAR를 찾지 못하므로, 수신한 wide path를 그대로 사용한다.
    let cwd_path = PathBuf::from(OsString::from_wide(&cwd_units));
    send_check_files(server, &cwd_path)?;

    // 5. DLL은 등록된 각 i18n 키에 대해 REQUEST_I18N을 먼저 보내고,
    // RESPOND_I18N을 받은 뒤 마지막에 PreparedOk를 보낸다.
    finish_handshake(server)?;

    Ok(cwd)
}

/// 파일 목록 뒤의 RPC 교환을 완료한다. RPC 하나가 message-mode 파이프의 한
/// 메시지이므로 헤더만 읽고 payload를 다음 메시지로 간주해서는 안 된다.
fn finish_handshake(server: &PipeServer) -> Result<(), PipeError> {
    let mut frame = vec![0u8; PIPE_BUFFER_SIZE];
    loop {
        let frame_len = server.read_message(&mut frame)?;
        let (id, payload) = parse_rpc_frame(&frame[..frame_len])?;
        match id {
            rpc_id::REQUEST_I18N => {
                let (key, default_raw) = parse_i18n_request(payload)?;
                // anemone에는 DLL의 hook 안내 문자열을 별도로 번역하는
                // production callback/table이 없으므로, DLL이 보낸 default_raw를
                // 그대로 돌려보내 lunahook_rs::host::tr의 기본값 의미를 보존한다.
                let key_bytes = key.to_le_bytes();
                let response = rpc_frame(rpc_id::RESPOND_I18N, &[&key_bytes, default_raw])
                    .ok_or_else(|| {
                        PipeError::Handshake(
                            "i18n 응답이 파이프 버퍼 크기를 초과했습니다".to_string(),
                        )
                    })?;
                server.write_command(&response).map_err(|error| {
                    PipeError::Handshake(format!("i18n 응답 전송 실패: {error}"))
                })?;
            }
            rpc_id::NOTIFY_PREPARED_OK if payload.is_empty() => return Ok(()),
            rpc_id::NOTIFY_PREPARED_OK => {
                return Err(PipeError::Handshake(format!(
                    "PreparedOk payload가 비어 있지 않습니다: {}바이트",
                    payload.len()
                )));
            }
            other => {
                return Err(PipeError::Handshake(format!(
                    "PreparedOk 또는 REQUEST_I18N 대신 RPC id={other}를 받았습니다"
                )));
            }
        }
    }
}

/// message-mode로 받은 RPC 한 프레임을 엄격하게 분해한다. 파이프 버퍼보다
/// 큰 payload는 `read_message` 단계에서 수용되지 않으며, 길이가 실제 메시지와
/// 다른 프레임도 핸드셰이크를 중단시킨다.
fn parse_rpc_frame(frame: &[u8]) -> Result<(u32, &[u8]), PipeError> {
    let header_size = size_of::<RpcHeader>();
    if frame.len() < header_size {
        return Err(PipeError::Handshake(format!(
            "RPC 헤더가 잘렸습니다: {}바이트",
            frame.len()
        )));
    }

    let id = read_u32(frame, 0);
    let payload_size = read_u32(frame, 4) as usize;
    let payload_end = header_size.checked_add(payload_size).ok_or_else(|| {
        PipeError::Handshake("RPC payload 길이 계산이 오버플로되었습니다".to_string())
    })?;
    if payload_end != frame.len() {
        return Err(PipeError::Handshake(format!(
            "RPC payload 길이 불일치: 헤더 {}, 실제 {}",
            payload_size,
            frame.len().saturating_sub(header_size)
        )));
    }
    Ok((id, &frame[header_size..payload_end]))
}

/// RPC payload의 길이-prefixed 인자를 하나 읽는다.
fn next_rpc_arg<'a>(payload: &'a [u8], cursor: &mut usize) -> Option<&'a [u8]> {
    let length_end = cursor.checked_add(4)?;
    if length_end > payload.len() {
        return None;
    }
    let length = read_u32(payload, *cursor) as usize;
    let value_end = length_end.checked_add(length)?;
    if value_end > payload.len() {
        return None;
    }
    *cursor = value_end;
    Some(&payload[length_end..value_end])
}

/// REQUEST_I18N은 `i32 key`와 원본 default bytes 두 인자를 가진다. 추가/누락
/// 인자는 거부해 응답의 key와 기본 문자열이 뒤섞이지 않도록 한다.
fn parse_i18n_request(payload: &[u8]) -> Result<(i32, &[u8]), PipeError> {
    let mut cursor = 0;
    let key = next_rpc_arg(payload, &mut cursor)
        .ok_or_else(|| PipeError::Handshake("REQUEST_I18N key 인자가 잘렸습니다".to_string()))?;
    if key.len() != size_of::<i32>() {
        return Err(PipeError::Handshake(format!(
            "REQUEST_I18N key 길이가 잘못되었습니다: {}바이트",
            key.len()
        )));
    }
    let default_raw = next_rpc_arg(payload, &mut cursor).ok_or_else(|| {
        PipeError::Handshake("REQUEST_I18N 기본 문자열 인자가 잘렸습니다".to_string())
    })?;
    if cursor != payload.len() {
        return Err(PipeError::Handshake(
            "REQUEST_I18N payload에 예상하지 못한 인자가 있습니다".to_string(),
        ));
    }
    let key =
        i32::from_le_bytes(key.try_into().map_err(|_| {
            PipeError::Handshake("REQUEST_I18N key를 읽을 수 없습니다".to_string())
        })?);
    Ok((key, default_raw))
}

/// 원본 LunaHost의 파일 목록 교환은 항목마다 `i32 길이`와 UTF-16 본문을
/// 별도 message로 쓰고, 마지막에 `-1`을 쓴다. `FileEvidence`는 경로 구분자를
/// 양쪽 모두 허용하고 대소문자를 무시하므로, 여기서는 Windows의 원래 wide
/// 이름과 디렉터리 구조를 보존한다.
fn send_check_files(server: &PipeServer, cwd: &Path) -> Result<(), PipeError> {
    let entries = collect_check_files(cwd);
    for units in entries {
        let Ok(size) = i32::try_from(units.len()) else {
            continue;
        };
        server
            .write_command(&size.to_le_bytes())
            .map_err(|error| PipeError::Handshake(format!("파일 목록 길이 전송 실패: {error}")))?;

        let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
        server
            .write_command(&bytes)
            .map_err(|error| PipeError::Handshake(format!("파일 목록 본문 전송 실패: {error}")))?;
    }

    server
        .write_command(&(-1i32).to_le_bytes())
        .map_err(|error| PipeError::Handshake(format!("파일 목록 종료 전송 실패: {error}")))
}

/// 게임 cwd 아래의 파일/디렉터리 항목을 cwd 기준 상대경로의 UTF-16 단위로
/// 모은다. 원본 LunaHost의 `CheckFileHelper`는 루트 항목 자체와 각 루트
/// 디렉터리의 직속 child만 보냈으므로, 그 범위를 보존해 임의의 하위 트리를
/// 재귀 탐색하지 않는다. 디렉터리 항목 자체도 `PathFileExists` 증거로 쓰일 수
/// 있어 목록에 포함하되, junction/symlink는 디렉터리로 따라가지 않는다.
///
/// 접근할 수 없는 디렉터리나 항목은 해당 부분만 건너뛴다. 루트 자체가 없거나
/// 읽을 수 없는 경우 빈 목록을 반환하며, 호출자는 그래도 -1 센티널을 보낸다.
fn collect_check_files(cwd: &Path) -> Vec<Vec<u16>> {
    let mut files = Vec::new();
    let mut total_units = 0usize;
    let mut visited_entries = 0usize;

    let Ok(root_entries) = fs::read_dir(cwd) else {
        return files;
    };
    for root_entry in root_entries {
        if visited_entries == MAX_CHECK_DIRECTORY_ENTRIES {
            return files;
        }
        visited_entries += 1;

        let Ok(root_entry) = root_entry else {
            continue;
        };
        let Ok(file_type) = root_entry.file_type() else {
            continue;
        };
        let relative = PathBuf::from(root_entry.file_name());
        if !append_check_file(&mut files, &mut total_units, &relative) {
            return files;
        }

        // `file_type` does not follow symlinks, so only a real root directory is
        // opened. Its children are intentionally not queued for further walking.
        if !file_type.is_dir() {
            continue;
        }
        let Ok(children) = fs::read_dir(root_entry.path()) else {
            continue;
        };
        for child in children {
            if visited_entries == MAX_CHECK_DIRECTORY_ENTRIES {
                return files;
            }
            visited_entries += 1;

            let Ok(child) = child else {
                continue;
            };
            let relative_child = relative.join(child.file_name());
            if !append_check_file(&mut files, &mut total_units, &relative_child) {
                return files;
            }
        }
    }

    files
}

/// 경로 하나를 추가한다. `false`는 전체 목록 제한에 도달해 열거를 중단해야
/// 한다는 뜻이고, 이름이 너무 길거나 비어 있는 경우에는 단순히 건너뛰고
/// `true`를 돌려줘 디렉터리의 다음 child를 계속 살핀다.
fn append_check_file(files: &mut Vec<Vec<u16>>, total_units: &mut usize, path: &Path) -> bool {
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    if units.is_empty() || units.len() > MAX_CHECK_FILE_UNITS {
        return true;
    }
    let Some(next_total) = total_units.checked_add(units.len()) else {
        return false;
    };
    if files.len() == MAX_CHECK_FILES || next_total > MAX_CHECK_FILE_TOTAL_UNITS {
        return false;
    }
    *total_units = next_total;
    files.push(units);
    true
}

#[cfg(test)]
#[path = "../../../tests/unit/hook/pipe_client/handshake.rs"]
mod tests;
