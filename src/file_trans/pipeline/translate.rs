//! non-blocking(HTTP) 엔진의 제한 동시성 줄 번역.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::TranslationContext;
use crate::file_trans::eztrans::should_translate_line;
use crate::file_trans::input::InputLine;
use crate::file_trans::{FileTranslationError, FileTranslationRequest};
use crate::translation::PreparedJob;
use crate::translation::worker::{TranslationDispatch, TranslationRequest};

/// non-blocking 엔진 줄 번역을 제한된 동시성으로 실행하고 결과를 입력 순서로 재조립한다.
///
/// 1줄 = 1 HTTP 요청인 구조는 그대로지만, 줄 단위 완주 대기(`block_on`)를 없애고
/// 최대 `FILE_HTTP_CONCURRENCY`개의 요청을 동시에 in-flight로 둔다. current_thread
/// 런타임이어도 I/O await 지점에서 인터리빙되므로 줄당 왕복 대신 배치 단위로
/// 지연이 겹친다. 취소 시 JoinSet을 drop해 in-flight 요청을 함께 취소한다.
pub(super) fn translate_lines(
    lines: &[InputLine],
    job_data: &FileTranslationRequest,
    translation: &TranslationContext<'_>,
) -> Result<Vec<String>, FileTranslationError> {
    if lines.is_empty() {
        return Ok(Vec::new());
    }
    let originals: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
    translation.runtime.block_on(translate_lines_async(
        &originals,
        job_data,
        translation.http_client,
        Arc::clone(&translation.http_concurrency),
    ))
}

async fn translate_lines_async(
    lines: &[&str],
    job_data: &FileTranslationRequest,
    client: &reqwest::Client,
    concurrency: std::sync::Arc<tokio::sync::Semaphore>,
) -> Result<Vec<String>, FileTranslationError> {
    let mut tasks = tokio::task::JoinSet::new();
    // JoinSet::join_next()의 Err(JoinError)는 어떤 태스크가 panic했는지 값으로
    // 알려주지 않는다. spawn 시점에 발급되는 task::Id를 index로 되짚을 수 있게
    // 맵으로 남겨 둔다 — panic한 줄을 조용히 원문으로 흘리지 않기 위해서다.
    let mut task_indices: HashMap<tokio::task::Id, usize> = HashMap::new();
    for (index, &line) in lines.iter().enumerate() {
        if !should_translate_line(line, job_data.no_trans_linefeed) {
            continue;
        }
        let task_client = client.clone();
        let task_job = job_data.translation.clone();
        let task_cancel = Arc::clone(&job_data.cancel_token);
        let task_concurrency = Arc::clone(&concurrency);
        let text: Arc<str> = Arc::from(line);
        let abort_handle = tasks.spawn(async move {
            (
                index,
                translate_line_task(text, task_job, task_client, task_cancel, task_concurrency)
                    .await,
            )
        });
        task_indices.insert(abort_handle.id(), index);
    }

    let mut results: Vec<Option<String>> = vec![None; lines.len()];
    let outcome: Result<(), FileTranslationError> = tokio::select! {
        _ = wait_for_cancellation(&job_data.cancel_token) => Err(FileTranslationError::Cancelled),
        _ = async {
            while let Some(joined) = tasks.join_next_with_id().await {
                match joined {
                    Ok((_, (index, translated))) => results[index] = Some(translated),
                    Err(join_error) => {
                        tracing::warn!("file translation task panicked: {join_error}");
                        // should_translate_line이 false라 애초에 spawn하지 않은 줄은
                        // task_indices에 없으므로 여기서 건드리지 않는다 — 그 경우
                        // results[index]가 None으로 남아 원문 유지되는 것이 정상이다.
                        // 반면 panic한 태스크는 반드시 실패 표식을 남겨야 한다: None을
                        // 그대로 두면 최종 조립에서 원문으로 대체돼 "번역 실패"가 아니라
                        // "번역 안 된 원문"이 조용히 출력된다.
                        if let Some(&index) = task_indices.get(&join_error.id()) {
                            results[index] = Some(format!("[번역 실패: 작업 패닉: {join_error}]"));
                        }
                    }
                }
            }
        } => Ok(()),
    };
    outcome?;

    // 드레인 브랜치가 wait_for_cancellation(20ms 폴링)보다 먼저 끝나면 취소가
    // 진행 중이어도 위 select 전체가 Ok(())로 빠질 수 있다. 그 사이 개별 태스크가
    // 취소를 감지해 남긴 "[번역 실패: 취소됨]" 결과가 정상 번역인 것처럼
    // write_output까지 흘러가지 않도록, 드레인이 끝난 뒤에도 취소 상태를 다시
    // 확인해 취소면 결과를 버리고 명시적으로 실패시킨다.
    if job_data.cancel_token.load(Ordering::SeqCst) {
        return Err(FileTranslationError::Cancelled);
    }

    Ok(results
        .into_iter()
        .zip(lines)
        .map(|(result, &original)| match result {
            Some(translated) => translated,
            None => original.to_string(),
        })
        .collect())
}

