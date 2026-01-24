//! 파일 번역 백그라운드 스레드
//!
//! 파일 읽기/쓰기, 번역 처리, 진행률 업데이트.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

use super::file_trans::{FileTransJobData, WriteType};
use super::file_trans_progress::*;

/// 파일 번역 스레드 메인 함수
pub fn file_trans_thread(job_data: Arc<FileTransJobData>) {
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
            Err(_) => return -1,
        }
    }

    total
}

/// 단일 파일 처리
fn process_single_file(
    input_path: &PathBuf,
    output_path: &PathBuf,
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
    let lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();

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
        send_progress_message(progress_hwnd, WM_PROGRESS_UPDATE, (i + 1) as usize, 0);
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
    unsafe {
        let _ = PostMessageW(Some(hwnd), msg, WPARAM(wparam), LPARAM(lparam));
    }
}

/// 파일명 전송
fn send_filename(hwnd: HWND, path: &PathBuf) {
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let wide: Vec<u16> = filename.encode_utf16().chain(std::iter::once(0)).collect();

    // 정적 버퍼에 복사 (PostMessage 후에도 유효하도록)
    static mut FILENAME_BUFFER: [u16; 260] = [0; 260];
    unsafe {
        let buffer_ptr = std::ptr::addr_of_mut!(FILENAME_BUFFER);
        let copy_len = wide.len().min(259);
        (&mut (*buffer_ptr))[..copy_len].copy_from_slice(&wide[..copy_len]);
        (&mut (*buffer_ptr))[copy_len] = 0;

        let _ = PostMessageW(
            Some(hwnd),
            WM_PROGRESS_NAME,
            WPARAM(0),
            LPARAM((&(*buffer_ptr)).as_ptr() as isize),
        );
    }
}

/// 에러 메시지 전송
fn send_error(hwnd: HWND, message: &str) {
    let wide: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();

    // 정적 버퍼에 복사
    static mut ERROR_BUFFER: [u16; 512] = [0; 512];
    unsafe {
        let buffer_ptr = std::ptr::addr_of_mut!(ERROR_BUFFER);
        let copy_len = wide.len().min(511);
        (&mut (*buffer_ptr))[..copy_len].copy_from_slice(&wide[..copy_len]);
        (&mut (*buffer_ptr))[copy_len] = 0;

        let _ = PostMessageW(
            Some(hwnd),
            WM_PROGRESS_ERROR,
            WPARAM(0),
            LPARAM((&(*buffer_ptr)).as_ptr() as isize),
        );
    }
}
