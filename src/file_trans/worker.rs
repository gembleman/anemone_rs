//! 파일 번역과 진행률 보고를 수행하는 백그라운드 작업.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
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
    let file_line_counts = match preflight_inputs(&job_data.input_files) {
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
fn preflight_inputs(files: &[PathBuf]) -> Result<Vec<usize>, String> {
    let mut counts = Vec::with_capacity(files.len());
    for path in files {
        let body = crate::util::read_utf8_translation_input(path)?;
        counts.push(count_reader_lines(
            BufReader::new(std::io::Cursor::new(body)),
            path,
        )?);
    }
    Ok(counts)
}

fn count_reader_lines<R: BufRead>(reader: R, path: &Path) -> Result<usize, String> {
    reader.lines().try_fold(0usize, |count, line| {
        line.map(|_| count + 1)
            .map_err(|e| format!("입력 파일을 읽을 수 없습니다: {}\n{e}", path.display()))
    })
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
    let input_file = File::open(input_path).map_err(|e| {
        format!(
            "입력 파일을 열 수 없습니다: {}\n{}",
            input_path.display(),
            e
        )
    })?;
    let reader = BufReader::new(input_file);

    // 최종 파일은 전체 번역과 flush가 성공한 뒤에만 교체한다.
    let mut pending_output = PendingOutput::create(output_path)?;

    // 출력은 UTF-8 BOM을 유지한다.
    pending_output
        .writer()
        .write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|e| e.to_string())?;

    report(ProgressEvent::FileLines(line_count as i32));

    // 한 줄 lookahead로 마지막 줄의 개행 여부를 보존한다.
    let mut lines_iter = reader.lines();

    let mut prev = lines_iter.next().transpose().map_err(|e| {
        format!(
            "입력 파일을 읽을 수 없습니다: {}\n{e}",
            input_path.display()
        )
    })?;
    // 첫 줄의 UTF-8 BOM은 본문에서 제외한다.
    if let Some(first) = prev.as_mut()
        && let Some(stripped) = first.strip_prefix('\u{FEFF}')
    {
        *first = stripped.to_string();
    }
    let mut idx: usize = 0;

    for next in lines_iter {
        let next = next.map_err(|e| {
            format!(
                "입력 파일을 읽을 수 없습니다: {}\n{e}",
                input_path.display()
            )
        })?;
        if job_data.cancel_token.load(Ordering::SeqCst) {
            return Err("사용자가 취소했습니다.".to_string());
        }

        let line = prev.take().expect("prev primed above");
        let translated = translate_line(&line, job_data, translation);
        write_output(
            pending_output.writer(),
            &line,
            &translated,
            job_data.write_type,
            false,
        )?;

        idx += 1;
        *global_current += 1;
        report(ProgressEvent::FileProgress(idx as i32));
        report(ProgressEvent::TotalProgress(*global_current));

        prev = Some(next);
    }

    // 남은 마지막 라인.
    if let Some(line) = prev {
        if job_data.cancel_token.load(Ordering::SeqCst) {
            return Err("사용자가 취소했습니다.".to_string());
        }
        let translated = translate_line(&line, job_data, translation);
        write_output(
            pending_output.writer(),
            &line,
            &translated,
            job_data.write_type,
            true,
        )?;

        idx += 1;
        *global_current += 1;
        report(ProgressEvent::FileProgress(idx as i32));
        report(ProgressEvent::TotalProgress(*global_current));
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
) -> String {
    // 빈 줄은 엔진에 보내지 않는다.
    if line.is_empty() {
        return line.to_string();
    }

    // 옵션이 켜지면 공백만 있는 단락 구분 줄도 유지한다.
    if job_data.no_trans_linefeed && line.trim().is_empty() {
        return line.to_string();
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

    let result = translation
        .runtime
        .block_on(TranslationDispatch::translate_async(
            &request,
            translation.http_client,
        ));

    match result {
        Ok(translated) => translated,
        Err(e) => {
            tracing::warn!(
                category = e.log_category(),
                status_code = ?e.log_status_code(),
                input_bytes = line.len(),
                "file translation line failed"
            );
            format!("[번역 실패: {}]", e)
        }
    }
}

/// 출력 형식에 따라 쓰기
fn write_output(
    writer: &mut BufWriter<File>,
    original: &str,
    translated: &str,
    write_type: WriteType,
    is_last: bool,
) -> Result<(), String> {
    match write_type {
        WriteType::TranslationOnly => {
            // 번역만
            writeln!(writer, "{}", translated).map_err(|e| e.to_string())?;
        }
        WriteType::OriginalAndTrans => {
            // 원문 + 번역
            writeln!(writer, "{}", original).map_err(|e| e.to_string())?;
            if is_last {
                write!(writer, "{}", translated).map_err(|e| e.to_string())?;
            } else {
                writeln!(writer, "{}", translated).map_err(|e| e.to_string())?;
            }
        }
        WriteType::OriginalTransNewline => {
            // 원문 + 번역 + 개행
            writeln!(writer, "{}", original).map_err(|e| e.to_string())?;
            writeln!(writer, "{}", translated).map_err(|e| e.to_string())?;
            if !is_last {
                writeln!(writer).map_err(|e| e.to_string())?;
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
