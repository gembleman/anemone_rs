use super::*;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::config::{LlmConfig, TranslationConfig};
use crate::file_trans::FileTransJobData;
use crate::translation::PreparedJob;

fn input_line(text: &str) -> InputLine {
    InputLine {
        text: text.to_string(),
        ending: crate::file_trans::input::LineEnding::Lf,
    }
}

fn llm_job(base_url: &str) -> FileTransJobData {
    FileTransJobData {
        input_files: Vec::new(),
        output_files: Vec::new(),
        write_type: crate::file_trans::WriteType::TranslationOnly,
        no_trans_linefeed: false,
        cancel_token: Arc::new(AtomicBool::new(false)),
        translation: PreparedJob::from_config(&TranslationConfig {
            engine: "llm".into(),
            llm: LlmConfig {
                model: "test".into(),
                api_key: "test".into(),
                base_url: base_url.to_string(),
                system_prompt: String::new(),
                temperature: 0.3,
                max_tokens: 10,
                glossary: Vec::new(),
                ..LlmConfig::default()
            },
            ..TranslationConfig::default()
        })
        .unwrap(),
    }
}

/// 요청 본문(헤더까지 읽은 뒤 Content-Length만큼)을 읽어 반환한다.
fn read_request_body(stream: &mut std::net::TcpStream) -> String {
    use std::io::{BufReader, Read};

    let mut reader = BufReader::new(stream);
    let mut header_bytes = Vec::new();
    let mut probe = [0u8; 1];
    while !header_bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        if reader.read_exact(&mut probe).is_err() || header_bytes.len() > 64 * 1024 {
            break;
        }
        header_bytes.push(probe[0]);
    }
    let header = String::from_utf8_lossy(&header_bytes).to_lowercase();
    let content_length = header
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; content_length];
    let _ = reader.read_exact(&mut body);
    String::from_utf8_lossy(&body).to_string()
}

/// OpenAI Responses API 형식으로 요청 본문의 user 메시지 본문을 그대로 에코한다.
fn write_llm_response(stream: &mut std::net::TcpStream, echo: &str) {
    use std::io::Write;

    let json = format!(
        r#"{{"output":[{{"type":"message","content":[{{"type":"output_text","text":{echo}}}]}}]}}"#,
        echo = serde_json::to_string(echo).unwrap()
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        json.len(),
        json
    );
    let _ = stream.write_all(response.as_bytes());
}

fn extract_user_text(body: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    value
        .pointer("/input")
        .and_then(|input| input.as_str())
        .filter(|text| text.contains("line"))
        .unwrap_or("?")
        .to_string()
}

#[test]
fn translate_lines_runs_concurrently_and_preserves_input_order() {
    const LINE_COUNT: usize = 4;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    // accept 순서(=발신 순서) 역순으로 지연해 늦게 보낸 줄이 먼저 완료되게 만든다.
    // 결과가 입력 순서로 재조립되지 않으면 여기서 드러난다. 연결별로 스레드를
    // 두어 서버 측이 병렬 처리한다 — 클라이언트 동시성을 검증하려면 서버 지연이
    // 직렬로 쌓이면 안 된다.
    std::thread::spawn(move || {
        for index in 0..LINE_COUNT {
            let (mut stream, _) = listener.accept().unwrap();
            std::thread::spawn(move || {
                let body = read_request_body(&mut stream);
                let line = extract_user_text(&body);
                std::thread::sleep(Duration::from_millis(400 - index as u64 * 100));
                write_llm_response(&mut stream, &line);
            });
        }
    });

    let job = llm_job(&format!("http://{address}"));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = crate::translation::http_common::create_client();
    let context = TranslationContext::new(&runtime, &client);
    let lines = (0..LINE_COUNT)
        .map(|index| input_line(&format!("line {index}")))
        .collect::<Vec<_>>();

    let started = Instant::now();
    let results = translate_lines(&lines, &job, &context).unwrap();
    let elapsed = started.elapsed();

    // 지연 400+300+200+100ms가 직렬이면 1초, 동시성이면 최대 400ms.
    assert!(
        elapsed < Duration::from_millis(600),
        "concurrent batch should beat sequential completion: {elapsed:?}"
    );
    assert_eq!(results, vec!["line 0", "line 1", "line 2", "line 3"]);
}

#[test]
fn translate_lines_batch_cancellation_is_prompt() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = listener.accept();
        std::thread::sleep(Duration::from_secs(10));
    });

    let cancel_token = Arc::new(AtomicBool::new(false));
    let mut job = llm_job(&format!("http://{address}"));
    job.cancel_token = Arc::clone(&cancel_token);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = crate::translation::http_common::create_client();
    let context = TranslationContext::new(&runtime, &client);
    let lines = (0..4)
        .map(|index| input_line(&format!("line {index}")))
        .collect::<Vec<_>>();

    let cancel = Arc::clone(&cancel_token);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        cancel.store(true, Ordering::SeqCst);
    });

    let started = Instant::now();
    let result = translate_lines(&lines, &job, &context);

    assert!(matches!(result, Err(FileTranslationError::Cancelled)));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "batch cancellation should not wait for the hanging server"
    );
}
