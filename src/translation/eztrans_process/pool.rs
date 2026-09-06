//! 실제 EzTrans helper 자식 프로세스 풀과 JSON Lines 프로토콜 클라이언트.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use super::{EzTransBatchTranslator, EzTransProcessConfig, WorkerRequest, WorkerResponse};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

enum PoolCommand {
    Translate {
        index: usize,
        originals: Vec<Arc<str>>,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
        response: mpsc::Sender<(usize, Result<Vec<String>, String>)>,
    },
    Shutdown,
}

pub(crate) struct EzTransProcessPool {
    config: EzTransProcessConfig,
    workers: Vec<mpsc::Sender<PoolCommand>>,
    joins: Mutex<Vec<JoinHandle<()>>>,
    next_worker: AtomicUsize,
}

impl EzTransProcessPool {
    pub(super) fn new(config: EzTransProcessConfig) -> Result<Self, String> {
        let process_count =
            crate::config::limits::eztrans_process_count_usize(config.process_count);
        let (ready_sender, ready_receiver) = mpsc::channel();
        let mut workers = Vec::with_capacity(process_count);
        let mut joins = Vec::with_capacity(process_count);

        for index in 0..process_count {
            let (sender, receiver) = mpsc::channel();
            let ready = ready_sender.clone();
            let worker_config = config.clone();
            let join = std::thread::Builder::new()
                .name(format!("anemone-eztrans-process-{index}"))
                .spawn(move || worker_loop(worker_config, receiver, ready))
                .map_err(|error| format!("EzTrans helper 관리 스레드 시작 실패: {error}"))?;
            workers.push(sender);
            joins.push(join);
        }
        drop(ready_sender);

        let mut errors = Vec::new();
        for _ in 0..process_count {
            match ready_receiver.recv() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(error),
                Err(_) => errors.push("EzTrans helper 준비 응답 채널이 종료되었습니다".into()),
            }
        }
        if !errors.is_empty() {
            for worker in &workers {
                let _ = worker.send(PoolCommand::Shutdown);
            }
            for join in joins {
                let _ = join.join();
            }
            return Err(format!(
                "EzTrans helper {}/{}개 초기화 실패: {}",
                errors.len(),
                process_count,
                errors.join("; ")
            ));
        }

        let mut normalized = config;
        normalized.process_count = process_count;
        Ok(Self {
            config: normalized,
            workers,
            joins: Mutex::new(joins),
            next_worker: AtomicUsize::new(0),
        })
    }

    pub(super) fn matches(&self, config: &EzTransProcessConfig) -> bool {
        self.config.dictionary_path == config.dictionary_path
            && self.config.ehnd_path == config.ehnd_path
            && self.config.process_count
                == crate::config::limits::eztrans_process_count_usize(config.process_count)
    }
}

impl EzTransBatchTranslator for EzTransProcessPool {
    fn process_count(&self) -> usize {
        self.workers.len()
    }

    fn translate_batches(
        &self,
        batches: Vec<Vec<Arc<str>>>,
        cancelled: &Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Vec<Vec<String>>, String> {
        if batches.is_empty() {
            return Ok(Vec::new());
        }
        let batch_count = batches.len();
        let (response_sender, response_receiver) = mpsc::channel();
        let start = self.next_worker.fetch_add(batch_count, Ordering::Relaxed);
        for (index, originals) in batches.into_iter().enumerate() {
            let worker = &self.workers[(start + index) % self.workers.len()];
            worker
                .send(PoolCommand::Translate {
                    index,
                    originals,
                    cancelled: cancelled.clone(),
                    response: response_sender.clone(),
                })
                .map_err(|_| "EzTrans helper 프로세스 큐가 종료되었습니다".to_string())?;
        }
        drop(response_sender);

        let mut completed = (0..batch_count)
            .map(|_| None)
            .collect::<Vec<Option<Vec<String>>>>();
        let mut remaining = batch_count;
        while remaining > 0 {
            if cancelled.load(Ordering::SeqCst) {
                return Err("사용자가 취소했습니다.".to_string());
            }
            match response_receiver.recv_timeout(std::time::Duration::from_millis(20)) {
                Ok((index, Ok(lines))) => {
                    completed[index] = Some(lines);
                    remaining -= 1;
                }
                Ok((_, Err(error))) => return Err(error),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("EzTrans helper 응답 채널이 종료되었습니다".to_string());
                }
            }
        }
        completed
            .into_iter()
            .map(|batch| {
                // 위 루프가 `remaining == 0`이 될 때까지 모든 index를 채우므로
                // 이 시점에는 항상 `Some`이지만, 프로덕션 코드에 expect를 남기지
                // 않기 위해 방어적으로 오류를 반환한다.
                batch.ok_or_else(|| "EzTrans helper 응답이 누락되었습니다".to_string())
            })
            .collect()
    }
}

