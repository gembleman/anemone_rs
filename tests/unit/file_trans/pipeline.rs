use super::*;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::config::{LlmConfig, TranslationConfig};
use crate::file_trans::FileTranslationRequest;
use crate::translation::PreparedJob;

fn input_line(text: &str) -> InputLine {
    InputLine {
        text: text.to_string(),
        ending: crate::file_trans::input::LineEnding::Lf,
    }
}

fn llm_job(base_url: &str) -> FileTranslationRequest {
    FileTranslationRequest {
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
    // 동시성이 실제로 일어났는지는 벽시계 시간이 아니라 서버가 관측한 동시
    // in-flight 요청 수의 최댓값으로 판정한다. 시간 임계값은 테스트를 병렬
    // 실행할 때의 CPU 경합만으로도 흔들려 flaky해진다(단독 실행은 통과, 전체
    // 실행 중 간헐적 실패). accept 순서(=발신 순서) 역순으로 지연해 늦게 보낸
    // 줄이 먼저 완료되게 만든다 — 결과가 입력 순서로 재조립되지 않으면 여기서
    // 드러난다. 연결별로 스레드를 두어 서버 측이 병렬 처리한다.
    let in_flight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let max_in_flight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_in_flight = Arc::clone(&in_flight);
    let server_max = Arc::clone(&max_in_flight);
    std::thread::spawn(move || {
        for index in 0..LINE_COUNT {
            let (mut stream, _) = listener.accept().unwrap();
            let in_flight = Arc::clone(&server_in_flight);
            let max_in_flight = Arc::clone(&server_max);
            std::thread::spawn(move || {
                let body = read_request_body(&mut stream);
                let line = extract_user_text(&body);
                // 본문을 다 읽은 시점부터가 진짜 in-flight 구간이다.
                let current = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                max_in_flight.fetch_max(current, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(400 - index as u64 * 100));
                in_flight.fetch_sub(1, Ordering::SeqCst);
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

    let results = translate_lines(&lines, &job, &context).unwrap();

    // 요청이 순차적으로만 나갔다면(동시성이 깨졌다면) 관측 최댓값은 1을
    // 넘지 못한다. LINE_COUNT(4)는 FILE_HTTP_CONCURRENCY(8) 이하이므로
    // 세마포어에 걸리지 않고 전부 동시에 in-flight가 되어야 한다.
    assert_eq!(
        max_in_flight.load(Ordering::SeqCst),
        LINE_COUNT,
        "translate_lines should run requests concurrently"
    );
    assert_eq!(results, vec!["line 0", "line 1", "line 2", "line 3"]);
}

/// `process_single_file`의 배치 수집 루프(결함 1)를 실제 파이프라인 경로로
/// 태워서 검증한다. 기존 `translate_lines_runs_concurrently_and_preserves_input_order`는
/// `translate_lines`를 직접 호출해 이미 여러 줄이 모인 슬라이스를 넘기므로,
/// non-blocking 엔진에서 배치가 항상 1줄로 쪼개지는 버그를 잡아내지 못했다.
/// 이 테스트는 `run`(=`run_inner` -> `process_single_file`)을 그대로 실행해
/// 배치 수집부터 검증한다.
#[test]
fn pipeline_batches_non_blocking_lines_and_overlaps_requests() {
    const LINE_COUNT: usize = 4;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    // 배치가 실제로 겹쳤는지는 벽시계 시간이 아니라 서버가 관측한 동시 in-flight
    // 요청 수의 최댓값으로 판정한다. 시간 임계값은 테스트 350여 개를 병렬 실행할
    // 때의 CPU 경합만으로도 흔들려(단독 0.4s vs 전체 실행 0.8s) flaky해진다.
    let in_flight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let max_in_flight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_in_flight = Arc::clone(&in_flight);
    let server_max = Arc::clone(&max_in_flight);
    std::thread::spawn(move || {
        for _ in 0..LINE_COUNT {
            let (mut stream, _) = listener.accept().unwrap();
            let in_flight = Arc::clone(&server_in_flight);
            let max_in_flight = Arc::clone(&server_max);
            std::thread::spawn(move || {
                let body = read_request_body(&mut stream);
                let line = extract_user_text(&body);
                // 본문을 다 읽은 시점부터가 진짜 in-flight 구간이다.
                let current = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                max_in_flight.fetch_max(current, Ordering::SeqCst);
                // 모든 요청이 겹칠 기회를 갖도록 충분히 붙잡아 둔다. 결함 1이
                // 되살아나 줄마다 순차 왕복이 되면 동시 관측치는 1을 넘지 못한다.
                std::thread::sleep(Duration::from_millis(200));
                in_flight.fetch_sub(1, Ordering::SeqCst);
                write_llm_response(&mut stream, &line);
            });
        }
    });

    let directory = std::env::temp_dir().join(format!(
        "anemone-pipeline-batch-test-{}-{}",
        std::process::id(),
        address.port()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let input_path = directory.join("input.txt");
    let output_path = directory.join("output.txt");
    let input_text = (0..LINE_COUNT)
        .map(|index| format!("line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&input_path, input_text).unwrap();

    let mut job = llm_job(&format!("http://{address}"));
    job.input_files = vec![input_path];
    job.output_files = vec![output_path.clone()];

    let events = std::sync::Mutex::new(Vec::new());
    run(&job, |event| events.lock().unwrap().push(event));

    let events = events.into_inner().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, FileTranslationProgress::Finished(Ok(_))))
            .count(),
        1,
        "pipeline should finish successfully exactly once: {events:?}"
    );

    // 결함 1이 되살아나 배치가 1줄로 쪼개지면 요청이 절대 겹치지 않아 관측
    // 최댓값이 1이 된다. LINE_COUNT(4)는 FILE_HTTP_CONCURRENCY(8) 이하이므로
    // 세마포어에 걸리지 않고 전부 동시에 in-flight가 되어야 한다.
    assert_eq!(
        max_in_flight.load(Ordering::SeqCst),
        LINE_COUNT,
        "non-blocking engine lines should be batched and overlapped end-to-end"
    );

    let output = std::fs::read_to_string(&output_path).unwrap();
    let output = output.strip_prefix('\u{feff}').unwrap_or(&output);
    let translated_lines: Vec<&str> = output.lines().collect();
    assert_eq!(
        translated_lines,
        (0..LINE_COUNT)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
    );

    let _ = std::fs::remove_dir_all(&directory);
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
