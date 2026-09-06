//! 파일 하나를 배치 단위로 읽고, 번역하고, 쓰는 흐름.
//!
//! [`process_single_file`]은 읽기(`collect_batch`) → 번역(`translate_batch`) →
//! 쓰기/진행 보고(`write_batch`) 세 단계로 나뉜다.

use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::atomic::Ordering;

use super::translate::translate_lines;
use super::{FILE_HTTP_BATCH_LINES, FileJob, FileRuntime, PROGRESS_REPORT_INTERVAL};
use crate::file_trans::eztrans::{
    EZTRANS_WINDOW_MAX_CHARS, EZTRANS_WINDOW_MAX_LINES, translate_eztrans_window,
};
use crate::file_trans::input::{InputLine, open_utf8_translation_input, read_input_line};
use crate::file_trans::output::{PendingOutput, write_output};
use crate::file_trans::{FileTranslationError, FileTranslationProgress, FileTranslationRequest};

/// 파일 하나를 처음부터 끝까지 배치 단위로 처리한다.
pub(super) fn process_single_file(
    file: FileJob<'_>,
    global_current: &mut i32,
    runtime: &mut FileRuntime<'_>,
    report: &impl Fn(FileTranslationProgress),
) -> Result<(), FileTranslationError> {
    let mut reader =
        open_utf8_translation_input(file.input_path).map_err(FileTranslationError::input)?;
    let mut pending_output = PendingOutput::create(file.output_path)?;
    pending_output
        .write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|error| FileTranslationError::output(error.to_string()))?;

    report(FileTranslationProgress::FileLines(file.line_count as i32));
    let mut next_line = read_input_line(&mut reader, file.input_path, true)?;
    let mut line_index = 0usize;

    while let Some(line) = next_line.take() {
        let (lines, remaining) = collect_batch(&mut reader, file.input_path, file.job_data, line)?;
        next_line = remaining;

        // 배치 하나에 여러 줄이 모여도(non-blocking 배치, EzTrans 창) 취소
        // 응답성은 줄 단위를 유지해야 한다. 배치 시작 전에도 한 번 검사해
        // 이미 취소된 상태면 번역 요청 자체를 보내지 않는다.
        if file.job_data.cancel_token.load(Ordering::SeqCst) {
            return Err(FileTranslationError::Cancelled);
        }
        let translated_lines = translate_batch(&lines, file.job_data, runtime)?;

        write_batch(
            &mut pending_output,
            BatchWrite {
                lines,
                translated_lines,
                has_more_after: next_line.is_some(),
                line_count: file.line_count,
                job_data: file.job_data,
            },
            &mut line_index,
            global_current,
            report,
        )?;
    }

    pending_output.persist()
}

/// 엔진 종류(blocking/non-blocking)에 맞춰 한 배치를 모은다.
///
/// non-blocking 엔진은 EzTrans 같은 서버 창(window) 개념이 없어 separator/window
/// char 카운트는 필요 없다. 다만 JoinSet 동시성이 실제로 발휘되려면 여러 줄을
/// 모아 한 번에 `translate_lines`로 넘겨야 하므로, `FILE_HTTP_BATCH_LINES`에
/// 도달하거나 파일 끝에 도달할 때까지 계속 모은다.
fn collect_batch<R: BufRead>(
    reader: &mut R,
    input_path: &Path,
    job_data: &FileTranslationRequest,
    first_line: InputLine,
) -> Result<(Vec<InputLine>, Option<InputLine>), FileTranslationError> {
    if job_data.translation.engine().is_blocking() {
        collect_eztrans_window(reader, input_path, first_line)
    } else {
        collect_http_batch(reader, input_path, first_line)
    }
}

fn collect_http_batch<R: BufRead>(
    reader: &mut R,
    input_path: &Path,
    mut line: InputLine,
) -> Result<(Vec<InputLine>, Option<InputLine>), FileTranslationError> {
    let mut lines = Vec::new();
    loop {
        lines.push(line);
        let next = read_input_line(reader, input_path, false)?;
        if lines.len() >= FILE_HTTP_BATCH_LINES {
            return Ok((lines, next));
        }
        match next {
            Some(next_line) => line = next_line,
            None => return Ok((lines, None)),
        }
    }
}

