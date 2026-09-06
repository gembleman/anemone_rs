use super::{Args, WriteType, run_with_config};
use crate::config::Config;
use crate::file_trans::WriteType as CoreWriteType;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "anemone-cli-file-trans-test-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn args(input: PathBuf, output: PathBuf, format: WriteType) -> Args {
    Args {
        input,
        output,
        engine: None,
        source: None,
        target: None,
        format,
        no_trans_linefeed: false,
    }
}

// ---- WriteType -> CoreWriteType 매핑 ----

#[test]
fn every_cli_write_type_maps_to_the_matching_core_write_type() {
    let cases = [
        (WriteType::Only, CoreWriteType::TranslationOnly),
        (WriteType::Both, CoreWriteType::OriginalAndTrans),
        (WriteType::BothNl, CoreWriteType::OriginalTransNewline),
    ];
    for (cli_value, expected) in cases {
        assert_eq!(CoreWriteType::from(cli_value), expected);
    }
}

// ---- run_with_config: 실제 사용자 설정 파일 없이 파일 번역 경로를 검증 ----

#[test]
fn run_with_config_translates_a_file_end_to_end_through_a_local_custom_api() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let dir = TestDirectory::new();
    let input_path = dir.0.join("input.txt");
    let output_path = dir.0.join("output.txt");
    std::fs::write(&input_path, "hello\n").unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let body = r#"{"translatedText":"안녕하세요"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let mut config = Config::default();
    config.translation.engine = "custom".to_string();
    config.translation.custom.url = format!("http://{address}/translate");

    let result = run_with_config(
        args(input_path, output_path.clone(), WriteType::Only),
        config,
    );
    assert_eq!(result, Ok(()));
    // 출력 파일은 항상 UTF-8 BOM으로 시작한다.
    assert_eq!(
        std::fs::read_to_string(&output_path).unwrap(),
        "\u{feff}안녕하세요\n"
    );
}

#[test]
fn run_with_config_rejects_an_output_path_equal_to_the_input_path_before_any_io() {
    let dir = TestDirectory::new();
    let path = dir.0.join("same.txt");
    std::fs::write(&path, "hello\n").unwrap();

    let mut config = Config::default();
    config.translation.engine = "custom".to_string();
    config.translation.custom.url = "http://127.0.0.1:1/translate".to_string();

    let result = run_with_config(args(path.clone(), path, WriteType::Only), config);
    assert!(result.is_err());
}

#[test]
fn run_with_config_rejects_an_invalid_engine_before_any_io() {
    let dir = TestDirectory::new();
    let input_path = dir.0.join("input.txt");
    let output_path = dir.0.join("output.txt");
    std::fs::write(&input_path, "hello\n").unwrap();

    let mut config = Config::default();
    config.translation.engine = "corrupted".to_string();

    let result = run_with_config(args(input_path, output_path, WriteType::Only), config);
    assert!(result.is_err());
}

#[test]
fn the_hook_bound_engine_is_not_a_file_trans_engine_option() {
    use clap::ValueEnum;

    let names: Vec<String> = super::FileEngine::value_variants()
        .iter()
        .filter_map(|variant| {
            variant
                .to_possible_value()
                .map(|value| value.get_name().to_string())
        })
        .collect();

    assert!(!names.contains(&"mys_translater".to_string()), "{names:?}");
    for engine in crate::translation::TranslationEngine::ALL {
        assert_eq!(
            names.contains(&engine.to_str().to_string()),
            engine.supports_file_translation(),
            "{engine:?}"
        );
    }
}

#[test]
fn run_with_config_rejects_a_configured_hook_bound_engine_before_any_io() {
    let dir = TestDirectory::new();
    let input_path = dir.0.join("input.txt");
    let output_path = dir.0.join("output.txt");
    std::fs::write(&input_path, "こんにちは\n").unwrap();

    let mut config = Config::default();
    config.translation.engine = "mys_translater".to_string();

    let result = run_with_config(
        args(input_path, output_path.clone(), WriteType::Only),
        config,
    );
    assert!(result.is_err());
    assert!(!output_path.exists());
}
