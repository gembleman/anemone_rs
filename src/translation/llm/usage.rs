//! 날짜별 LLM 호출 수와 입출력 byte를 `llm_usage.json`에 기록한다.
//! File I/O는 mutex로 직렬화하고 일일 임계 초과는 process당 한 번 경고한다.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use time::{Date, OffsetDateTime};

/// 일일 호출 수 임계 — 초과 시 경고 한 번. 사용자가 직접 편집해 늘릴 수 있음.
const DAILY_CALL_WARN_THRESHOLD: u32 = 500;

/// 보관 일수 — 이보다 오래된 날짜 엔트리는 다음 저장 시 정리.
const RETAIN_DAYS: usize = 30;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DayStats {
    calls: u32,
    /// 입력 바이트(UTF-8) 누적 — 토큰 추정치(대략 4 bytes/token).
    input_bytes: u64,
    /// 출력 바이트(UTF-8) 누적.
    output_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct UsageFile {
    /// "YYYY-MM-DD" → 통계
    #[serde(default)]
    days: BTreeMap<String, DayStats>,
}

fn usage_path() -> &'static PathBuf {
    static USAGE_PATH: OnceLock<PathBuf> = OnceLock::new();
    USAGE_PATH.get_or_init(|| crate::runtime::data_file("llm_usage.json"))
}

fn lock() -> &'static Mutex<UsageFile> {
    static FILE: OnceLock<Mutex<UsageFile>> = OnceLock::new();
    FILE.get_or_init(|| {
        let loaded = std::fs::read_to_string(usage_path())
            .ok()
            .and_then(|s| serde_json::from_str::<UsageFile>(&s).ok())
            .unwrap_or_default();
        Mutex::new(loaded)
    })
}

trait DateProvider {
    fn today(&self) -> Date;
}

struct SystemDateProvider;

impl DateProvider for SystemDateProvider {
    fn today(&self) -> Date {
        match OffsetDateTime::now_local() {
            Ok(now) => now.date(),
            Err(error) => {
                // 로컬 오프셋을 확인할 수 없는 비정상 환경에서도 사용량 기록은
                // 중단하지 않는다. 이 경우에만 UTC 날짜로 명시적으로 폴백한다.
                static WARNED: AtomicBool = AtomicBool::new(false);
                if !WARNED.swap(true, Ordering::Relaxed) {
                    tracing::warn!("로컬 날짜 확인 실패, UTC 날짜 사용: {error}");
                }
                OffsetDateTime::now_utc().date()
            }
        }
    }
}

/// 사용자 기준 자정에 초기화되도록 local 날짜 key를 반환한다.
fn today_key(provider: &impl DateProvider) -> String {
    let date = provider.today();
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

fn prune(file: &mut UsageFile) {
    if file.days.len() > RETAIN_DAYS {
        let to_remove: Vec<String> = file.days.keys().rev().skip(RETAIN_DAYS).cloned().collect();
        for k in to_remove {
            file.days.remove(&k);
        }
    }
}

fn save_locked(file: &UsageFile) {
    let path = usage_path();
    if let Err(error) = save_to_path(file, path) {
        tracing::warn!("llm_usage 저장 실패: {error}");
    }
}

fn save_to_path(file: &UsageFile, path: &std::path::Path) -> Result<(), String> {
    let serialized =
        serde_json::to_string_pretty(file).map_err(|error| format!("직렬화 실패: {error}"))?;
    let _: UsageFile =
        serde_json::from_str(&serialized).map_err(|error| format!("저장 전 검증 실패: {error}"))?;
    crate::fs_util::atomic_write(path, serialized.as_bytes()).map_err(|error| error.to_string())
}

struct RecordOutcome {
    key: String,
    calls: u32,
    threshold_crossed: bool,
}

fn apply_record(
    file: &mut UsageFile,
    provider: &impl DateProvider,
    input_bytes: usize,
    output_bytes: usize,
) -> RecordOutcome {
    let key = today_key(provider);
    let entry = file.days.entry(key.clone()).or_default();
    let prev_calls = entry.calls;
    entry.calls = entry.calls.saturating_add(1);
    entry.input_bytes = entry.input_bytes.saturating_add(input_bytes as u64);
    entry.output_bytes = entry.output_bytes.saturating_add(output_bytes as u64);
    let outcome = RecordOutcome {
        key,
        calls: entry.calls,
        threshold_crossed: prev_calls < DAILY_CALL_WARN_THRESHOLD
            && entry.calls >= DAILY_CALL_WARN_THRESHOLD,
    };
    prune(file);
    outcome
}

/// 한 번의 LLM 호출 성공을 기록. 임계 초과 시 한 번만 경고.
pub fn record(input_bytes: usize, output_bytes: usize) {
    let outcome = {
        let mut g = match lock().lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let outcome = apply_record(&mut g, &SystemDateProvider, input_bytes, output_bytes);
        save_locked(&g);
        outcome
    };

    if outcome.threshold_crossed {
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "LLM 일일 호출 수 임계({}) 초과 — 오늘({}) 현재 {} 회. \
                 비용 폭주에 주의. llm_usage.json 확인.",
                DAILY_CALL_WARN_THRESHOLD,
                outcome.key,
                outcome.calls,
            );
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/translation/llm/usage.rs"]
mod tests;
