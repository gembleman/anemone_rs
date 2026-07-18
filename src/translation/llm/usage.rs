//! LLM 일일 사용량 카운터
//!
//! 사용자 데이터 디렉터리의 `llm_usage.json`에 날짜별 호출 수와 누적
//! 입력/출력 바이트를 기록한다.
//! 모든 LLM 호출의 성공 응답 시점에서 [`record`] 를 부른다. 일일 임계 초과 시
//! 프로세스 수명 동안 한 번만 경고 로그를 남긴다.
//!
//! 워커 스레드에서 호출되므로 file I/O 는 `Mutex` 로 직렬화한다. LLM 호출 자체가
//! 수 초 단위라 디스크 쓰기 1회는 상대적으로 무시 가능.

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

/// 오늘 날짜 키 ("YYYY-MM-DD"). 시스템 로컬 타임 기준 — 자정에 카운터가
/// 리셋되는 게 사용자 관점에서 자연스럽기 때문.
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
mod tests {
    use super::*;

    struct FixedDate(Date);

    impl DateProvider for FixedDate {
        fn today(&self) -> Date {
            self.0
        }
    }

    fn date(year: i32, month: time::Month, day: u8) -> Date {
        Date::from_calendar_date(year, month, day).unwrap()
    }

    #[test]
    fn injected_local_date_changes_the_bucket_at_midnight() {
        let mut file = UsageFile::default();
        apply_record(
            &mut file,
            &FixedDate(date(2026, time::Month::July, 18)),
            10,
            20,
        );
        apply_record(
            &mut file,
            &FixedDate(date(2026, time::Month::July, 19)),
            30,
            40,
        );

        assert_eq!(file.days["2026-07-18"].calls, 1);
        assert_eq!(file.days["2026-07-19"].calls, 1);
        assert_eq!(file.days["2026-07-19"].input_bytes, 30);
    }

    #[test]
    fn retention_keeps_the_newest_thirty_injected_dates() {
        let mut file = UsageFile::default();
        for day in 1u8..=31 {
            apply_record(
                &mut file,
                &FixedDate(date(2026, time::Month::January, day)),
                1,
                1,
            );
        }

        assert_eq!(file.days.len(), RETAIN_DAYS);
        assert!(!file.days.contains_key("2026-01-01"));
        assert!(file.days.contains_key("2026-01-31"));
    }

    #[test]
    fn usage_file_is_replaced_atomically_with_valid_json() {
        let root = std::env::temp_dir().join(format!(
            "anemone-usage-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("llm_usage.json");
        let mut file = UsageFile::default();
        file.days.insert(
            "2026-07-18".to_string(),
            DayStats {
                calls: 3,
                input_bytes: 20,
                output_bytes: 40,
            },
        );

        save_to_path(&file, &path).unwrap();

        let loaded: UsageFile =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.days["2026-07-18"].calls, 3);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
