//! 새 후크 설치와 기존 후크 제거의 wire 순서를 보장하는 helper.

use crate::hook::HookRequest;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum HookInstallError {
    InstallRequest,
    RemoveRequest,
}

/// 설치 요청을 먼저 큐에 넣고, 큐 삽입이 확인된 뒤 기존 주소를 제거한다.
///
/// 현재 LunaHook 프로토콜은 NewHook 성공 ACK를 제공하지 않으므로 게임 측
/// 설치 성공까지를 보장한다고 표시하지 않는다. 이 순서는 적어도 워커가
/// 종료된 경우 기존 후크를 먼저 제거하는 손실을 막고, 두 요청의 wire 순서를
/// 보장한다.
pub(super) fn queue_hook_replacement<F>(
    hook_param: Box<lunahook_rs::params::RawHookParam>,
    previous: Option<u64>,
    mut request: F,
) -> std::result::Result<u64, HookInstallError>
where
    F: FnMut(HookRequest) -> std::result::Result<(), ()>,
{
    let address = hook_param.address;
    request(HookRequest::NewHook(hook_param)).map_err(|_| HookInstallError::InstallRequest)?;
    if let Some(previous) = previous.filter(|previous| *previous != address)
        && request(HookRequest::RemoveHook(previous)).is_err()
    {
        return Err(HookInstallError::RemoveRequest);
    }
    Ok(address)
}

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/hook_find/install.rs"]
mod tests;