/// 한 줄을 제한 동시성으로 번역한다. 빈 줄/줄바꿈 전용 줄은 유지하고
/// HTTP 실패는 표식으로 바꿔 계속한다. Err 반환은 없어 JoinSet 드레인이
/// 항상 완주한다 — 취소는 바깥 select의 JoinSet drop으로 처리된다.
async fn translate_line_task(
    text: Arc<str>,
    job: PreparedJob,
    client: reqwest::Client,
    cancel_token: Arc<std::sync::atomic::AtomicBool>,
    concurrency: Arc<tokio::sync::Semaphore>,
) -> String {
    let Ok(_permit) = concurrency.acquire_owned().await else {
        return "[번역 실패: 동시성 게이트가 닫혔습니다]".to_string();
    };
    if cancel_token.load(Ordering::SeqCst) {
        return "[번역 실패: 취소됨]".to_string();
    }
    let request = TranslationRequest { id: 0, text, job };
    match TranslationDispatch::translate_async(&request, &client).await {
        Ok(translated) => translated,
        Err(error) => {
            tracing::warn!(
                category = error.log_category(),
                status_code = ?error.log_status_code(),
                "file translation line failed"
            );
            format!("[번역 실패: {error}]")
        }
    }
}

/// 한 줄을 동기 번역한다. 빈 줄은 유지하고 HTTP 실패는 표식으로 바꿔 계속한다.
/// 실제 파이프라인은 `translate_lines`(제한 동시성)를 쓰고, 이 함수는 단일 줄
/// 취소 동작을 검증하는 테스트 전용으로 남긴다.
#[cfg(test)]
pub fn translate_line(
    line: &str,
    job_data: &FileTranslationRequest,
    translation: &TranslationContext<'_>,
) -> Result<String, FileTranslationError> {
    if !should_translate_line(line, job_data.no_trans_linefeed) {
        return Ok(line.to_string());
    }

    let request = TranslationRequest {
        id: 0,
        text: std::sync::Arc::from(line),
        job: job_data.translation.clone(),
    };

    let result = translation.runtime.block_on(async {
        tokio::select! {
            result = TranslationDispatch::translate_async(&request, translation.http_client) => Some(result),
            () = wait_for_cancellation(&job_data.cancel_token) => None,
        }
    });

    match result {
        None => Err(FileTranslationError::Cancelled),
        Some(Ok(translated)) => Ok(translated),
        Some(Err(error)) => {
            tracing::warn!(
                category = error.log_category(),
                status_code = ?error.log_status_code(),
                input_bytes = line.len(),
                "file translation line failed"
            );
            Ok(format!("[번역 실패: {error}]"))
        }
    }
}

async fn wait_for_cancellation(token: &std::sync::atomic::AtomicBool) {
    while !token.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
