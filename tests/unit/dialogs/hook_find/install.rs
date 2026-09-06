use super::*;

#[test]
fn replacement_queues_new_hook_before_removing_previous() {
    let mut requests = Vec::new();
    let hook = lunahook_rs::params::RawHookParam {
        address: 0x2222,
        ..Default::default()
    };
    let result = queue_hook_replacement(Box::new(hook), Some(0x1111), |request| {
        match request {
            HookRequest::NewHook(param) => requests.push(("new", param.address)),
            HookRequest::RemoveHook(address) => requests.push(("remove", address)),
            _ => unreachable!("replacement helper only emits install/remove"),
        }
        Ok(())
    });

    assert_eq!(result, Ok(0x2222));
    assert_eq!(requests, [("new", 0x2222), ("remove", 0x1111)]);
}

#[test]
fn failed_new_hook_request_keeps_previous_hook_untouched() {
    let mut requests = Vec::new();
    let hook = lunahook_rs::params::RawHookParam {
        address: 0x2222,
        ..Default::default()
    };
    let result = queue_hook_replacement(Box::new(hook), Some(0x1111), |request| match request {
        HookRequest::NewHook(param) => {
            requests.push(("new", param.address));
            Err(())
        }
        HookRequest::RemoveHook(address) => {
            requests.push(("remove", address));
            Ok(())
        }
        _ => unreachable!("replacement helper only emits install/remove"),
    });

    assert_eq!(result, Err(HookInstallError::InstallRequest));
    assert_eq!(requests, [("new", 0x2222)]);
}
