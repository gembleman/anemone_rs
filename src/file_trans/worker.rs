//! 파일 번역과 진행률 보고를 수행하는 백그라운드 작업.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState};

use super::{FileTransJobData, ProgressEvent, WriteType, validate_job_paths};
use crate::translation::{
    http_common::shared_client,
    worker::{TranslationDispatch, TranslationRequest},
};

/// 작업 중 시스템 절전만 막고 화면 절전은 허용하는 RAII 가드.
struct SleepBlocker;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 성공 시에만 최종 경로로 교체되는 임시 출력 파일.
struct PendingOutput {
    final_path: PathBuf,
    temp_path: PathBuf,
    writer: Option<BufWriter<File>>,
}

impl PendingOutput {
    fn create(final_path: &Path) -> Result<Self, String> {
        let parent = final_path.parent().unwrap_or(Path::new(""));
        let name = final_path.file_name().unwrap_or_default().to_string_lossy();

        for _ in 0..100 {
            let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let temp_path = parent.join(format!(
                ".{name}.anemone-{}-{sequence}.tmp",
                std::process::id()
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
            {
                Ok(file) => {
                    return Ok(Self {
                        final_path: final_path.to_path_buf(),
                        temp_path,
                        writer: Some(BufWriter::new(file)),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!(
                        "임시 출력 파일을 생성할 수 없습니다: {}\n{error}",
                        temp_path.display()
                    ));
                }
            }
        }

        Err(format!(
            "고유한 임시 출력 파일을 생성할 수 없습니다: {}",
            final_path.display()
        ))
    }

    fn writer(&mut self) -> &mut BufWriter<File> {
        self.writer.as_mut().expect("writer exists until persist")
    }

    fn persist(mut self) -> Result<(), String> {
        let mut writer = self.writer.take().expect("writer exists until persist");
        writer.flush().map_err(|error| {
            format!(
                "임시 출력 파일을 저장할 수 없습니다: {}\n{error}",
                self.temp_path.display()
            )
        })?;
        writer.get_ref().sync_all().map_err(|error| {
            format!(
                "임시 출력 파일을 디스크에 반영할 수 없습니다: {}\n{error}",
                self.temp_path.display()
            )
        })?;
        drop(writer);

        crate::fs_util::atomic_replace(&self.temp_path, &self.final_path).map_err(|error| {
            format!(
                "완성된 출력 파일을 최종 경로로 옮길 수 없습니다: {}\n{error}",
                self.final_path.display()
            )
        })?;

        self.temp_path.clear();
        Ok(())
    }
}

impl Drop for PendingOutput {
    fn drop(&mut self) {
        if !self.temp_path.as_os_str().is_empty()
            && let Err(error) = std::fs::remove_file(&self.temp_path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                "임시 출력 파일 삭제 실패 ({}): {error}",
                self.temp_path.display()
            );
        }
    }
}

impl SleepBlocker {
    fn new() -> Self {
        // SAFETY: SetThreadExecutionState 는 부수효과 없는 kernel32 호출.
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
        }
        Self
    }
}

impl Drop for SleepBlocker {
    fn drop(&mut self) {
        // SAFETY: SetThreadExecutionState 는 부수효과 없는 kernel32 호출.
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS);
        }
    }
}

