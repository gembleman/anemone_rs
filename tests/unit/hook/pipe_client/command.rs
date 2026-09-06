use super::*;
use lunahook_rs::params::HookType;
use lunahook_rs::protocol::HostCommand;

/// anemone이 만든 NEW_HOOK 바이트를 lunahook_rs의 파서로 되읽는 왕복 검사.
/// 주입 DLL과 같은 파서를 쓰므로, 이 테스트가 통과하면 wire 계약도 유지된다.
#[test]
fn new_hook_round_trips_through_lunahook_parser() {
    let hp = lunahook_rs::params::RawHookParam {
        address: 0x0000_7FF6_1234_5678,
        hook_type: (HookType::USING_STRING | HookType::CODEC_UTF16).bits(),
        offset: -80,
        codepage: 932,
        ..lunahook_rs::params::RawHookParam::default()
    };
    let mut hp = hp;
    let name = b"UserH1";
    hp.name[..name.len()].copy_from_slice(name);

    let bytes = build_new_hook(&hp);
    let (_, parsed) = lunahook_rs::protocol::dispatch_host_rpc(&bytes)
        .expect("lunahook parser accepts our NEW_HOOK");
    let Some(HostCommand::NewHook(cmd)) = parsed else {
        panic!("expected NewHook command");
    };
    assert_eq!(cmd.address, hp.address);
    assert_eq!(cmd.hook_type, hp.hook_type);
    assert_eq!(cmd.offset, hp.offset);
    assert_eq!(cmd.codepage, hp.codepage);
    assert_eq!(&cmd.name[..6], b"UserH1");
}

#[test]
fn raw_hook_wire_image_zeroes_repr_c_padding() {
    let hp = lunahook_rs::params::RawHookParam {
        address: 1,
        offset: -2,
        length_offset: -3,
        padding: u64::MAX,
        jittype: lunahook_rs::params::JitType::PC,
        ..Default::default()
    };
    let bytes = serialize_raw_hook_param(&hp);
    assert_eq!(bytes.len(), size_of::<lunahook_rs::params::RawHookParam>());
    // length_offset occupies 396..398 and the C ABI aligns the following
    // u64 at 400, leaving two bytes that must not leak uninitialized data.
    assert_eq!(&bytes[398..400], &[0, 0]);
    assert_eq!(
        u64::from_le_bytes(bytes[400..408].try_into().unwrap()),
        u64::MAX
    );
}

#[test]
fn remove_hook_round_trips() {
    let bytes = build_remove_hook(0x1234);
    match lunahook_rs::protocol::dispatch_host_rpc(&bytes)
        .expect("remove parses")
        .1
    {
        Some(HostCommand::RemoveHook(address)) => {
            assert_eq!(address, 0x1234);
        }
        _ => panic!("expected RemoveHook"),
    }
}

#[test]
fn find_hook_text_search_carries_user_text() {
    let sp = build_text_search_param("テスト");
    let bytes = build_find_hook(&sp);
    match lunahook_rs::protocol::dispatch_host_rpc(&bytes)
        .expect("find parses")
        .1
    {
        Some(HostCommand::FindHook(sp)) => {
            let units: Vec<u16> = sp
                .text
                .iter()
                .take_while(|unit| **unit != 0)
                .copied()
                .collect();
            assert_eq!(String::from_utf16_lossy(&units), "テスト");
            assert_eq!(sp.search_time_ms, 30_000);
        }
        _ => panic!("expected FindHook"),
    }
}

#[test]
fn find_hook_wire_image_zeroes_all_repr_c_padding() {
    let sp = lunahook_rs::protocol::SearchParam {
        codepage: 932,
        padding: u64::MAX,
        is_jit_hook: 1,
        share_mem_size: u64::MAX,
        ..Default::default()
    };
    let bytes = serialize_search_param(&sp);
    assert_eq!(bytes.len(), size_of::<lunahook_rs::protocol::SearchParam>());
    assert_eq!(&bytes[60..64], &[0, 0, 0, 0]);
    assert_eq!(&bytes[629..630], &[0]);
    assert_eq!(&bytes[758..760], &[0, 0]);
    assert_eq!(
        u64::from_le_bytes(bytes[64..72].try_into().unwrap()),
        u64::MAX
    );
}
