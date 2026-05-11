//! 파일 번역 백그라운드 스레드
//!
//! 파일 읽기/쓰기, 번역 처리, 진행률 업데이트.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState},
    UI::WindowsAndMessaging::PostMessageW,
};

use crate::constants::{
    WM_PROGRESS_COMPLETE, WM_PROGRESS_CURRENT, WM_PROGRESS_ERROR, WM_PROGRESS_INDEX,
    WM_PROGRESS_LIST_SIZE, WM_PROGRESS_NAME, WM_PROGRESS_TOTAL_COUNT, WM_PROGRESS_TOTAL_SIZE,
    WM_PROGRESS_UPDATE,
};
use crate::util::to_wide;
use super::file_trans::{FileTransJobData, WriteType};

/// 시스템 절전 진입을 차단하는 RAII 가드.
///
/// 생성 시 `ES_CONTINUOUS | ES_SYSTEM_REQUIRED` 로 sleep 을 막고,
/// drop 시 `ES_CONTINUOUS` 로 복원해 다시 OS 기본 동작에 맡긴다.
/// `ES_DISPLAY_REQUIRED` 는 일부러 빼서 모니터 절전은 허용한다 — 사용자가
/// 자리를 비웠을 때까지 화면 켜두는 건 과한 동작이라 판단.
struct SleepBlocker;

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

    // isize를 HWND로 변환
    let progress_hwnd = HWND(job_data.progress_hwnd as *mut std::ffi::c_void);

    // 입출력 파일 수 확인
    if job_data.input_files.len() != job_data.output_files.len() {
        send_error(
            progress_hwnd,
            "입력 파일과 출력 파일 수가 일치하지 않습니다.",
        );
        return;
    }

    // 전체 라인 수 계산
    let total_lines = calculate_total_lines(&job_data.input_files);
    if total_lines < 0 {
        send_error(progress_hwnd, "파일을 읽을 수 없습니다.");
        return;
    }

    // 전체 파일 수 및 라인 수 전송
    send_progress_message(
        progress_hwnd,
        WM_PROGRESS_TOTAL_COUNT,
        0,
        job_data.input_files.len() as isize,
    );
    send_progress_message(
        progress_hwnd,
        WM_PROGRESS_TOTAL_SIZE,
        0,
        total_lines as isize,
    );

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
            send_error(progress_hwnd, "사용자가 취소했습니다.");
            return;
        }

        // 파일 인덱스 업데이트
        send_progress_message(progress_hwnd, WM_PROGRESS_INDEX, 0, (idx + 1) as isize);

        // 파일명 전송
        send_filename(progress_hwnd, input_path);

        // 단일 파일 처리
        match process_single_file(
            input_path,
            output_path,
            &job_data,
            progress_hwnd,
            &mut global_current_line,
        ) {
            Ok(()) => {}
            Err(e) => {
                send_error(progress_hwnd, &e);
                return;
            }
        }
    }

    // 완료
    send_progress_message(progress_hwnd, WM_PROGRESS_COMPLETE, 0, 0);
}

/// 전체 라인 수 계산
fn calculate_total_lines(files: &[PathBuf]) -> i32 {
    let mut total = 0i32;

    for path in files {
        match File::open(path) {
            Ok(file) => {
                let reader = BufReader::new(file);
                total += reader.lines().count() as i32;
            }
            Err(e) => {
                tracing::error!("Failed to open file {}: {e}", path.display());
                return -1;
            }
        }
    }

    total
}

