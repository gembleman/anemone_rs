use super::*;

#[test]
fn rolling_writer_bounds_total_log_storage() {
    let directory = std::env::temp_dir().join(format!(
        "anemone-log-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&directory);
    let mut writer = RollingLogWriter::open(&directory).unwrap();
    let block = vec![b'x'; 300 * 1024];
    for _ in 0..20 {
        writer.write_all(&block).unwrap();
    }
    writer.flush().unwrap();
    drop(writer);

    let files: Vec<_> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect();
    let total: u64 = files
        .iter()
        .map(|entry| entry.metadata().unwrap().len())
        .sum();
    assert!(files.len() <= LOG_FILE_COUNT);
    assert!(total <= LOG_FILE_BYTES * LOG_FILE_COUNT as u64);
    fs::remove_dir_all(directory).unwrap();
}

/// 후킹 디버그 로그는 앱 로그와 같은 디렉터리에 이름만 달리 쌓인다. 회전한
/// 세대도 자기 이름을 쓰므로 두 로그가 서로를 지우지 않는다.
#[test]
fn a_named_log_rotates_under_its_own_name() {
    let directory = std::env::temp_dir().join(format!(
        "anemone-named-log-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&directory);

    let mut app = RollingLogWriter::open(&directory).unwrap();
    app.write_all(b"app\n").unwrap();
    app.flush().unwrap();
    drop(app);

    let limit = 1024;
    let mut hook = RollingLogWriter::open_named(&directory, "lunahook.log", limit).unwrap();
    let block = vec![b'y'; 400];
    for _ in 0..6 {
        hook.write_all(&block).unwrap();
    }
    hook.flush().unwrap();
    drop(hook);

    assert!(directory.join("lunahook.log.1").exists());
    assert!(!directory.join(format!("{APP_LOG_NAME}.1")).exists());
    assert_eq!(
        fs::read(directory.join(APP_LOG_NAME)).unwrap(),
        b"app\n",
        "앱 로그는 그대로 남는다"
    );
    for generation in 0..LOG_FILE_COUNT {
        let path = if generation == 0 {
            directory.join("lunahook.log")
        } else {
            directory.join(format!("lunahook.log.{generation}"))
        };
        if path.exists() {
            assert!(fs::metadata(&path).unwrap().len() <= limit);
        }
    }
    fs::remove_dir_all(directory).unwrap();
}

/// 로그 쓰기가 부르는 쪽을 막으면 안 된다. 후킹 로그는 파이프를 읽는
/// 스레드에서 나오고, 거기가 밀리면 DLL의 동기 `WriteFile`이 막혀 게임이
/// 멈춘다. 대기열이 넘쳐도 버릴지언정 블록하지 않아야 한다.
#[test]
fn the_async_writer_never_blocks_its_caller() {
    let directory = std::env::temp_dir().join(format!(
        "anemone-async-log-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&directory);
    let inner = RollingLogWriter::open_named(&directory, "async.log", 1024 * 1024).unwrap();
    let mut writer = AsyncLogWriter::spawn(inner);

    // 대기열 길이를 훌쩍 넘겨 밀어 넣는다. 넘친 것은 버려지되 멈추면 안 된다.
    let started = std::time::Instant::now();
    for index in 0..(LOG_QUEUE_LINES * 4) {
        writer
            .write_all(format!("line {index}\n").as_bytes())
            .unwrap();
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "쓰기가 막혔다: {elapsed:?}"
    );

    // 쓰기 스레드가 실제로 파일에 남긴다.
    let path = directory.join("async.log");
    let mut written = 0;
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        written = fs::metadata(&path).map_or(0, |metadata| metadata.len());
        if written > 0 {
            break;
        }
    }
    assert!(written > 0, "로그 스레드가 아무것도 쓰지 않았다");

    drop(writer);
    let _ = fs::remove_dir_all(&directory);
}