/// 파일 번역 작업을 실행한다.
///
/// 진행 이벤트는 UI 종류와 무관한 호출자 콜백으로 전달한다.
pub(crate) fn run(job_data: &FileTransJobData, report: impl Fn(ProgressEvent)) {
    // 작업이 끝날 때까지 시스템 절전을 막는다.
    let _sleep_guard = SleepBlocker::new();

    // 입력과 출력은 일대일이어야 한다.
    if job_data.input_files.len() != job_data.output_files.len() {
        report(ProgressEvent::Error(
            "입력 파일과 출력 파일 수가 일치하지 않습니다.".to_string(),
        ));
        return;
    }

    if let Err(message) = validate_job_paths(&job_data.input_files, &job_data.output_files) {
        report(ProgressEvent::Error(message));
        return;
    }

    // 라인별 HTTP 응답을 동기적으로 기다릴 전용 runtime이다.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            report(ProgressEvent::Error(format!("tokio 런타임 생성 실패: {e}")));
            return;
        }
    };
    // 전역 client를 공유해 connection pool과 TLS session을 재사용한다.
    let http_client = shared_client();
    let translation = TranslationContext {
        runtime: &rt,
        http_client: &http_client,
    };

    // 한 번의 사전 검사에서 UTF-8과 줄 수를 확인한다.
    let file_line_counts = match preflight_inputs(&job_data.input_files, &job_data.cancel_token) {
        Ok(counts) => counts,
        Err(error) => {
            report(ProgressEvent::Error(error));
            return;
        }
    };
    let total_lines = match file_line_counts.iter().try_fold(0i32, |total, &count| {
        i32::try_from(count)
            .ok()
            .and_then(|count| total.checked_add(count))
    }) {
        Some(total) => total,
        None => {
            report(ProgressEvent::Error(
                "입력 파일의 전체 줄 수가 너무 많습니다.".to_string(),
            ));
            return;
        }
    };

    report(ProgressEvent::TotalFiles(job_data.input_files.len() as i32));
    report(ProgressEvent::TotalLines(total_lines));

    let mut global_current_line = 0;

    for (idx, (input_path, output_path)) in job_data
        .input_files
        .iter()
        .zip(job_data.output_files.iter())
        .enumerate()
    {
        if job_data.cancel_token.load(Ordering::SeqCst) {
            report(ProgressEvent::Error("사용자가 취소했습니다.".to_string()));
            return;
        }

        report(ProgressEvent::FileIndex((idx + 1) as i32));

        send_filename(input_path, &report);

        match process_single_file(
            input_path,
            output_path,
            job_data,
            file_line_counts[idx],
            &mut global_current_line,
            &translation,
            &report,
        ) {
            Ok(()) => {}
            Err(e) => {
                report(ProgressEvent::Error(e));
                return;
            }
        }
    }

    report(ProgressEvent::Complete);
}

struct TranslationContext<'a> {
    runtime: &'a tokio::runtime::Runtime,
    http_client: &'a reqwest::Client,
}

/// UTF-8 검증과 파일별 줄 수 계산을 결합한 사전 검사.
fn preflight_inputs(
    files: &[PathBuf],
    cancel_token: &std::sync::atomic::AtomicBool,
) -> Result<Vec<usize>, String> {
    let mut counts = Vec::with_capacity(files.len());
    for path in files {
        let reader = crate::util::open_utf8_translation_input(path)?;
        counts.push(validate_and_count_reader(reader, path, cancel_token)?);
    }
    Ok(counts)
}