/// EzTrans 서버 창(window) 제한(줄 수/문자 수)에 맞춰 배치를 모은다.
fn collect_eztrans_window<R: BufRead>(
    reader: &mut R,
    input_path: &Path,
    mut line: InputLine,
) -> Result<(Vec<InputLine>, Option<InputLine>), FileTranslationError> {
    let mut lines = Vec::new();
    let mut window_chars = 0usize;
    loop {
        let separator_chars = usize::from(!lines.is_empty());
        let line_chars = line.text.chars().count();
        let exceeds_window = !lines.is_empty()
            && (lines.len() >= EZTRANS_WINDOW_MAX_LINES
                || window_chars
                    .saturating_add(separator_chars)
                    .saturating_add(line_chars)
                    > EZTRANS_WINDOW_MAX_CHARS);
        if exceeds_window {
            return Ok((lines, Some(line)));
        }

        window_chars = window_chars
            .saturating_add(separator_chars)
            .saturating_add(line_chars);
        lines.push(line);
        match read_input_line(reader, input_path, false)? {
            Some(next_line) => line = next_line,
            None => return Ok((lines, None)),
        }
    }
}

/// 배치를 엔진에 맞춰 번역한다(EzTrans 창 번역 vs non-blocking 제한 동시성).
fn translate_batch(
    lines: &[InputLine],
    job_data: &FileTranslationRequest,
    runtime: &mut FileRuntime<'_>,
) -> Result<Vec<String>, FileTranslationError> {
    if job_data.translation.engine().is_blocking() {
        let pool = runtime.eztrans_pool.ok_or_else(|| {
            FileTranslationError::backend("EzTrans 파일 번역 helper 풀이 준비되지 않았습니다")
        })?;
        translate_eztrans_window(lines, job_data, pool, &mut runtime.eztrans_cache)
    } else {
        translate_lines(lines, job_data, runtime.translation)
    }
}

/// [`write_batch`]에 필요한 배치 결과와 부가 정보를 묶은 구조체.
struct BatchWrite<'a> {
    lines: Vec<InputLine>,
    translated_lines: Vec<String>,
    /// 이 배치 뒤에도 더 읽을 줄이 있는지. `WriteType::OriginalTransNewline`의
    /// 줄 사이 빈 줄 삽입 여부를 결정한다.
    has_more_after: bool,
    line_count: usize,
    job_data: &'a FileTranslationRequest,
}

/// 번역된 배치를 출력에 쓰고 진행 상황을 보고한다. 취소 응답성을 위해 줄 단위로
/// 취소 상태를 확인한다 — 배치가 커질수록 취소가 늦게 반영되고 이미 번역된
/// 나머지 줄까지 써버린 뒤 persist까지 끝나버릴 수 있기 때문이다. 여기서 즉시
/// 끊어 남은 줄을 쓰지 않고 pending_output을 persist하지 않은 채 반환한다.
fn write_batch(
    pending_output: &mut PendingOutput,
    batch: BatchWrite<'_>,
    line_index: &mut usize,
    global_current: &mut i32,
    report: &impl Fn(FileTranslationProgress),
) -> Result<(), FileTranslationError> {
    let batch_len = batch.lines.len();
    for (batch_index, (line, translated)) in batch
        .lines
        .into_iter()
        .zip(batch.translated_lines)
        .enumerate()
    {
        write_output(
            pending_output,
            &line.text,
            &translated,
            batch.job_data.write_type,
            line.ending,
            batch_index + 1 < batch_len || batch.has_more_after,
        )?;

        *line_index += 1;
        *global_current += 1;
        if *line_index == 1
            || *line_index == batch.line_count
            || line_index.is_multiple_of(PROGRESS_REPORT_INTERVAL)
        {
            report(FileTranslationProgress::FileProgress(*line_index as i32));
            report(FileTranslationProgress::TotalProgress(*global_current));
        }

        if batch.job_data.cancel_token.load(Ordering::SeqCst) {
            return Err(FileTranslationError::Cancelled);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/file_trans/batch.rs"]
mod tests;
