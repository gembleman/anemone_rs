use super::{WorkerRequest, WorkerResponse, write_worker_response};

#[test]
fn worker_protocol_round_trips_multiline_unicode() {
    let request = WorkerRequest {
        text: "一行目\n二行目".into(),
    };
    let encoded = serde_json::to_string(&request).unwrap();
    let decoded: WorkerRequest = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.text, request.text);

    let mut response = Vec::new();
    write_worker_response(&mut response, Ok("첫째 줄\n둘째 줄".into())).unwrap();
    assert_eq!(response.last(), Some(&b'\n'));
    let decoded: WorkerResponse = serde_json::from_slice(&response).unwrap();
    assert_eq!(decoded.result.unwrap(), "첫째 줄\n둘째 줄");
}

#[test]
fn worker_protocol_preserves_engine_errors() {
    let mut response = Vec::new();
    write_worker_response(&mut response, Err("번역 실패".into())).unwrap();
    let decoded: WorkerResponse = serde_json::from_slice(&response).unwrap();
    assert_eq!(decoded.result.unwrap_err(), "번역 실패");
}
