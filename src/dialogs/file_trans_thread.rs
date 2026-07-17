//! 파일 번역 백그라운드 스레드
//!
//! 파일 읽기/쓰기, 번역 처리, 진행률 업데이트.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::{
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState},
};

use super::file_trans::{FileTransJobData, WriteType, validate_job_paths};
use super::file_trans_progress::ProgressEvent;
use crate::translation::{
    TranslationEngine, get_eztrans_manager,
    http_common::shared_client,
    worker::{TranslationDispatch, TranslationRequest},
};

/// 시스템 절전 진입을 차단하는 RAII 가드.
///
/// 생성 시 `ES_CONTINUOUS | ES_SYSTEM_REQUIRED` 로 sleep 을 막고,
/// drop 시 `ES_CONTINUOUS` 로 복원해 다시 OS 기본 동작에 맡긴다.
/// `ES_DISPLAY_REQUIRED` 는 일부러 빼서 모니터 절전은 허용한다 — 사용자가
/// 자리를 비웠을 때까지 화면 켜두는 건 과한 동작이라 판단.
struct SleepBlocker;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 최종 경로와 같은 디렉터리의 임시 출력 파일.
///
/// `persist` 전까지는 Drop 시 임시 파일을 제거하므로 번역 오류와 취소가 기존
/// 결과 파일에 영향을 주지 않는다.
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

        let source: Vec<u16> = self
            .temp_path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let destination: Vec<u16> = self
            .final_path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe {
            MoveFileExW(
                windows::core::PCWSTR(source.as_ptr()),
                windows::core::PCWSTR(destination.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|error| {
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

/// 파일 번역 스레드 메인 함수
pub fn file_trans_thread(job_data: Arc<FileTransJobData>) {
    // 긴 배치 번역 중 OS 가 절전으로 진입하지 않도록 함수 전체 동안 가드 유지.
    let _sleep_guard = SleepBlocker::new();

    // 입출력 파일 수 확인
    if job_data.input_files.len() != job_data.output_files.len() {
        job_data.progress.send(ProgressEvent::Error(
            "입력 파일과 출력 파일 수가 일치하지 않습니다.".to_string(),
        ));
        return;
    }

    if let Err(message) = validate_job_paths(&job_data.input_files, &job_data.output_files) {
        job_data.progress.send(ProgressEvent::Error(message));
        return;
    }

    // EzTrans 는 최초 한 번 dll/dat 을 매니저에 적재해야 한다. 실패하면 전체 작업
    // 중단 — 라인마다 같은 에러로 실패하는 것보다 사전에 끊는 편이 친절하다.
    if job_data.engine == TranslationEngine::EzTrans {
        let manager = get_eztrans_manager();
        let init_result = match manager.lock() {
            Ok(mut mgr) => mgr.init(&job_data.eztrans_dll_path, &job_data.eztrans_dat_path),
            Err(_) => Err("EzTrans 매니저 잠금 실패".to_string()),
        };
        if let Err(e) = init_result {
            job_data
                .progress
                .send(ProgressEvent::Error(format!("EzTrans 초기화 실패: {e}")));
            return;
        }
    }

    // 비동기 HTTP 엔진 호출용 자체 tokio runtime. 디스패치 워커를 거치지 않고
    // 라인 단위로 동기적 응답이 필요하기 때문에 별도 런타임을 둔다.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            job_data
                .progress
                .send(ProgressEvent::Error(format!("tokio 런타임 생성 실패: {e}")));
            return;
        }
    };
    // 디스패치 워커와 동일한 프로세스 전역 클라이언트를 공유한다 — connection pool /
    // TLS 세션 재사용으로 라인 단위 동기 호출의 DNS·핸드셰이크 비용 제거.
    let http_client = shared_client();

    // UTF-8 검증과 파일별 줄 수 계산을 한 번의 사전 검사로 수행한다.
    let file_line_counts = match preflight_inputs(&job_data.input_files) {
        Ok(counts) => counts,
        Err(error) => {
            job_data.progress.send(ProgressEvent::Error(error));
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
            job_data.progress.send(ProgressEvent::Error(
                "입력 파일의 전체 줄 수가 너무 많습니다.".to_string(),
            ));
            return;
        }
    };

    // 전체 파일 수 및 라인 수 전송
    job_data
        .progress
        .send(ProgressEvent::TotalFiles(job_data.input_files.len() as i32));
    job_data
        .progress
        .send(ProgressEvent::TotalLines(total_lines));

    let mut global_current_line = 0;

    // 파일별 순차 처리
    for (idx, (input_path, output_path)) in job_data
        .input_files
        .iter()
        .zip(job_data.output_files.iter())
        .enumerate()
    {
        // 취소 체크
        if job_data.cancel_token.load(Ordering::SeqCst) {
            job_data
                .progress
                .send(ProgressEvent::Error("사용자가 취소했습니다.".to_string()));
            return;
        }

        // 파일 인덱스 업데이트
        job_data
            .progress
            .send(ProgressEvent::FileIndex((idx + 1) as i32));

        // 파일명 전송
        send_filename(&job_data, input_path);

        // 단일 파일 처리
        match process_single_file(
            input_path,
            output_path,
            &job_data,
            file_line_counts[idx],
            &mut global_current_line,
            &rt,
            &http_client,
        ) {
            Ok(()) => {}
            Err(e) => {
                job_data.progress.send(ProgressEvent::Error(e));
                return;
            }
        }
    }

    // 완료
    job_data.progress.send(ProgressEvent::Complete);
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
    rt: &tokio::runtime::Runtime,
    http_client: &reqwest::Client,
) -> Result<(), String> {
    // 입력 파일 열기
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

    // UTF-8 BOM 쓰기
    pending_output
        .writer()
        .write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|e| e.to_string())?;

    // 리스트 크기 전송
    job_data
        .progress
        .send(ProgressEvent::FileLines(line_count as i32));

    // 스트리밍 라인 처리. 마지막 라인 판정을 위해 1-라인 lookahead 패턴 사용 —
    // `prev` 가 직전에 읽은 라인이고, 새 라인이 도착하면 prev 를 "마지막 아님"
    // 으로 출력한다. 루프 종료 후 남은 prev 가 진짜 마지막 라인.
    let mut lines_iter = reader.lines();

    let mut prev = lines_iter.next().transpose().map_err(|e| {
        format!(
            "입력 파일을 읽을 수 없습니다: {}\n{e}",
            input_path.display()
        )
    })?;
    // 첫 줄이 UTF-8 BOM 으로 시작하면 떼어낸다. 사전 검증에서 인코딩은
    // 확인되었지만, BOM 자체는 본문에 섞이지 않도록 명시적으로 제거.
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
        // 취소 체크
        if job_data.cancel_token.load(Ordering::SeqCst) {
            return Err("사용자가 취소했습니다.".to_string());
        }

        let line = prev.take().expect("prev primed above");
        let translated = translate_line(&line, job_data, rt, http_client);
        write_output(
            pending_output.writer(),
            &line,
            &translated,
            job_data.write_type,
            false,
        )?;

        idx += 1;
        *global_current += 1;
        job_data
            .progress
            .send(ProgressEvent::FileProgress(idx as i32));
        job_data
            .progress
            .send(ProgressEvent::TotalProgress(*global_current));

        prev = Some(next);
    }

    // 남은 마지막 라인.
    if let Some(line) = prev {
        if job_data.cancel_token.load(Ordering::SeqCst) {
            return Err("사용자가 취소했습니다.".to_string());
        }
        let translated = translate_line(&line, job_data, rt, http_client);
        write_output(
            pending_output.writer(),
            &line,
            &translated,
            job_data.write_type,
            true,
        )?;

        idx += 1;
        *global_current += 1;
        job_data
            .progress
            .send(ProgressEvent::FileProgress(idx as i32));
        job_data
            .progress
            .send(ProgressEvent::TotalProgress(*global_current));
    }

    pending_output.persist()
}

