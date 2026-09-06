use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use tracing_subscriber::filter::{LevelFilter, filter_fn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, fmt};

pub const LOG_FILE_BYTES: u64 = 1024 * 1024;
pub const LOG_FILE_COUNT: usize = 4;

pub const APP_LOG_NAME: &str = "anemone.log";

pub struct RollingLogWriter {
    directory: PathBuf,
    name: String,
    limit: u64,
    file: Option<File>,
    length: u64,
}

impl RollingLogWriter {
    pub fn open(directory: &Path) -> io::Result<Self> {
        Self::open_named(directory, APP_LOG_NAME, LOG_FILE_BYTES)
    }

    /// 이름과 파일 상한만 다른, 같은 회전 정책의 로그를 연다. 후킹 디버그
    /// 로그처럼 앱 로그와 섞이면 안 되는 기록을 따로 담는 데 쓴다.
    pub fn open_named(directory: &Path, name: &str, limit: u64) -> io::Result<Self> {
        fs::create_dir_all(directory)?;
        let path = directory.join(name);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let length = file.metadata().map_or(0, |metadata| metadata.len());
        Ok(Self {
            directory: directory.to_path_buf(),
            name: name.to_string(),
            limit: limit.max(1),
            file: Some(file),
            length,
        })
    }

    fn generation_path(&self, generation: usize) -> PathBuf {
        self.directory.join(format!("{}.{generation}", self.name))
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.take();
        let oldest = self.generation_path(LOG_FILE_COUNT - 1);
        if oldest.exists() {
            fs::remove_file(oldest)?;
        }
        for generation in (1..LOG_FILE_COUNT - 1).rev() {
            let source = self.generation_path(generation);
            if source.exists() {
                fs::rename(source, self.generation_path(generation + 1))?;
            }
        }
        let current = self.directory.join(&self.name);
        if current.exists() {
            fs::rename(&current, self.generation_path(1))?;
        }
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(current)?,
        );
        self.length = 0;
        Ok(())
    }
}

impl Write for RollingLogWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.length > 0 && self.length.saturating_add(buffer.len() as u64) > self.limit {
            self.rotate()?;
        }
        // 단일 이벤트가 상한보다 큰 경우에도 파일 상한을 지킨다. 애플리케이션
        // 이벤트는 원문을 기록하지 않으므로 정상적으로는 이 경로에 들어오지 않는다.
        let allowed = (self.limit - self.length).min(buffer.len() as u64) as usize;
        let written = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("log file is closed"))?
            .write(&buffer[..allowed])?;
        self.length += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("log file is closed"))?
            .flush()
    }
}

/// 로그 줄을 전용 스레드로 넘겨 쓰게 한다.
///
/// 후킹 디버그 로그는 글자 단위 훅에서 초당 수천 줄이 온다. 그 파일 쓰기를
/// 파이프를 읽는 스레드에서 직접 하면 읽기가 그만큼 밀리고, 파이프 버퍼가
/// 차면 DLL의 동기 `WriteFile`이 막혀 **게임 스레드가 멈춘다.** 로그를 켰다고
/// 게임이 느려지면 안 되므로 쓰기를 이 스레드 밖으로 뺀다.
///
/// 밀린 줄은 한 번에 모아 쓴다 — 바쁠수록 배치가 커져 시스템 콜이 줄고,
/// 한가하면 곧바로 flush되어 로그가 바로 보인다.
struct AsyncLogWriter {
    sender: std::sync::mpsc::SyncSender<Vec<u8>>,
}

/// 살아 있는 쓰기 스레드들. subscriber가 writer를 프로세스 수명 내내 들고 있어
/// sender가 절대 drop되지 않으므로, 종료 시 [`flush_and_stop`]이 여기서 꺼내
/// 빈 배치(중지 신호)를 보내고 기다린다.
static LOG_THREADS: std::sync::Mutex<Vec<LogThread>> = std::sync::Mutex::new(Vec::new());

struct LogThread {
    sender: std::sync::mpsc::SyncSender<Vec<u8>>,
    handle: std::thread::JoinHandle<()>,
}

/// 대기열에 남은 줄을 쓰고 로그 스레드를 끝낸다. 프로세스 종료 직전에 한 번
/// 부른다 — 그러지 않으면 마지막 배치가 통째로 사라진다.
///
/// 제한 시간을 두고, 못 끝내면 두고 넘어간다. 로그를 마저 쓰겠다고 종료가
/// 멈춰 서면 안 된다 — 후킹 워커와 같은 정책이다.
pub fn flush_and_stop() {
    let threads = {
        let mut slot = LOG_THREADS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *slot)
    };
    // 대기열이 가득 차 신호가 들어가지 못할 수 있다. 그때는 스레드가 남은
    // 줄을 다 쓰고 recv에서 멈추므로, 아래 제한 시간까지만 기다린다.
    for thread in &threads {
        let _ = thread.sender.try_send(Vec::new());
    }
    let start = std::time::Instant::now();
    const MAX_WAIT: std::time::Duration = std::time::Duration::from_millis(500);
    for thread in threads {
        while !thread.handle.is_finished() && start.elapsed() < MAX_WAIT {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if thread.handle.is_finished() {
            let _ = thread.handle.join();
        }
    }
}