fn validate_and_count_reader<R: Read>(
    mut reader: R,
    path: &Path,
    cancel_token: &std::sync::atomic::AtomicBool,
) -> Result<usize, String> {
    const CHUNK_SIZE: usize = 64 * 1024;
    let mut buffer = [0u8; CHUNK_SIZE];
    let mut pending = Vec::with_capacity(4);
    let mut total_body_bytes = 0usize;
    let mut newline_count = 0usize;
    let mut last_was_newline = false;
    let mut first_chunk = true;

    loop {
        if cancel_token.load(Ordering::SeqCst) {
            return Err("사용자가 취소했습니다.".to_string());
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("입력 파일을 읽을 수 없습니다: {}\n{e}", path.display()))?;
        if read == 0 {
            break;
        }
        let mut chunk = &buffer[..read];
        if first_chunk {
            first_chunk = false;
            chunk = chunk.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(chunk);
        }
        total_body_bytes = total_body_bytes.saturating_add(chunk.len());
        newline_count = newline_count.saturating_add(chunk.iter().filter(|&&b| b == b'\n').count());
        if let Some(&last) = chunk.last() {
            last_was_newline = last == b'\n';
        }

        pending.extend_from_slice(chunk);
        match std::str::from_utf8(&pending) {
            Ok(_) => pending.clear(),
            Err(error) if error.error_len().is_none() => {
                let tail = pending.split_off(error.valid_up_to());
                pending = tail;
            }
            Err(error) => {
                let byte = total_body_bytes
                    .saturating_sub(pending.len())
                    .saturating_add(error.valid_up_to());
                return Err(format!(
                    "UTF-8 디코딩 실패(byte {byte}): UTF-8 또는 UTF-8 BOM 파일만 사용할 수 있습니다. ({})",
                    path.display()
                ));
            }
        }
    }
    if !pending.is_empty() {
        return Err(format!(
            "UTF-8 디코딩 실패(byte {}): UTF-8 또는 UTF-8 BOM 파일만 사용할 수 있습니다. ({})",
            total_body_bytes.saturating_sub(pending.len()),
            path.display()
        ));
    }
    Ok(newline_count + usize::from(total_body_bytes > 0 && !last_was_newline))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineEnding {
    None,
    Lf,
    CrLf,
}

impl LineEnding {
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::None => b"",
            Self::Lf => b"\n",
            Self::CrLf => b"\r\n",
        }
    }

    fn separator(self) -> &'static [u8] {
        match self {
            Self::CrLf => b"\r\n",
            Self::None | Self::Lf => b"\n",
        }
    }
}

struct InputLine {
    text: String,
    ending: LineEnding,
}

fn read_input_line<R: BufRead>(
    reader: &mut R,
    path: &Path,
    first_line: bool,
) -> Result<Option<InputLine>, String> {
    const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
    let mut bytes = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .map_err(|e| format!("입력 파일을 읽을 수 없습니다: {}\n{e}", path.display()))?;
        if available.is_empty() {
            break;
        }
        let take = available
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(take) > MAX_LINE_BYTES {
            return Err(format!(
                "입력 파일의 한 줄이 허용 크기({MAX_LINE_BYTES}바이트)를 초과했습니다: {}",
                path.display()
            ));
        }
        let found_newline = available[take - 1] == b'\n';
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if found_newline {
            break;
        }
    }
    if bytes.is_empty() {
        return Ok(None);
    }

    let ending = if bytes.ends_with(b"\r\n") {
        bytes.truncate(bytes.len() - 2);
        LineEnding::CrLf
    } else if bytes.ends_with(b"\n") {
        bytes.pop();
        LineEnding::Lf
    } else {
        LineEnding::None
    };
    if first_line && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        bytes.drain(..3);
    }
    if first_line && bytes.is_empty() && ending == LineEnding::None {
        return Ok(None);
    }
    let text = String::from_utf8(bytes).map_err(|error| {
        format!(
            "UTF-8 디코딩 실패: UTF-8 또는 UTF-8 BOM 파일만 사용할 수 있습니다. ({})\n{error}",
            path.display()
        )
    })?;
    Ok(Some(InputLine { text, ending }))
}

/// 단일 파일 처리
fn process_single_file(
    input_path: &Path,
    output_path: &Path,
    job_data: &FileTransJobData,
    line_count: usize,
    global_current: &mut i32,
    translation: &TranslationContext<'_>,
    report: &impl Fn(ProgressEvent),
) -> Result<(), String> {
    let mut reader = crate::util::open_utf8_translation_input(input_path)?;

    // 최종 파일은 전체 번역과 flush가 성공한 뒤에만 교체한다.
    let mut pending_output = PendingOutput::create(output_path)?;

    // 출력은 UTF-8 BOM을 유지한다.
    pending_output
        .writer()
        .write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|e| e.to_string())?;

    report(ProgressEvent::FileLines(line_count as i32));

    let mut prev = read_input_line(&mut reader, input_path, true)?;
    let mut idx: usize = 0;

    while let Some(line) = prev.take() {
        let next = read_input_line(&mut reader, input_path, false)?;
        if job_data.cancel_token.load(Ordering::SeqCst) {
            return Err("사용자가 취소했습니다.".to_string());
        }
        let translated = translate_line(&line.text, job_data, translation)?;
        write_output(
            pending_output.writer(),
            &line.text,
            &translated,
            job_data.write_type,
            line.ending,
            next.is_some(),
        )?;

        idx += 1;
        *global_current += 1;
        report(ProgressEvent::FileProgress(idx as i32));
        report(ProgressEvent::TotalProgress(*global_current));

        prev = next;
    }

    pending_output.persist()
}