/// 라인 번역.
///
/// 빈/공백 라인은 그대로 통과시킨다. 그 외 라인은 작업의 엔진 설정에 따라
/// 동기적으로 한 줄씩 호출한다. EzTrans 는 글로벌 매니저를 통해 동기 호출,
/// HTTP 기반 엔진(Google/DeepL/Papago/LLM)은 디스패치의 `translate_async`
/// (재시도/DeepL 폴백 포함) 를 자체 tokio 런타임 위에서 `block_on` 한다.
///
/// 한 줄 실패가 전체 배치를 중단시키지는 않도록, 실패 시에는 `[번역 실패: ...]`
/// 표식을 반환한다. 호출자는 이 문자열을 결과 파일에 그대로 기록한다.
fn translate_line(
    line: &str,
    job_data: &FileTransJobData,
    rt: &tokio::runtime::Runtime,
    http_client: &reqwest::Client,
) -> String {
    // 빈 라인(길이 0) 은 옵션과 무관하게 통과 — 엔진에 보낼 의미도 없고 EmptyText
    // 에러만 받는다.
    if line.is_empty() {
        return line.to_string();
    }

    // no_trans_linefeed: 공백/탭만 있는 "줄바꿈 라인" 도 통과시킨다. 영문 등 일부
    // 텍스트에서 단락 구분용으로 공백만 있는 줄이 나오는데, 그것까지 번역기에
    // 넘기면 결과가 어지러워진다.
    if job_data.no_trans_linefeed && line.trim().is_empty() {
        return line.to_string();
    }

    let request = TranslationRequest {
        id: 0,
        // Arc::from(&str) 은 buffer 한 번 alloc — 이후 워커/엔진 경로 전체에서
        // 추가 복제 없음.
        text: std::sync::Arc::from(line),
        engine: job_data.engine,
        source_lang: job_data.source_lang,
        target_lang: job_data.target_lang,
        credentials: job_data.credentials.clone(),
    };

    let result = rt.block_on(TranslationDispatch::translate_async(&request, http_client));

    match result {
        Ok(translated) => translated,
        Err(e) => {
            tracing::warn!("라인 번역 실패: {} (원문: {:.60})", e, line);
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
fn send_filename(job_data: &FileTransJobData, path: &Path) {
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    job_data.progress.send(ProgressEvent::FileName(filename));
}

#[cfg(test)]
mod tests {
    use super::{PendingOutput, count_reader_lines};
    use std::io::{BufReader, Cursor, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "anemone-file-output-test-{}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn dropped_pending_output_preserves_existing_file() {
        let directory = TestDirectory::new();
        let output = directory.0.join("result.txt");
        std::fs::write(&output, "기존 결과").unwrap();

        {
            let mut pending = PendingOutput::create(&output).unwrap();
            pending.writer().write_all(b"incomplete").unwrap();
        }

        assert_eq!(std::fs::read_to_string(output).unwrap(), "기존 결과");
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[test]
    fn persisted_output_replaces_existing_file() {
        let directory = TestDirectory::new();
        let output = directory.0.join("result.txt");
        std::fs::write(&output, "기존 결과").unwrap();

        let mut pending = PendingOutput::create(&output).unwrap();
        pending.writer().write_all("완성 결과".as_bytes()).unwrap();
        pending.persist().unwrap();

        assert_eq!(std::fs::read_to_string(output).unwrap(), "완성 결과");
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[test]
    fn line_read_error_is_returned_instead_of_skipped() {
        let reader = BufReader::new(Cursor::new(vec![0xFF, b'\n']));

        let error = count_reader_lines(reader, Path::new("invalid.txt")).unwrap_err();

        assert!(error.contains("invalid.txt"));
        assert!(error.contains("입력 파일을 읽을 수 없습니다"));
    }
}
