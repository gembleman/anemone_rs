//! 파일 번역용 EzTrans helper 프로세스 풀.
//!
//! EzTrans DLL은 프로세스 전역 상태를 사용하므로 같은 프로세스 안에서 인스턴스를
//! 병렬 호출하지 않는다. 대신 같은 실행 파일을 숨김 worker 모드로 실행해 주소 공간을
//! 격리하고, JSON Lines 프로토콜로 배치 번역을 요청한다.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use serde::{Deserialize, Serialize};

use super::eztrans_actor::RegisteredDllDirectory;
use super::{EzTransTranslator, Language};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EzTransProcessConfig {
    pub dll_path: String,
    pub dat_path: String,
    pub process_count: usize,
}

#[derive(Serialize, Deserialize)]
struct WorkerRequest {
    text: String,
}

#[derive(Serialize, Deserialize)]
struct WorkerResponse {
    result: Result<String, String>,
}

/// 파일 워커가 의존하는 최소 배치 번역 인터페이스. 단위 테스트에서는 DLL 대신
/// 메모리 mock을 주입한다.
pub trait EzTransBatchTranslator: Send + Sync {
    fn process_count(&self) -> usize;

    fn translate_batches(
        &self,
        batches: Vec<Vec<Arc<str>>>,
        cancelled: &Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Vec<Vec<String>>, String>;
}

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
    fn new(config: EzTransProcessConfig) -> Result<Self, String> {
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

    fn matches(&self, config: &EzTransProcessConfig) -> bool {
        self.config.dll_path == config.dll_path
            && self.config.dat_path == config.dat_path
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
        Ok(completed
            .into_iter()
            .map(|batch| batch.expect("all helper responses were collected"))
            .collect())
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
    // EHND 초기화는 여러 프로세스에서도 DAT 내부의 공유 파일을 동시에 만질 수 있다.
    // 프로세스는 병렬 실행하되 초기화 구간만 전역 게이트(GUI actor와 공유)로
    // 직렬화해 간헐적 시작 실패를 막는다.
    let initialized = super::eztrans_actor::eztrans_initialization_gate()
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
                    translate_resilient(&mut client, &config, &originals)
                };
                let _ = response.send((index, result));
            }
            PoolCommand::Shutdown => break,
        }
    }
}

enum CallError {
    Engine(String),
    Transport(String),
}

fn translate_resilient(
    client: &mut WorkerClient,
    config: &EzTransProcessConfig,
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
            *client = WorkerClient::spawn(config).map_err(|restart_error| {
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
            config,
            &originals[0],
        )?]);
    }
    let midpoint = originals.len() / 2;
    let mut left = translate_resilient(client, config, &originals[..midpoint])?;
    left.extend(translate_resilient(client, config, &originals[midpoint..])?);
    Ok(left)
}

fn translated_single_boundary_fallback(
    client: &mut WorkerClient,
    config: &EzTransProcessConfig,
    original: &str,
) -> Result<String, String> {
    match client.translate(original) {
        Ok(value) => Ok(value),
        Err(CallError::Engine(error)) => Ok(format!("[번역 실패: {error}]")),
        Err(CallError::Transport(first_error)) => {
            *client = WorkerClient::spawn(config).map_err(|restart_error| {
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
            .arg("--dll")
            .arg(&config.dll_path)
            .arg("--dat")
            .arg(&config.dat_path)
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

pub(crate) struct EzTransProcessPoolRegistry {
    slot: Mutex<Option<Arc<EzTransProcessPool>>>,
}

impl EzTransProcessPoolRegistry {
    pub fn new() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }

    pub fn get(&self, config: &EzTransProcessConfig) -> Result<Arc<EzTransProcessPool>, String> {
        let mut guard = self
            .slot
            .lock()
            .map_err(|_| "EzTrans helper 풀 상태가 손상되었습니다".to_string())?;
        if let Some(pool) = guard.as_ref()
            && pool.matches(config)
        {
            return Ok(pool.clone());
        }
        let pool = Arc::new(EzTransProcessPool::new(config.clone())?);
        *guard = Some(pool.clone());
        Ok(pool)
    }
}

/// 숨김 CLI worker 진입점. stdout은 부모와의 프로토콜 전용이다.
pub(crate) fn run_eztrans_worker(dll_path: &str, dat_path: &str) -> Result<(), String> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut writer = BufWriter::new(stdout.lock());
    let _dll_directory = match RegisteredDllDirectory::register(dll_path) {
        Ok(directory) => directory,
        Err(error) => {
            write_worker_response(&mut writer, Err(error.clone()))?;
            return Err(error);
        }
    };
    let translator = match EzTransTranslator::new(dll_path, dat_path) {
        Ok(translator) => {
            write_worker_response(&mut writer, Ok("ready".into()))?;
            translator
        }
        Err(error) => {
            write_worker_response(&mut writer, Err(error.clone()))?;
            return Err(error);
        }
    };

    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("helper 요청 읽기 실패: {error}"))?;
        if read == 0 {
            return Ok(());
        }
        let request: WorkerRequest = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                write_worker_response(&mut writer, Err(format!("helper 요청 파싱 실패: {error}")))?;
                continue;
            }
        };
        let result = translator
            .translate(&request.text, Language::Jpn, Language::Kor)
            .map_err(|error| error.to_string());
        write_worker_response(&mut writer, result)?;
    }
}

fn write_worker_response(
    writer: &mut impl Write,
    result: Result<String, String>,
) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, &WorkerResponse { result })
        .map_err(|error| format!("helper 응답 직렬화 실패: {error}"))?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|error| format!("helper 응답 전송 실패: {error}"))
}

#[cfg(test)]
#[path = "../../tests/unit/translation/eztrans_process.rs"]
mod tests;
