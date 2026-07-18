use std::collections::HashMap;
use std::sync::Arc;

use quick_cache::unsync::Cache;

use super::input::InputLine;
use super::{FileTransJobData, FileTranslationError};
use crate::translation::EzTransBatchTranslator;

const EZTRANS_BATCH_MAX_LINES: usize = 256;
const EZTRANS_BATCH_MAX_CHARS: usize = 96 * 1024;
pub(super) const EZTRANS_WINDOW_MAX_LINES: usize = 50_000;
pub(super) const EZTRANS_WINDOW_MAX_CHARS: usize = 16 * 1024 * 1024;
pub(super) const EZTRANS_CACHE_MAX_ENTRIES: usize = 100_000;

/// 파일 작업 동안만 유지되는 bounded scan-resistant 번역 캐시.
/// 설정/사전이 다른 다음 작업으로는 넘어가지 않는다.
pub struct BoundedTranslationCache {
    entries: Cache<Arc<str>, Arc<str>>,
}

impl BoundedTranslationCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Cache::new(capacity),
        }
    }

    fn get(&self, original: &str) -> Option<&str> {
        self.entries.get(original).map(AsRef::as_ref)
    }

    fn insert(&mut self, original: Arc<str>, translated: Arc<str>) {
        self.entries.insert(original, translated);
    }
}

pub fn translate_eztrans_window(
    lines: &[InputLine],
    job_data: &FileTransJobData,
    translator: &dyn EzTransBatchTranslator,
    cache: &mut BoundedTranslationCache,
) -> Result<Vec<String>, FileTranslationError> {
    let mut results = lines
        .iter()
        .map(|line| line.text.clone())
        .collect::<Vec<_>>();
    let mut miss_by_text: HashMap<Arc<str>, usize> = HashMap::new();
    let mut misses = Vec::<Arc<str>>::new();
    let mut line_misses = vec![None; lines.len()];

    for (index, line) in lines.iter().enumerate() {
        if !should_translate_line(&line.text, job_data.no_trans_linefeed) {
            continue;
        }
        if let Some(translated) = cache.get(&line.text) {
            results[index] = translated.to_string();
            continue;
        }
        if let Some(&miss_index) = miss_by_text.get(line.text.as_str()) {
            line_misses[index] = Some(miss_index);
            continue;
        }
        let original: Arc<str> = Arc::from(line.text.as_str());
        let miss_index = misses.len();
        miss_by_text.insert(original.clone(), miss_index);
        misses.push(original);
        line_misses[index] = Some(miss_index);
    }

    if misses.is_empty() {
        return Ok(results);
    }
    let batches = partition_eztrans_batches(&misses, translator.process_count());
    let translated_batches = translator
        .translate_batches(batches, &job_data.cancel_token)
        .map_err(FileTranslationError::backend)?;
    let translated_misses = translated_batches.into_iter().flatten().collect::<Vec<_>>();
    if translated_misses.len() != misses.len() {
        return Err(FileTranslationError::backend(format!(
            "EzTrans helper 결과 수가 일치하지 않습니다: 요청 {}, 응답 {}",
            misses.len(),
            translated_misses.len()
        )));
    }

    for (original, translated) in misses.iter().cloned().zip(&translated_misses) {
        cache.insert(original, Arc::from(translated.as_str()));
    }
    for (index, miss_index) in line_misses.into_iter().enumerate() {
        if let Some(miss_index) = miss_index {
            results[index] = translated_misses[miss_index].clone();
        }
    }
    Ok(results)
}

pub fn partition_eztrans_batches(
    originals: &[Arc<str>],
    process_count: usize,
) -> Vec<Vec<Arc<str>>> {
    if originals.is_empty() {
        return Vec::new();
    }
    let process_count = process_count.max(1);
    let total_chars = originals
        .iter()
        .map(|text| text.chars().count().saturating_add(1))
        .sum::<usize>();
    let target_lines = originals
        .len()
        .div_ceil(process_count)
        .clamp(1, EZTRANS_BATCH_MAX_LINES);
    let target_chars = total_chars
        .div_ceil(process_count)
        .clamp(1, EZTRANS_BATCH_MAX_CHARS);
    let mut batches = Vec::new();
    let mut current = Vec::new();
    let mut current_chars = 0usize;

    for original in originals {
        let separator = usize::from(!current.is_empty());
        let chars = original.chars().count();
        let exceeds_target = !current.is_empty()
            && (current.len() >= target_lines
                || current_chars
                    .saturating_add(separator)
                    .saturating_add(chars)
                    > target_chars
                || current.len() >= EZTRANS_BATCH_MAX_LINES
                || current_chars
                    .saturating_add(separator)
                    .saturating_add(chars)
                    > EZTRANS_BATCH_MAX_CHARS);
        if exceeds_target {
            batches.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        current_chars = current_chars
            .saturating_add(usize::from(!current.is_empty()))
            .saturating_add(chars);
        current.push(original.clone());
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

pub(super) fn should_translate_line(line: &str, no_trans_linefeed: bool) -> bool {
    !line.is_empty() && !(no_trans_linefeed && line.trim().is_empty())
}

/// EzTrans는 다중 줄 입력의 줄바꿈 양옆에 공백 하나를 삽입한다. 원문 경계에
/// 이미 공백이 있으면 추가하지 않으므로, 원문에 없던 경계 공백만 제거한다.
pub fn split_eztrans_batch(translated: &str, originals: &[&str]) -> Option<Vec<String>> {
    let mut parts = translated
        .split('\n')
        .map(|part| part.strip_suffix('\r').unwrap_or(part).to_string())
        .collect::<Vec<_>>();
    if parts.len() != originals.len() {
        return None;
    }

    let last = parts.len().saturating_sub(1);
    for (index, (part, original)) in parts.iter_mut().zip(originals).enumerate() {
        if index > 0
            && !original.starts_with(char::is_whitespace)
            && let Some(stripped) = part.strip_prefix(' ')
        {
            *part = stripped.to_string();
        }
        if index < last
            && !original.ends_with(char::is_whitespace)
            && let Some(stripped) = part.strip_suffix(' ')
        {
            part.truncate(stripped.len());
        }
    }
    Some(parts)
}