impl Drop for EzTransProcessPool {
    fn drop(&mut self) {
        for worker in &self.workers {
            let _ = worker.send(PoolCommand::Shutdown);
        }
        if let Ok(joins) = self.joins.get_mut() {
            for join in joins.drain(..) {
                let _ = join.join();
            }
        }
    }
}

fn worker_loop(
    config: EzTransProcessConfig,
    receiver: mpsc::Receiver<PoolCommand>,
    ready: mpsc::Sender<Result<(), String>>,
) {
    // EHND 초기화가 여러 프로세스에서 동시에 몰리지 않도록 한다.
    // 프로세스는 병렬 실행하되 초기화 구간만 전역 게이트(GUI actor와 공유)로
    // 직렬화해 간헐적 시작 실패를 막는다.
    let initialized = super::super::eztrans_actor::eztrans_initialization_gate()
        .lock()
        .map_err(|_| "EzTrans helper 초기화 잠금이 손상되었습니다".to_string())
        .and_then(|_guard| WorkerClient::spawn(&config));
    let mut client = match initialized {
        Ok(client) => {
            let _ = ready.send(Ok(()));
            client
        }
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };

    while let Ok(command) = receiver.recv() {
        match command {
            PoolCommand::Translate {
                index,
                originals,
                cancelled,
                response,
            } => {
                let result = if cancelled.load(Ordering::SeqCst) {
                    Err("사용자가 취소했습니다.".to_string())
                } else {
                    let mut respawn = || WorkerClient::spawn(&config);
                    translate_resilient(&mut client, &mut respawn, &originals)
                };
                let _ = response.send((index, result));
            }
            PoolCommand::Shutdown => break,
        }
    }
}

#[derive(Debug)]
enum CallError {
    Engine(String),
    Transport(String),
}

/// helper 프로세스와의 한 왕복(단발 번역 요청/응답)을 추상화한다.
///
/// 운영 경로는 항상 실제 자식 프로세스와 통신하는 [`WorkerClient`]를 쓰지만,
/// 재시도/이분 탐색 알고리즘 자체는 실제 프로세스 없이도 테스트할 수 있도록
/// 이 트레이트로 분리해 둔다.
trait EzTransWorkerClient {
    fn translate(&mut self, text: &str) -> Result<String, CallError>;
}

impl EzTransWorkerClient for WorkerClient {
    fn translate(&mut self, text: &str) -> Result<String, CallError> {
        WorkerClient::translate(self, text)
    }
}

fn translate_resilient<C: EzTransWorkerClient>(
    client: &mut C,
    respawn: &mut impl FnMut() -> Result<C, String>,
    originals: &[Arc<str>],
) -> Result<Vec<String>, String> {
    if originals.is_empty() {
        return Ok(Vec::new());
    }
    let combined = originals
        .iter()
        .map(AsRef::as_ref)
        .collect::<Vec<&str>>()
        .join("\n");
    let translated = match client.translate(&combined) {
        Ok(value) => Ok(value),
        Err(CallError::Engine(error)) => Err(CallError::Engine(error)),
        Err(CallError::Transport(first_error)) => {
            *client = respawn().map_err(|restart_error| {
                format!("EzTrans helper 재시작 실패: {restart_error} (최초 오류: {first_error})")
            })?;
            client.translate(&combined)
        }
    };

    match translated {
        Ok(translated) => {
            let refs = originals.iter().map(AsRef::as_ref).collect::<Vec<&str>>();
            if let Some(parts) = crate::file_trans::split_eztrans_batch(&translated, &refs) {
                return Ok(parts);
            }
            tracing::warn!(
                input_lines = originals.len(),
                output_lines = translated.split('\n').count(),
                "EzTrans helper batch changed line boundaries; bisecting batch"
            );
        }
        Err(CallError::Engine(error)) if originals.len() == 1 => {
            return Ok(vec![format!("[번역 실패: {error}]")]);
        }
        Err(CallError::Engine(error)) => {
            tracing::warn!(input_lines = originals.len(), %error, "EzTrans batch failed; bisecting batch");
        }
        Err(CallError::Transport(error)) => return Err(error),
    }

    if originals.len() == 1 {
        return Ok(vec![translated_single_boundary_fallback(
            client,
            respawn,
            &originals[0],
        )?]);
    }
    let midpoint = originals.len() / 2;
    let mut left = translate_resilient(client, respawn, &originals[..midpoint])?;
    left.extend(translate_resilient(
        client,
        respawn,
        &originals[midpoint..],
    )?);
    Ok(left)
}

