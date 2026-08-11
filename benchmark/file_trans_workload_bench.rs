//! 파일 번역 워크로드 측정 (원문 #1 전제 확인).
//!
//! 4차 종결 판정: **#1(파일 번역 줄 단위 요청 → 배치)은 워크로드 측정 대기**.
//! 재논의 조건은 "번역 대상 줄 수 ≥ 10,000" + "줄당 평균 왕복 지연 > 0.5초".
//!
//! 여기서는 실제 앱 경로 그대로(worker → Custom 엔진 → HTTP)로:
//! 1. `should_translate_line` 필터링 후 실제 요청이 나간 줄 수 (mock 서버가 카운트)
//! 2. 줄당 왕복 지연의 로컬 하한 (실제 네트워크 RTT는 여기에 더해진다)

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anemone_rs::{
    BenchmarkConfig as Config, BenchmarkCustomApiConfig as CustomApiConfig,
};
use anemone_rs::file_trans::{
    FileTranslationProgress as ProgressEvent, FileTranslationSupervisor, FileTranslationTask,
    WriteType,
};
use anemone_rs::translation::PreparedJob;

/// 요청마다 즉시 JSON을 돌려주는 로컬 HTTP mock 서버.
/// 요청 수를 카운트해 번역 대상 줄 수를 확인한다.
struct MockServer {
    addr: String,
    request_count: Arc<AtomicU64>,
}

impl MockServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let request_count = Arc::new(AtomicU64::new(0));

        let count = request_count.clone();
        thread::spawn(move || {
            // 파일 번역 worker는 줄 단위로 순차 요청하므로 순차 accept로 충분하다.
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    break;
                };
                count.fetch_add(1, Ordering::Relaxed);
                let _ = serve_one(&mut stream);
            }
        });

        Self {
            addr: format!("http://{addr}/translate"),
            request_count,
        }
    }
}

/// 연결 하나를 한 요청으로 처리하고 응답한다.
fn serve_one(stream: &mut TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    if request_line.trim().is_empty() {
        return Ok(());
    }
    // 헤더와 본문을 소진한다 (Content-Length만큼).
    let mut content_length = 0usize;
    for _ in 0..64 {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if let Some(value) = line
            .to_ascii_lowercase()
            .strip_prefix("content-length:")
        {
            content_length = value.trim().parse().unwrap_or(0);
        }
        if line == "\r\n" {
            break;
        }
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;

    let response_body = r#"{"translation":"번역 결과입니다"}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    stream.write_all(response.as_bytes())?;
    Ok(())
}

fn custom_job(server: &MockServer, input_files: Vec<PathBuf>, output_files: Vec<PathBuf>) -> anemone_rs::file_trans::FileTranslationRequest {
    use anemone_rs::file_trans::FileTranslationRequest;
    let mut config = Config::default();
    config.translation.engine = "custom".to_string();
    config.translation.source_lang = "ja".to_string();
    config.translation.target_lang = "ko".to_string();
    config.translation.custom_apis = vec![CustomApiConfig {
        name: "mock".into(),
        url: server.addr.clone(),
        api_key: String::new(),
        auth_header: String::new(),
        auth_scheme: String::new(),
        headers: "{}".into(),
        request_template: r#"{"text":"{text}"}"#.into(),
        response_path: "/translation".into(),
    }];
    config.translation.custom_api = "mock".into();
    let translation = PreparedJob::from_config(&config.translation).unwrap();

    FileTranslationRequest {
        input_files,
        output_files,
        write_type: WriteType::TranslationOnly,
        no_trans_linefeed: true,
        cancel_token: Arc::new(AtomicBool::new(false)),
        translation,
    }
}

fn receive_through_terminal(task: &FileTranslationTask) -> Vec<ProgressEvent> {
    let mut events = Vec::new();
    loop {
        let event = task
            .recv_event_timeout(Duration::from_secs(30))
            .expect("file translation terminal event");
        let terminal = event.is_terminal();
        events.push(event);
        if terminal {
            events.extend(task.drain_events());
            return events;
        }
    }
}

/// 대표 대사 파일에서 번역 대상 줄 수와 줄당 왕복 지연을 측정한다.
#[test]
#[ignore = "performance benchmark that measures per-line round-trip workload"]
fn measures_per_line_round_trip_workload() {
    assert!(
        !std::hint::black_box(cfg!(debug_assertions)),
        "performance measurements must run with cargo test --release"
    );
    let server = MockServer::start();
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // 10,000줄 고유 문장 샘플 — 번역 대상 줄 수를 그대로 관찰할 수 있다.
    let input = project_root
        .join("benchmark")
        .join("unique_japanese_translation_sample.txt");
    let output = std::env::temp_dir().join(format!(
        "anemone-rt-bench-{}.txt",
        std::process::id()
    ));

    let job = custom_job(&server, vec![input.clone()], vec![output.clone()]);
    let started = Instant::now();
    let supervisor = FileTranslationSupervisor::new();
    let task = supervisor.start(job).unwrap();
    let events = receive_through_terminal(&task);

    let elapsed = started.elapsed();
    let requests = server.request_count.load(Ordering::Relaxed);

    assert!(matches!(
        events.last(),
        Some(ProgressEvent::Finished(Ok(_)))
    ));
    assert_eq!(requests, 10_000, "10,000줄 고유 샘플은 전부 번역 대상이어야 한다");
    eprintln!(
        "[bench file-trans-workload] total_lines=10000 translated_requests={requests} elapsed={:.3}s per_line={:.1}us",
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1_000_000.0 / requests as f64
    );

    // 번역 대상 줄 수가 조건(10,000 이상)을 충족하고, 로컬 왕복 하한을 기록한다.
    // 실제 네트워크 왕복은 RTT + 서버 처리 시간이 더해지므로 로컬 수치는 하한이다.
    let _ = std::fs::remove_file(&output);
}
