use super::*;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

/// 호출 순서/전달된 텍스트를 기록하는 공유 로그. 재시작으로 `client`가
/// 교체되어도 같은 로그를 계속 공유하도록 `Rc<RefCell<_>>`로 둔다.
#[derive(Clone, Default)]
struct CallLog(Rc<RefCell<Vec<String>>>);

impl CallLog {
    fn record(&self, text: &str) {
        self.0.borrow_mut().push(text.to_string());
    }

    fn calls(&self) -> Vec<String> {
        self.0.borrow().clone()
    }
}

/// 실제 자식 프로세스 대신 스크립트된 응답을 순서대로 반환하는 fake 클라이언트.
struct FakeWorkerClient {
    responses: VecDeque<Result<String, CallError>>,
    log: CallLog,
}

impl FakeWorkerClient {
    fn new(log: CallLog, responses: Vec<Result<String, CallError>>) -> Self {
        Self {
            responses: responses.into(),
            log,
        }
    }
}

impl EzTransWorkerClient for FakeWorkerClient {
    fn translate(&mut self, text: &str) -> Result<String, CallError> {
        self.log.record(text);
        self.responses
            .pop_front()
            .unwrap_or_else(|| panic!("스크립트에 없는 추가 translate 호출: {text}"))
    }
}

fn arcs(items: &[&str]) -> Vec<Arc<str>> {
    items.iter().map(|s| Arc::from(*s)).collect()
}

/// 절대 호출되지 않아야 하는 respawn (재시작이 필요 없는 시나리오용).
fn never_respawn() -> impl FnMut() -> Result<FakeWorkerClient, String> {
    || panic!("이 시나리오에서는 재시작이 필요 없다")
}

#[test]
fn a_matching_line_count_response_is_split_without_any_retry() {
    let log = CallLog::default();
    let mut client = FakeWorkerClient::new(log.clone(), vec![Ok("Hello\nBye".to_string())]);
    let mut respawn = never_respawn();

    let result = translate_resilient(&mut client, &mut respawn, &arcs(&["안녕", "잘가"]));

    assert_eq!(
        result.unwrap(),
        vec!["Hello".to_string(), "Bye".to_string()]
    );
    assert_eq!(log.calls(), vec!["안녕\n잘가".to_string()]);
}

#[test]
fn a_single_item_engine_error_becomes_an_inline_failure_marker() {
    let log = CallLog::default();
    let mut client = FakeWorkerClient::new(
        log.clone(),
        vec![Err(CallError::Engine("모델 오류".to_string()))],
    );
    let mut respawn = never_respawn();

    let result = translate_resilient(&mut client, &mut respawn, &arcs(&["안녕"]));

    assert_eq!(result.unwrap(), vec!["[번역 실패: 모델 오류]".to_string()]);
    assert_eq!(log.calls(), vec!["안녕".to_string()]);
}

#[test]
fn a_batch_engine_error_bisects_into_independent_single_calls() {
    let log = CallLog::default();
    let mut client = FakeWorkerClient::new(
        log.clone(),
        vec![
            Err(CallError::Engine("배치 실패".to_string())),
            Ok("A".to_string()),
            Ok("B".to_string()),
        ],
    );
    let mut respawn = never_respawn();

    let result = translate_resilient(&mut client, &mut respawn, &arcs(&["a", "b"]));

    assert_eq!(result.unwrap(), vec!["A".to_string(), "B".to_string()]);
    assert_eq!(
        log.calls(),
        vec!["a\nb".to_string(), "a".to_string(), "b".to_string()]
    );
}

#[test]
fn a_line_count_mismatch_bisects_even_though_the_call_succeeded() {
    let log = CallLog::default();
    let mut client = FakeWorkerClient::new(
        log.clone(),
        vec![
            // 두 줄을 보냈는데 한 줄만 돌아와 원문과 줄 수가 맞지 않는다.
            Ok("한줄로뭉침".to_string()),
            Ok("A".to_string()),
            Ok("B".to_string()),
        ],
    );
    let mut respawn = never_respawn();

    let result = translate_resilient(&mut client, &mut respawn, &arcs(&["a", "b"]));

    assert_eq!(result.unwrap(), vec!["A".to_string(), "B".to_string()]);
}

#[test]
fn a_transport_error_triggers_exactly_one_respawn_then_succeeds() {
    let log = CallLog::default();
    let mut client = FakeWorkerClient::new(
        log.clone(),
        vec![Err(CallError::Transport("연결 끊김".to_string()))],
    );
    let spawn_count = Rc::new(RefCell::new(0));
    let spawn_count_clone = spawn_count.clone();
    let respawned_log = log.clone();
    let mut respawn = move || {
        *spawn_count_clone.borrow_mut() += 1;
        Ok(FakeWorkerClient::new(
            respawned_log.clone(),
            vec![Ok("Y".to_string())],
        ))
    };

    let result = translate_resilient(&mut client, &mut respawn, &arcs(&["x"]));

    assert_eq!(result.unwrap(), vec!["Y".to_string()]);
    assert_eq!(
        *spawn_count.borrow(),
        1,
        "재시작은 정확히 한 번만 일어나야 한다"
    );
    assert_eq!(log.calls(), vec!["x".to_string(), "x".to_string()]);
}