fn translated_single_boundary_fallback<C: EzTransWorkerClient>(
    client: &mut C,
    respawn: &mut impl FnMut() -> Result<C, String>,
    original: &str,
) -> Result<String, String> {
    match client.translate(original) {
        Ok(value) => Ok(value),
        Err(CallError::Engine(error)) => Ok(format!("[번역 실패: {error}]")),
        Err(CallError::Transport(first_error)) => {
            *client = respawn().map_err(|restart_error| {
                format!("EzTrans helper 재시작 실패: {restart_error} (최초 오류: {first_error})")
            })?;
            match client.translate(original) {
                Ok(value) => Ok(value),
                Err(CallError::Engine(error)) => Ok(format!("[번역 실패: {error}]")),
                Err(CallError::Transport(error)) => Err(error),
            }
        }
    }
}

struct WorkerClient {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl WorkerClient {
    fn spawn(config: &EzTransProcessConfig) -> Result<Self, String> {
        use std::os::windows::process::CommandExt;

        let executable = worker_executable()?;
        let mut command = Command::new(&executable);
        command
            .arg("eztrans-worker")
            .arg("--dictionary")
            .arg(&config.dictionary_path)
            .arg("--ehnd")
            .arg(&config.ehnd_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);
        let mut child = command.spawn().map_err(|error| {
            format!(
                "EzTrans helper 프로세스를 시작할 수 없습니다 ({}): {error}",
                executable.display()
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "EzTrans helper stdin을 열 수 없습니다".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "EzTrans helper stdout을 열 수 없습니다".to_string())?;
        let mut client = Self {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
        };
        let ready = client.read_response().map_err(|error| match error {
            CallError::Engine(error) | CallError::Transport(error) => error,
        })?;
        if ready != "ready" {
            return Err("EzTrans helper가 잘못된 준비 응답을 보냈습니다".to_string());
        }
        Ok(client)
    }

    fn translate(&mut self, text: &str) -> Result<String, CallError> {
        serde_json::to_writer(
            &mut self.stdin,
            &WorkerRequest {
                text: text.to_string(),
            },
        )
        .map_err(|error| CallError::Transport(format!("helper 요청 직렬화 실패: {error}")))?;
        self.stdin
            .write_all(b"\n")
            .and_then(|_| self.stdin.flush())
            .map_err(|error| CallError::Transport(format!("helper 요청 전송 실패: {error}")))?;
        self.read_response()
    }

    fn read_response(&mut self) -> Result<String, CallError> {
        let mut line = String::new();
        let read = self
            .stdout
            .read_line(&mut line)
            .map_err(|error| CallError::Transport(format!("helper 응답 읽기 실패: {error}")))?;
        if read == 0 {
            return Err(CallError::Transport(
                "EzTrans helper가 응답 없이 종료되었습니다".into(),
            ));
        }
        let response: WorkerResponse = serde_json::from_str(&line)
            .map_err(|error| CallError::Transport(format!("helper 응답 파싱 실패: {error}")))?;
        response.result.map_err(CallError::Engine)
    }
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn worker_executable() -> Result<std::path::PathBuf, String> {
    let current = std::env::current_exe()
        .map_err(|error| format!("현재 실행 파일 경로 확인 실패: {error}"))?;
    if current
        .parent()
        .and_then(|parent| parent.file_name())
        .is_some_and(|name| name == "deps")
        && let Some(candidate) = current
            .parent()
            .and_then(|parent| parent.parent())
            .map(|parent| parent.join("anemone_rs.exe"))
        && candidate.is_file()
    {
        return Ok(candidate);
    }
    Ok(current)
}

#[cfg(test)]
#[path = "../../../tests/unit/translation/eztrans_process/pool.rs"]
mod tests;