/// 라인 번역.
///
/// 한 줄을 동기 번역한다. 빈 줄은 유지하고 실패는 표식으로 바꿔 배치를 계속한다.
fn translate_line(
    line: &str,
    job_data: &FileTransJobData,
    translation: &TranslationContext<'_>,
) -> Result<String, String> {
    // 빈 줄은 엔진에 보내지 않는다.
    if line.is_empty() {
        return Ok(line.to_string());
    }

    // 옵션이 켜지면 공백만 있는 단락 구분 줄도 유지한다.
    if job_data.no_trans_linefeed && line.trim().is_empty() {
        return Ok(line.to_string());
    }

    let request = TranslationRequest {
        id: 0,
        // 한 번 할당한 원문을 워커와 엔진이 공유한다.
        text: std::sync::Arc::from(line),
        engine: job_data.engine,
        source_lang: job_data.source_lang,
        target_lang: job_data.target_lang,
        credentials: job_data.credentials.clone(),
    };

    let result = translation.runtime.block_on(async {
        tokio::select! {
            result = TranslationDispatch::translate_async(&request, translation.http_client) => Some(result),
            () = wait_for_cancellation(&job_data.cancel_token) => None,
        }
    });

    match result {
        None => Err("사용자가 취소했습니다.".to_string()),
        Some(Ok(translated)) => Ok(translated),
        Some(Err(e)) => {
            tracing::warn!(
                category = e.log_category(),
                status_code = ?e.log_status_code(),
                input_bytes = line.len(),
                "file translation line failed"
            );
            Ok(format!("[번역 실패: {}]", e))
        }
    }
}

async fn wait_for_cancellation(token: &std::sync::atomic::AtomicBool) {
    while !token.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// 출력 형식에 따라 쓰기
fn write_output<W: Write>(
    writer: &mut W,
    original: &str,
    translated: &str,
    write_type: WriteType,
    ending: LineEnding,
    has_more: bool,
) -> Result<(), String> {
    let separator = ending.separator();
    match write_type {
        WriteType::TranslationOnly => {
            writer
                .write_all(translated.as_bytes())
                .map_err(|e| e.to_string())?;
            writer
                .write_all(ending.bytes())
                .map_err(|e| e.to_string())?;
        }
        WriteType::OriginalAndTrans => {
            writer
                .write_all(original.as_bytes())
                .map_err(|e| e.to_string())?;
            writer.write_all(separator).map_err(|e| e.to_string())?;
            writer
                .write_all(translated.as_bytes())
                .map_err(|e| e.to_string())?;
            writer
                .write_all(ending.bytes())
                .map_err(|e| e.to_string())?;
        }
        WriteType::OriginalTransNewline => {
            writer
                .write_all(original.as_bytes())
                .map_err(|e| e.to_string())?;
            writer.write_all(separator).map_err(|e| e.to_string())?;
            writer
                .write_all(translated.as_bytes())
                .map_err(|e| e.to_string())?;
            if has_more {
                writer.write_all(separator).map_err(|e| e.to_string())?;
                writer.write_all(separator).map_err(|e| e.to_string())?;
            } else {
                writer
                    .write_all(ending.bytes())
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(())
}

/// 파일명 전송
fn send_filename(path: &Path, report: &impl Fn(ProgressEvent)) {
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    report(ProgressEvent::FileName(filename));
}

#[cfg(test)]
#[path = "../../tests/unit/file_trans/worker.rs"]
mod tests;
