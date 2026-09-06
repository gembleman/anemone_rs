use super::*;
use lunahook_rs::params::HookType;
use lunahook_rs::protocol::rpc_frame;

#[test]
fn rpc_info_notification_is_decoded() {
    let kind = (HostInfo::Warning as u32).to_le_bytes();
    let codepage = 932u32.to_le_bytes();
    let buffer =
        rpc_frame(rpc_id::NOTIFY_TEXT, &[&kind, &codepage, b"cannot install"]).expect("info frame");

    match parse_notification(&buffer).expect("parses") {
        Notification::Info { warning, message } => {
            assert!(warning);
            assert_eq!(message, "cannot install");
        }
        other => panic!("expected Info, got {other:?}"),
    }
}

#[test]
fn rpc_engine_notification_is_decoded() {
    let buffer =
        lunahook_rs::protocol::notify_engine_detected("KiriKiri").expect("engine notification");

    match parse_notification(&buffer).expect("parses") {
        Notification::EngineDetected(name) => assert_eq!(name, "KiriKiri"),
        other => panic!("expected EngineDetected, got {other:?}"),
    }
}

#[test]
fn inserting_notification_preserves_hook_code() {
    let buffer = lunahook_rs::protocol::notify_hook_inserting(
        0x1234,
        &"HQ0@1234".encode_utf16().collect::<Vec<_>>(),
    )
    .expect("inserting notification");

    match parse_notification(&buffer).expect("parses") {
        Notification::Inserting { address, hook_code } => {
            assert_eq!(address, 0x1234);
            assert_eq!(hook_code, "HQ0@1234");
        }
        other => panic!("expected Inserting, got {other:?}"),
    }
}

#[test]
fn rpc_text_notification_preserves_detected_codepage() {
    let hp = lunahook_rs::params::RawHookParam {
        address: 0x1234,
        hook_type: (HookType::USING_STRING | HookType::CODEC_UTF8).bits(),
        ..Default::default()
    };
    let mut frame = Vec::new();
    lunahook_rs::protocol::serialize_text_output(
        &mut frame,
        &lunahook_rs::protocol::ThreadParam {
            process_id: 7,
            addr: 8,
            ctx: 9,
            ctx2: 10,
        },
        &hp,
        hp.hook_type,
        b"hello\0",
        1252,
    )
    .expect("text frame");

    let Notification::Text(text) = parse_notification(&frame).expect("text parses") else {
        panic!("expected text notification");
    };
    assert_eq!(text.detected_codepage, 1252);
    assert_eq!(text.process_id, 7);
    assert_eq!(text.payload, b"hello\0");
}

#[test]
fn found_hook_preserves_install_parameters_without_foreign_callbacks() {
    let mut hp = lunahook_rs::params::RawHookParam {
        address: 0x1234_5678,
        offset: -24,
        index: 8,
        split: 16,
        split_index: -4,
        hook_type: (HookType::USING_STRING | HookType::CODEC_UTF16).bits(),
        codepage: 932,
        length_offset: 2,
        padding: 3,
        user_value: 4,
        filter_fun: 0xDEAD,
        ..Default::default()
    };
    hp.module[..4].copy_from_slice(&[b'g' as u16, b'a' as u16, b'm' as u16, b'e' as u16]);
    hp.function[..4].copy_from_slice(b"Text");
    let text_blob: Vec<u8> = "후보"
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    let frame = lunahook_rs::protocol::notify_hook_found(&hp, &text_blob).expect("frame");

    let Notification::FoundHook(found) = parse_notification(&frame).expect("found hook") else {
        panic!("expected found hook notification");
    };
    assert_eq!(found.text, "후보");
    assert_eq!(found.hook_param.address, hp.address);
    assert_eq!(found.hook_param.offset, -24);
    assert_eq!(found.hook_param.index, 8);
    assert_eq!(found.hook_param.module[..4], hp.module[..4]);
    assert_eq!(found.hook_param.function[..4], hp.function[..4]);
    assert_eq!(found.hook_param.filter_fun, 0);
}
