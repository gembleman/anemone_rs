//! 파일 번역용 EzTrans helper 프로세스 풀.
//!
//! EzTrans 세션은 번역 컨텍스트를 재사용하므로 같은 프로세스 안에서 인스턴스를
//! 병렬 호출하지 않는다. 대신 같은 실행 파일을 숨김 worker 모드로 실행해 주소 공간을
//! 격리하고, JSON Lines 프로토콜로 배치 번역을 요청한다.

mod pool;

use std::io::{BufRead, BufWriter, Write};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::{EzTransTranslator, Language};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EzTransProcessConfig {
    pub dictionary_path: String,
    pub ehnd_path: String,
    pub process_count: usize,
}

#[derive(Serialize, Deserialize)]
pub(super) struct WorkerRequest {
    text: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct WorkerResponse {
    result: Result<String, String>,
}

/// 파일 워커가 의존하는 최소 배치 번역 인터페이스. 단위 테스트에서는 실제 세션 대신
/// 메모리 mock을 주입한다.
pub trait EzTransBatchTranslator: Send + Sync {
    fn process_count(&self) -> usize;

    fn translate_batches(
        &self,
        batches: Vec<Vec<Arc<str>>>,
        cancelled: &Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Vec<Vec<String>>, String>;
}

pub(crate) struct EzTransProcessPoolRegistry {
    slot: Mutex<Option<Arc<pool::EzTransProcessPool>>>,
}

impl EzTransProcessPoolRegistry {
    pub fn new() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }

    pub fn get(
        &self,
        config: &EzTransProcessConfig,
    ) -> Result<Arc<pool::EzTransProcessPool>, String> {
        let mut guard = self
            .slot
            .lock()
            .map_err(|_| "EzTrans helper 풀 상태가 손상되었습니다".to_string())?;
        if let Some(pool) = guard.as_ref()
            && pool.matches(config)
        {
            return Ok(pool.clone());
        }
        let pool = Arc::new(pool::EzTransProcessPool::new(config.clone())?);
        *guard = Some(pool.clone());
        Ok(pool)
    }
}

/// 숨김 CLI worker 진입점. stdout은 부모와의 프로토콜 전용이다.
pub(crate) fn run_eztrans_worker(dictionary_path: &str, ehnd_path: &str) -> Result<(), String> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut writer = BufWriter::new(stdout.lock());
    let mut translator = match EzTransTranslator::new(dictionary_path, ehnd_path) {
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
#[path = "../../../tests/unit/translation/eztrans_process.rs"]
mod tests;