/// 단일 파일 처리
fn process_single_file(
    input_path: &Path,
    output_path: &Path,
    job_data: &FileTransJobData,
    progress_hwnd: HWND,
    global_current: &mut i32,
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

    // 출력 파일 생성
    let output_file = File::create(output_path).map_err(|e| {
        format!(
            "출력 파일을 생성할 수 없습니다: {}\n{}",
            output_path.display(),
            e
        )
    })?;
    let mut writer = BufWriter::new(output_file);

    // UTF-8 BOM 쓰기
    writer
        .write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|e| e.to_string())?;

    // 라인 읽기
    let lines: Vec<String> = reader.lines().filter_map(|l| {
        l.inspect_err(|e| tracing::warn!("Failed to read line: {e}")).ok()
    }).collect();

    let line_count = lines.len();

    // 리스트 크기 전송
    send_progress_message(progress_hwnd, WM_PROGRESS_LIST_SIZE, 0, line_count as isize);

    // 라인별 처리
    for (i, line) in lines.iter().enumerate() {
        // 취소 체크
        if job_data.cancel_token.load(Ordering::SeqCst) {
            return Err("사용자가 취소했습니다.".to_string());
        }

        // 번역 처리
        let translated = translate_line(line, job_data.no_trans_linefeed);

        // 출력 형식에 따라 쓰기
        write_output(
            &mut writer,
            line,
            &translated,
            job_data.write_type,
            i == line_count - 1,
        )?;

        // 진행률 업데이트
        *global_current += 1;
        send_progress_message(progress_hwnd, WM_PROGRESS_UPDATE, i + 1, 0);
        send_progress_message(
            progress_hwnd,
            WM_PROGRESS_CURRENT,
            0,
            *global_current as isize,
        );
    }

    // 버퍼 플러시
    writer.flush().map_err(|e| e.to_string())?;

    Ok(())
}

/// 라인 번역 (현재는 플레이스홀더 - 실제 번역 엔진 연동 필요)
fn translate_line(line: &str, no_trans_linefeed: bool) -> String {
    // 줄바꿈만 있는 라인 처리
    if no_trans_linefeed && line.trim().is_empty() {
        return line.to_string();
    }

    // TODO: 실제 번역 엔진 연동
    // 현재는 원문 앞에 [번역] 태그 추가
    if line.trim().is_empty() {
        line.to_string()
    } else {
        format!("[번역] {}", line)
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

/// 진행률 메시지 전송
fn send_progress_message(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) {
    // SAFETY: hwnd was reconstructed from a valid isize stored in FileTransJobData.
    // PostMessageW is safe to call from any thread.
    unsafe {
        let _ = PostMessageW(Some(hwnd), msg, WPARAM(wparam), LPARAM(lparam));
    }
}

/// 파일명 전송
fn send_filename(hwnd: HWND, path: &Path) {
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    post_wide_string(hwnd, WM_PROGRESS_NAME, &filename);
}

/// 에러 메시지 전송
fn send_error(hwnd: HWND, message: &str) {
    post_wide_string(hwnd, WM_PROGRESS_ERROR, message);
}

/// UTF-16 문자열을 힙에 박스로 담아 LPARAM 로 PostMessage 한다.
///
/// 수신측은 `lparam` 을 `*mut Vec<u16>` 으로 받아 `Box::from_raw` 로 회수해
/// 자동 free 한다. 이전 구현은 `static mut` 버퍼를 공유했지만, PostMessage 가
/// 비동기 큐잉이라 수신측이 처리하기 전에 송신측이 같은 버퍼를 덮어쓰는
/// 데이터 레이스가 있었다 (연속 파일 처리 시 파일명 메시지가 섞일 수 있음).
fn post_wide_string(hwnd: HWND, msg: u32, s: &str) {
    let boxed: Box<Vec<u16>> = Box::new(to_wide(s));
    let raw = Box::into_raw(boxed);

    // SAFETY: raw points to a leaked Vec<u16> owned by us; receiver reclaims via Box::from_raw.
    // PostMessageW only queues; ownership transfer is atomic at message-queue boundary.
    let result = unsafe { PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(raw as isize)) };
    if result.is_err() {
        // PostMessage 실패 — 수신자가 박스를 회수하지 못하므로 직접 회수해서 누수 방지.
        // SAFETY: raw is still valid; we just leaked it via into_raw above.
        let _ = unsafe { Box::from_raw(raw) };
        tracing::warn!("PostMessageW failed for msg {msg:#x}");
    }
}
