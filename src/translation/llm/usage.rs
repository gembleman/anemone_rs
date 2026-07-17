//! LLM 일일 사용량 카운터
//!
//! `<exe_dir>/llm_usage.json` 에 날짜별 호출 수와 누적 입력/출력 바이트를 기록한다.
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
    USAGE_PATH.get_or_init(|| {
        if let Ok(exe_path) = std::env::current_exe()
            && let Some(exe_dir) = exe_path.parent()
        {
            return exe_dir.join("llm_usage.json");
        }
        PathBuf::from("llm_usage.json")
    })
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

/// 오늘 날짜 키 ("YYYY-MM-DD"). 시스템 로컬 타임 기준 — 자정에 카운터가
/// 리셋되는 게 사용자 관점에서 자연스럽기 때문.
fn today_key() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: GetLocalTime 은 부수효과 없는 Win32 호출. 값만 받음.
    let st = unsafe { GetLocalTime() };
    if st.wYear == 0 {
        // SYSTEMTIME 이 0 인 경우는 사실상 없지만 방어적으로 폴백.
        return "1970-01-01".to_string();
    }
    format!("{:04}-{:02}-{:02}", st.wYear, st.wMonth, st.wDay)
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
    match serde_json::to_string_pretty(file) {
        Ok(s) => {
            if let Err(e) = std::fs::write(path, s) {
                tracing::warn!("llm_usage 저장 실패: {e}");
            }
        }
        Err(e) => tracing::warn!("llm_usage 직렬화 실패: {e}"),
    }
}

/// 한 번의 LLM 호출 성공을 기록. 임계 초과 시 한 번만 경고.
pub fn record(input_bytes: usize, output_bytes: usize) {
    let key = today_key();
    let (today_calls, threshold_crossed) = {
        let mut g = match lock().lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let entry = g.days.entry(key.clone()).or_default();
        let prev_calls = entry.calls;
        entry.calls = entry.calls.saturating_add(1);
        entry.input_bytes = entry.input_bytes.saturating_add(input_bytes as u64);
        entry.output_bytes = entry.output_bytes.saturating_add(output_bytes as u64);
        let crossed =
            prev_calls < DAILY_CALL_WARN_THRESHOLD && entry.calls >= DAILY_CALL_WARN_THRESHOLD;
        let calls = entry.calls;
        prune(&mut g);
        save_locked(&g);
        (calls, crossed)
    };

    if threshold_crossed {
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "LLM 일일 호출 수 임계({}) 초과 — 오늘({}) 현재 {} 회. \
                 비용 폭주에 주의. llm_usage.json 확인.",
                DAILY_CALL_WARN_THRESHOLD,
                key,
                today_calls,
            );
        }
    }
}