#[test]
fn a_failed_respawn_reports_both_the_original_and_restart_errors() {
    let log = CallLog::default();
    let mut client = FakeWorkerClient::new(
        log.clone(),
        vec![Err(CallError::Transport("최초 연결 끊김".to_string()))],
    );
    let mut respawn = || Err::<FakeWorkerClient, _>("재시작 실패 이유".to_string());

    let result = translate_resilient(&mut client, &mut respawn, &arcs(&["x"]));

    let error = result.unwrap_err();
    assert!(error.contains("최초 연결 끊김"), "error: {error}");
    assert!(error.contains("재시작 실패 이유"), "error: {error}");
}

#[test]
fn a_failed_respawn_for_a_multi_item_batch_returns_before_bisecting() {
    let log = CallLog::default();
    let mut client = FakeWorkerClient::new(
        log.clone(),
        vec![Err(CallError::Transport("복구 불가".to_string()))],
    );
    let mut respawn = || Err::<FakeWorkerClient, _>("역시 실패".to_string());

    // originals가 2개라도 respawn 자체가 실패하면 이분 탐색으로 내려가지 않고 즉시 반환한다.
    let result = translate_resilient(&mut client, &mut respawn, &arcs(&["a", "b"]));
    assert!(result.is_err());
    assert_eq!(log.calls(), vec!["a\nb".to_string()]);
}

#[test]
fn a_single_item_line_mismatch_falls_back_to_a_fresh_single_shot_call() {
    let log = CallLog::default();
    let mut client = FakeWorkerClient::new(
        log.clone(),
        vec![
            // 원문은 한 줄인데 응답이 두 줄로 쪼개져 경계가 어긋난다.
            Ok("line1\nline2".to_string()),
            Ok("최종번역".to_string()),
        ],
    );
    let mut respawn = never_respawn();

    let result = translate_resilient(&mut client, &mut respawn, &arcs(&["hello"]));

    assert_eq!(result.unwrap(), vec!["최종번역".to_string()]);
    assert_eq!(log.calls(), vec!["hello".to_string(), "hello".to_string()]);
}

#[test]
fn single_boundary_fallback_retries_once_after_a_transport_error_and_succeeds() {
    let log = CallLog::default();
    let mut client =
        FakeWorkerClient::new(log.clone(), vec![Err(CallError::Transport("끊김".into()))]);
    let mut respawn = || {
        Ok(FakeWorkerClient::new(
            log.clone(),
            vec![Ok("성공".to_string())],
        ))
    };

    let result = translated_single_boundary_fallback(&mut client, &mut respawn, "hello");
    assert_eq!(result.unwrap(), "성공");
}

#[test]
fn single_boundary_fallback_wraps_an_engine_error_on_the_retry_attempt_too() {
    let log = CallLog::default();
    let mut client =
        FakeWorkerClient::new(log.clone(), vec![Err(CallError::Transport("끊김".into()))]);
    let mut respawn = || {
        Ok(FakeWorkerClient::new(
            log.clone(),
            vec![Err(CallError::Engine("실패2".to_string()))],
        ))
    };

    let result = translated_single_boundary_fallback(&mut client, &mut respawn, "hello");
    assert_eq!(result.unwrap(), "[번역 실패: 실패2]");
}

#[test]
fn single_boundary_fallback_surfaces_a_transport_error_on_the_retry_attempt() {
    let log = CallLog::default();
    let mut client =
        FakeWorkerClient::new(log.clone(), vec![Err(CallError::Transport("끊김".into()))]);
    let mut respawn = || {
        Ok(FakeWorkerClient::new(
            log.clone(),
            vec![Err(CallError::Transport("실패3".to_string()))],
        ))
    };

    let result = translated_single_boundary_fallback(&mut client, &mut respawn, "hello");
    assert_eq!(result.unwrap_err(), "실패3");
}

#[test]
fn single_boundary_fallback_reports_a_failed_respawn() {
    let log = CallLog::default();
    let mut client =
        FakeWorkerClient::new(log.clone(), vec![Err(CallError::Transport("끊김".into()))]);
    let mut respawn = || Err::<FakeWorkerClient, _>("재시작실패".to_string());

    let result = translated_single_boundary_fallback(&mut client, &mut respawn, "hello");
    let error = result.unwrap_err();
    assert!(error.contains("끊김"));
    assert!(error.contains("재시작실패"));
}

#[test]
fn worker_executable_always_resolves_to_a_path_that_exists() {
    let path = worker_executable().unwrap();
    assert!(
        path.is_file(),
        "worker_executable()이 존재하지 않는 경로를 반환함: {path:?}"
    );
}
