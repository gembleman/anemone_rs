use super::{Args, resolve_engine, resolve_languages, resolve_text, run_with_config};
use crate::config::Config;
use crate::translation::Language;

// ---- resolve_text ----

#[test]
fn resolve_text_uses_the_text_argument_and_trims_trailing_newline() {
    let result = resolve_text(Some("안녕하세요\r\n".to_string()), false, || {
        panic!("stdin을 읽으면 안 됩니다")
    });
    assert_eq!(result, Ok("안녕하세요".to_string()));
}

#[test]
fn resolve_text_only_trims_trailing_line_breaks_not_internal_ones() {
    let result = resolve_text(Some("line1\r\nline2\r\n".to_string()), false, || {
        panic!("stdin을 읽으면 안 됩니다")
    });
    assert_eq!(result, Ok("line1\r\nline2".to_string()));
}

#[test]
fn resolve_text_reports_missing_text_when_neither_arg_nor_stdin_is_given() {
    let result = resolve_text(None, false, || panic!("stdin을 읽으면 안 됩니다"));
    assert_eq!(
        result,
        Err("번역할 텍스트가 없습니다. 텍스트를 인자로 주거나 --stdin을 사용하세요.".to_string())
    );
}

#[test]
fn resolve_text_reads_from_the_injected_stdin_reader_when_flag_is_set() {
    let result = resolve_text(None, true, || Ok("stdin 텍스트\n".to_string()));
    assert_eq!(result, Ok("stdin 텍스트".to_string()));
}

#[test]
fn resolve_text_propagates_a_stdin_read_failure() {
    let result = resolve_text(None, true, || {
        Err("stdin 읽기 실패: 파이프 닫힘".to_string())
    });
    assert_eq!(result, Err("stdin 읽기 실패: 파이프 닫힘".to_string()));
}

#[test]
fn resolve_text_rejects_text_that_is_empty_after_trimming() {
    let result = resolve_text(Some("\r\n".to_string()), false, || {
        panic!("stdin을 읽으면 안 됩니다")
    });
    assert_eq!(result, Err("빈 텍스트는 번역할 수 없습니다.".to_string()));
}

// ---- resolve_engine ----

#[test]
fn resolve_engine_prefers_the_cli_override_over_config() {
    let mut config = Config::default();
    config.translation.engine = "deepl".to_string();
    let engine = resolve_engine(Some(super::super::Engine::Google), &config).unwrap();
    assert_eq!(engine, crate::translation::TranslationEngine::Google);
}

#[test]
fn resolve_engine_falls_back_to_config_when_no_override_is_given() {
    let mut config = Config::default();
    config.translation.engine = "deepl".to_string();
    let engine = resolve_engine(None, &config).unwrap();
    assert_eq!(engine, crate::translation::TranslationEngine::DeepL);
}

#[test]
fn resolve_engine_reports_a_corrupted_config_engine() {
    let mut config = Config::default();
    config.translation.engine = "not-a-real-engine".to_string();
    assert!(resolve_engine(None, &config).is_err());
}

// ---- resolve_languages ----

#[test]
fn resolve_languages_uses_explicit_overrides() {
    let config = Config::default();
    let (source, target) =
        resolve_languages(&Some("en".to_string()), &Some("fr".to_string()), &config).unwrap();
    assert_eq!(source, Language::Eng);
    assert_eq!(target, Language::Fra);
}

#[test]
fn resolve_languages_falls_back_to_config_defaults() {
    let config = Config::default();
    let (source, target) = resolve_languages(&None, &None, &config).unwrap();
    assert_eq!(source, Language::Jpn);
    assert_eq!(target, Language::Kor);
}

#[test]
fn resolve_languages_rejects_an_unknown_source_code() {
    let config = Config::default();
    let error = resolve_languages(&Some("xx".to_string()), &None, &config).unwrap_err();
    assert_eq!(error, "알 수 없는 소스 언어 코드: xx");
}

#[test]
fn resolve_languages_rejects_an_unknown_target_code() {
    let config = Config::default();
    let error = resolve_languages(&None, &Some("xx".to_string()), &config).unwrap_err();
    assert_eq!(error, "알 수 없는 타겟 언어 코드: xx");
}

// ---- run_with_config: 실제 사용자 설정 파일을 건드리지 않는 통합 경로 ----

fn args_with_text(text: &str) -> Args {
    Args {
        text: Some(text.to_string()),
        engine: None,
        source: None,
        target: None,
        stdin: false,
    }
}

#[test]
fn run_with_config_translates_end_to_end_through_a_local_custom_api() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

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

    let result = run_with_config(args_with_text("hello"), config);
    assert_eq!(result, Ok(()));
}

#[test]
fn run_with_config_reports_an_invalid_custom_api_url_before_any_network_call() {
    // 잘못된 URL은 job 준비 단계에서 검증에 걸려야 한다 — 네트워크를 재시도하며
    // 기다리는 backend 오류 경로(수 초의 지수 백오프)를 타면 테스트가 느려지므로,
    // 여기서는 네트워크에 닿기 전에 실패하는 경로만 검증한다.
    let mut config = Config::default();
    config.translation.engine = "custom".to_string();
    config.translation.custom.url = String::new();

    let result = run_with_config(args_with_text("hello"), config);
    assert!(result.is_err());
}

#[test]
fn run_with_config_rejects_an_invalid_engine_before_any_network_call() {
    let mut config = Config::default();
    config.translation.engine = "corrupted".to_string();

    let result = run_with_config(args_with_text("hello"), config);
    assert!(result.is_err());
}
