use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use lunahook_rs::FileEvidence;
use lunahook_rs::engine::{Architecture, DetectionEvidence};
use lunahook_rs::protocol::{HOOK_PIPE, HOST_PIPE, notify_prepared_ok, rpc_frame, rpc_id};

use super::*;

/// DLL 역할을 하는 가짜 클라이언트. HOOK_PIPE는 쓰기 전용, HOST_PIPE는
/// 읽기 전용으로 연다 — `PipeServer`가 반대 방향으로 만들었기 때문이다
/// (`server.rs`의 named pipe 왕복 테스트와 같은 패턴).
fn open_pipe(prefix: &str, pid: u32, write: bool) -> File {
    let name = format!("{prefix}{pid}");
    let mut options = OpenOptions::new();
    loop {
        let opened = if write {
            options.read(false).write(true).open(&name)
        } else {
            options.read(true).write(false).open(&name)
        };
        match opened {
            Ok(file) => return file,
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// 접속 직후 클라이언트가 쓰기/닫기를 하면, 서버의 `wait_connect()`가
/// (hook_pipe/host_pipe를 순서대로 접속시키는 도중) 아직 host_pipe 접속을
/// 확인하기 전에 클라이언트가 먼저 끊어버리는 경합이 생길 수 있다
/// (`GetOverlappedResult`가 접속 완료 대신 끊김을 보게 된다). 채널로
/// "서버가 wait_connect에 성공했다"는 신호를 받은 뒤에만 파이프를 건드리게
/// 해서 이 경합을 없앤다.
fn ready_channel() -> (Sender<()>, Receiver<()>) {
    mpsc::channel()
}

/// 정상적인 DLL의 handshake 순서를 그대로 재현하는 클라이언트.
///
/// 서버가 길이(4바이트)와 본문을 **별도의** `read_exact` 호출로 나눠 읽으므로
/// (`perform_handshake` 3단계), message-mode 파이프에서 한 메시지 = 한
/// `WriteFile`이라는 계약을 지키려면 여기서도 두 번에 나눠 써야 한다. 하나로
/// 합쳐 쓰면 첫 `read_exact(4)`가 더 큰 메시지의 일부만 받게 되어 실제 DLL
/// 동작과 어긋난다.
fn write_valid_handshake_prefix(hook_writer: &mut File, cwd: &str) {
    hook_writer.write_all(&[0u8; VERSION_WIRE_SIZE]).unwrap();
    hook_writer.write_all(&COMPATIBLE_SIG_BYTES).unwrap();
    let units: Vec<u16> = cwd.encode_utf16().collect();
    hook_writer
        .write_all(&(units.len() as u32).to_le_bytes())
        .unwrap();
    let body: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    hook_writer.write_all(&body).unwrap();
}

fn expect_sentinel(host_reader: &mut File) {
    let mut sentinel = [0u8; 4];
    host_reader.read_exact(&mut sentinel).unwrap();
    assert_eq!(
        i32::from_le_bytes(sentinel),
        -1,
        "빈 파일 목록은 -1 센티널이어야 한다"
    );
}

fn read_file_list(host_reader: &mut File) -> Vec<String> {
    let mut files = Vec::new();
    loop {
        let mut size_bytes = [0u8; 4];
        host_reader.read_exact(&mut size_bytes).unwrap();
        let size = i32::from_le_bytes(size_bytes);
        if size == -1 {
            return files;
        }
        assert!((0..=MAX_CHECK_FILE_UNITS as i32).contains(&size));
        let mut body = vec![0u8; size as usize * 2];
        host_reader.read_exact(&mut body).unwrap();
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(u16::from_le_bytes)
            .collect();
        files.push(String::from_utf16_lossy(&units));
    }
}

fn read_rpc_frame(host_reader: &mut File) -> (u32, Vec<Vec<u8>>) {
    let mut frame = vec![0u8; PIPE_BUFFER_SIZE];
    let frame_len = host_reader.read(&mut frame).unwrap();
    assert!(frame_len >= size_of::<RpcHeader>());
    let frame = &frame[..frame_len];
    let id = u32::from_le_bytes(frame[..4].try_into().unwrap());
    let payload_size = u32::from_le_bytes(frame[4..8].try_into().unwrap()) as usize;
    assert_eq!(frame.len(), size_of::<RpcHeader>() + payload_size);
    let payload = &frame[size_of::<RpcHeader>()..];

    let mut args = Vec::new();
    let mut cursor = 0usize;
    while cursor < payload.len() {
        let length_end = cursor + 4;
        assert!(length_end <= payload.len());
        let length = u32::from_le_bytes(payload[cursor..length_end].try_into().unwrap()) as usize;
        let value_end = length_end + length;
        assert!(value_end <= payload.len());
        args.push(payload[length_end..value_end].to_vec());
        cursor = value_end;
    }
    assert_eq!(cursor, payload.len());
    (id, args)
}

fn expect_i18n_response(host_reader: &mut File, key: i32, default_raw: &[u8]) {
    let (id, args) = read_rpc_frame(host_reader);
    assert_eq!(id, rpc_id::RESPOND_I18N);
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], key.to_le_bytes());
    assert_eq!(args[1], default_raw);
}

fn test_directory(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("anemone-handshake-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("Data").join("深い")).unwrap();
    fs::write(root.join("RIO.INI"), b"[RailLore]").unwrap();
    fs::write(root.join("episode.WAR"), b"war").unwrap();
    fs::write(root.join("Data").join("직속.dat"), b"direct child").unwrap();
    fs::write(root.join("Data").join("深い").join("台詞.dat"), b"text").unwrap();
    root
}

#[test]
fn a_well_formed_handshake_returns_the_game_working_directory() {
    let pid = 0x5EED_1000;
    let server = PipeServer::create(pid).expect("파이프 생성");
    let (ready_tx, ready_rx) = ready_channel();
    let root = test_directory("well-formed");
    let cwd = root.to_str().unwrap().to_owned();
    let expected_cwd = cwd.clone();
    let client_cwd = cwd.clone();

    let client = std::thread::spawn(move || {
        let mut hook_writer = open_pipe(HOOK_PIPE, pid, true);
        let mut host_reader = open_pipe(HOST_PIPE, pid, false);
        let _ = ready_rx.recv();

        write_valid_handshake_prefix(&mut hook_writer, &client_cwd);
        let files = read_file_list(&mut host_reader);
        assert!(
            files
                .iter()
                .any(|file| file.eq_ignore_ascii_case("RIO.INI"))
        );
        assert!(
            files
                .iter()
                .any(|file| file.eq_ignore_ascii_case("episode.WAR"))
        );
        assert!(files.iter().any(|file| {
            file.replace('\\', "/")
                .eq_ignore_ascii_case("Data/직속.dat")
        }));
        assert!(!files.iter().any(|file| {
            file.replace('\\', "/")
                .eq_ignore_ascii_case("Data/深い/台詞.dat")
        }));
        assert!(
            files
                .iter()
                .all(|file| !std::path::Path::new(file).is_absolute())
        );
        let evidence = DetectionEvidence::new(Architecture::host(), files.clone());
        assert!(FileEvidence::parse("RIO.INI").matches_any(&evidence));
        assert!(FileEvidence::parse("*.WAR").matches_any(&evidence));

        let requests: [(i32, &[u8]); 2] = [
            (17, b"Installing hook"),
            (-3, &[0x00, 0xE3, 0x81, 0x82, 0xFF]),
        ];
        for (key, default_raw) in requests {
            let key_bytes = key.to_le_bytes();
            let request = rpc_frame(rpc_id::REQUEST_I18N, &[&key_bytes, default_raw])
                .expect("i18n request frame");
            hook_writer.write_all(&request).unwrap();
            expect_i18n_response(&mut host_reader, key, default_raw);
        }
        hook_writer
            .write_all(&notify_prepared_ok().unwrap())
            .unwrap();
    });

    server
        .wait_connect()
        .expect("클라이언트가 양쪽 파이프에 접속해야 한다");
    ready_tx
        .send(())
        .expect("클라이언트에 신호를 보낼 수 있어야 한다");
    let cwd = perform_handshake(&server).expect("정상 핸드셰이크는 성공해야 한다");
    assert_eq!(cwd, expected_cwd);
    client.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_missing_working_directory_still_has_a_bounded_empty_list() {
    let root =
        std::env::temp_dir().join(format!("anemone-handshake-missing-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    assert!(collect_check_files(&root).is_empty());
}

#[test]
fn file_evidence_collection_stays_at_the_original_shallow_depth() {
    let root = test_directory("shallow");
    let files = collect_check_files(&root)
        .into_iter()
        .map(|units| String::from_utf16_lossy(&units).replace('\\', "/"))
        .collect::<Vec<_>>();

    assert!(files.iter().any(|file| file.eq_ignore_ascii_case("Data")));
    assert!(
        files
            .iter()
            .any(|file| file.eq_ignore_ascii_case("Data/직속.dat"))
    );
    assert!(
        !files
            .iter()
            .any(|file| file.eq_ignore_ascii_case("Data/深い/台詞.dat"))
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_mismatched_signature_is_rejected() {
    let pid = 0x5EED_1001;
    let server = PipeServer::create(pid).expect("파이프 생성");
    let (ready_tx, ready_rx) = ready_channel();

    let client = std::thread::spawn(move || {
        let mut hook_writer = open_pipe(HOOK_PIPE, pid, true);
        // wait_connect()는 두 파이프 모두의 접속을 기다린다 — 이 테스트는
        // HOST_PIPE로 아무것도 주고받지 않지만, 접속 자체는 해야 한다.
        let _host_reader = open_pipe(HOST_PIPE, pid, false);
        let _ = ready_rx.recv();

        hook_writer.write_all(&[0u8; VERSION_WIRE_SIZE]).unwrap();
        // 서명이 전부 0이면 실제 COMPATIBLE_SIG_BYTES와 다르다(그 배열은
        // 0을 포함하지 않는 상수다).
        hook_writer
            .write_all(&[0u8; COMPATIBLE_SIG_BYTES.len()])
            .unwrap();
        // 서버가 실패를 감지하고 반환한 뒤 스레드가 끊겨도 조용히 지나가도록
        // 쓰기 실패는 무시한다.
        let _ = hook_writer.flush();
    });

    server.wait_connect().expect("접속은 성공해야 한다");
    ready_tx
        .send(())
        .expect("클라이언트에 신호를 보낼 수 있어야 한다");
    let result = perform_handshake(&server);
    match result {
        Err(PipeError::Handshake(message)) => {
            assert!(message.contains("버전 시그니처"));
        }
        other => panic!("서명 불일치를 기대했는데 {other:?}"),
    }
    client.join().unwrap();
}

#[test]
fn an_absurdly_long_working_directory_length_is_rejected() {
    let pid = 0x5EED_1002;
    let server = PipeServer::create(pid).expect("파이프 생성");
    let (ready_tx, ready_rx) = ready_channel();

    let client = std::thread::spawn(move || {
        let mut hook_writer = open_pipe(HOOK_PIPE, pid, true);
        let _host_reader = open_pipe(HOST_PIPE, pid, false);
        let _ = ready_rx.recv();

        hook_writer.write_all(&[0u8; VERSION_WIRE_SIZE]).unwrap();
        hook_writer.write_all(&COMPATIBLE_SIG_BYTES).unwrap();
        // 1024을 넘는 길이는 즉시 거부되어야 한다 — 실제 본문은 보낼 필요조차 없다.
        hook_writer.write_all(&2000u32.to_le_bytes()).unwrap();
    });

    server.wait_connect().expect("접속은 성공해야 한다");
    ready_tx
        .send(())
        .expect("클라이언트에 신호를 보낼 수 있어야 한다");
    let result = perform_handshake(&server);
    match result {
        Err(PipeError::Handshake(message)) => {
            assert!(message.contains("비정상적인 작업 폴더 길이"));
        }
        other => panic!("길이 초과 거부를 기대했는데 {other:?}"),
    }
    client.join().unwrap();
}

#[test]
fn an_unexpected_rpc_instead_of_prepared_ok_is_rejected() {
    let pid = 0x5EED_1003;
    let server = PipeServer::create(pid).expect("파이프 생성");
    let (ready_tx, ready_rx) = ready_channel();

    let client = std::thread::spawn(move || {
        let mut hook_writer = open_pipe(HOOK_PIPE, pid, true);
        let mut host_reader = open_pipe(HOST_PIPE, pid, false);
        let _ = ready_rx.recv();

        write_valid_handshake_prefix(&mut hook_writer, "");
        expect_sentinel(&mut host_reader);
        // PreparedOk 대신 DETACH 헤더를 보낸다.
        let bogus =
            lunahook_rs::protocol::rpc_frame(lunahook_rs::protocol::rpc_id::DETACH, &[]).unwrap();
        hook_writer.write_all(&bogus).unwrap();
    });

    server.wait_connect().expect("접속은 성공해야 한다");
    ready_tx
        .send(())
        .expect("클라이언트에 신호를 보낼 수 있어야 한다");
    let result = perform_handshake(&server);
    match result {
        Err(PipeError::Handshake(message)) => {
            assert!(message.contains("PreparedOk"));
        }
        other => panic!("PreparedOk 불일치를 기대했는데 {other:?}"),
    }
    client.join().unwrap();
}

/// 빈 문자열(cwd 길이 0)도 정상 경로로 받아들여야 한다 — 게임이 매우 짧은
/// 작업 폴더 표기를 보낼 수도 있는 경계값이다.
#[test]
fn an_empty_working_directory_is_accepted() {
    let pid = 0x5EED_1004;
    let server = PipeServer::create(pid).expect("파이프 생성");
    let (ready_tx, ready_rx) = ready_channel();

    let client = std::thread::spawn(move || {
        let mut hook_writer = open_pipe(HOOK_PIPE, pid, true);
        let mut host_reader = open_pipe(HOST_PIPE, pid, false);
        let _ = ready_rx.recv();

        write_valid_handshake_prefix(&mut hook_writer, "");
        expect_sentinel(&mut host_reader);
        hook_writer
            .write_all(&notify_prepared_ok().unwrap())
            .unwrap();
    });

    server.wait_connect().expect("접속은 성공해야 한다");
    ready_tx
        .send(())
        .expect("클라이언트에 신호를 보낼 수 있어야 한다");
    let cwd = perform_handshake(&server).expect("빈 cwd도 성공해야 한다");
    assert_eq!(cwd, "");
    client.join().unwrap();
}

/// 클라이언트가 핸드셰이크 도중 연결을 끊으면 Disconnected로 보고해야 한다.
///
/// 클라이언트가 접속 직후 바로 닫아버리면 "접속 자체의 실패"(wait_connect의
/// GetOverlappedResult 실패)와 "접속 이후 핸드셰이크 도중 끊김"(이 테스트가
/// 보려는 것)이 경합한다. 채널로 순서를 강제해 wait_connect가 반드시 먼저
/// 성공한 뒤에만 연결을 끊도록 만든다.
#[test]
fn a_client_disconnecting_mid_handshake_is_reported() {
    let pid = 0x5EED_1005;
    let server = PipeServer::create(pid).expect("파이프 생성");
    let (ready_tx, ready_rx) = ready_channel();

    let client = std::thread::spawn(move || {
        let hook_writer = open_pipe(HOOK_PIPE, pid, true);
        let _host_reader = open_pipe(HOST_PIPE, pid, false);
        // 서버의 wait_connect()가 이미 성공을 확인한 뒤에만 연결을 끊는다.
        let _ = ready_rx.recv();
        drop(hook_writer);
    });

    server.wait_connect().expect("접속은 성공해야 한다");
    ready_tx
        .send(())
        .expect("클라이언트 스레드에 신호를 보낼 수 있어야 한다");
    let result = perform_handshake(&server);
    assert!(matches!(result, Err(PipeError::Disconnected)));
    client.join().unwrap();
}