/// 스레드가 밀렸을 때 대기열에 쌓아 둘 줄 수. 넘치면 **버린다** — 로그를
/// 남기겠다고 게임을 멈춰 세우는 것이 이 구조가 막으려는 바로 그 일이다.
const LOG_QUEUE_LINES: usize = 4096;

/// 한 번에 파일로 내보낼 최대 바이트. 회전 상한보다 훨씬 작게 잡아, 배치
/// 하나가 파일 상한에 걸려 잘리는 일이 없게 한다.
const LOG_BATCH_BYTES: usize = 64 * 1024;

impl AsyncLogWriter {
    fn spawn(mut writer: RollingLogWriter) -> Self {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<Vec<u8>>(LOG_QUEUE_LINES);
        let handle = std::thread::Builder::new()
            .name("anemone-log".to_string())
            .spawn(move || {
                // 빈 배치는 [`flush_and_stop`]의 중지 신호다. 로그 이벤트는
                // 언제나 한 줄 이상이라 실제 데이터와 헷갈리지 않는다.
                while let Ok(first) = receiver.recv() {
                    if first.is_empty() {
                        break;
                    }
                    let mut batch = first;
                    let mut stopping = false;
                    while batch.len() < LOG_BATCH_BYTES {
                        match receiver.try_recv() {
                            Ok(next) if next.is_empty() => {
                                stopping = true;
                                break;
                            }
                            Ok(next) => batch.extend_from_slice(&next),
                            Err(_) => break,
                        }
                    }
                    // `RollingLogWriter::write`가 회전을 처리하므로 부분 쓰기를
                    // 직접 돌린다. `write_all`은 회전 경계를 모른다.
                    let mut rest = &batch[..];
                    while !rest.is_empty() {
                        match writer.write(rest) {
                            Ok(0) | Err(_) => break,
                            Ok(written) => rest = &rest[written..],
                        }
                    }
                    let _ = writer.flush();
                    if stopping {
                        break;
                    }
                }
                let _ = writer.flush();
            })
            .expect("로그 스레드를 만들지 못했습니다");
        LOG_THREADS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(LogThread {
                sender: sender.clone(),
                handle,
            });
        Self { sender }
    }
}

impl Write for AsyncLogWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        // 대기열이 가득 차면 버린다. 여기서 블록하면 호출한 스레드가 멈추고,
        // 그것이 곧 게임이 멈추는 경로다.
        let _ = self.sender.try_send(buffer.to_vec());
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // 실제 flush는 쓰기 스레드가 배치마다 한다.
        Ok(())
    }
}

/// 후킹 디버그 이벤트의 tracing target. 이 target의 이벤트만 [`HOOK_LOG_NAME`]
/// 으로 가고, 앱 로그에는 들어가지 않는다.
pub const LUNAHOOK_TARGET: &str = "lunahook";

pub const HOOK_LOG_NAME: &str = "lunahook.log";

/// 후킹 로그는 게임 텍스트를 그대로 담아 앱 로그보다 훨씬 빨리 찬다. 디버그
/// 전용이므로 상한을 넉넉히 잡는다.
const HOOK_LOG_FILE_BYTES: u64 = 4 * 1024 * 1024;

pub fn init(hook_debug_log: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    crate::runtime::ensure_data_directories()?;
    let logs = crate::runtime::logs_dir();

    // 앱 로그는 후킹 target을 받지 않는다. 게임 원문을 남기지 않는다는 성질을
    // 후킹 로그를 켜도 그대로 유지한다.
    let app_layer = fmt::layer()
        .with_target(false)
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(AsyncLogWriter::spawn(
            RollingLogWriter::open(&logs)?,
        )))
        .with_filter(LevelFilter::from_level(configured_level()))
        .with_filter(filter_fn(|metadata| metadata.target() != LUNAHOOK_TARGET));

    let hook_layer = hook_debug_log
        .then(|| {
            RollingLogWriter::open_named(&logs, HOOK_LOG_NAME, HOOK_LOG_FILE_BYTES).map(|writer| {
                fmt::layer()
                    .with_target(false)
                    .with_ansi(false)
                    .with_writer(std::sync::Mutex::new(AsyncLogWriter::spawn(writer)))
                    // 후킹 이벤트는 전부 INFO다. 상한을 명시해야 registry의
                    // 전역 최대 레벨이 TRACE로 열리지 않는다.
                    .with_filter(LevelFilter::INFO)
                    .with_filter(filter_fn(|metadata| metadata.target() == LUNAHOOK_TARGET))
            })
        })
        .transpose()?;

    tracing_subscriber::registry()
        .with(app_layer)
        .with(hook_layer)
        .try_init()?;
    Ok(())
}

fn configured_level() -> tracing::Level {
    let default = if cfg!(debug_assertions) {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    match std::env::var("ANEMONE_LOG")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "warn" | "warning" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _ => default,
    }
}

pub fn report_init_failure(error: &dyn std::fmt::Display) {
    let message = format!("로그 초기화 실패(애플리케이션은 계속 실행): {error}");
    eprintln!("{message}");
    use windows_sys::Win32::System::Diagnostics::Debug::OutputDebugStringW;
    let wide: Vec<u16> = message.encode_utf16().chain([0]).collect();
    unsafe { OutputDebugStringW(wide.as_ptr()) };
}

#[cfg(test)]
#[path = "../tests/unit/logging.rs"]
mod tests;
