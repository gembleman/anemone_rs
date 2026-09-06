//! 번역 캐시 sqlite put 지연 벤치마크 (2-g 측정).
//!
//! `TranslationCacheStore::put`은 UI 스레드에서 `synchronous=FULL` commit
//! (fsync)을 수행한다. 느린 디스크/바이러스 검사 환경에서 UI 블록이 얼마나
//! 되는지 10,000회 put으로 실측한다. 대조군으로 `synchronous=NORMAL` 동일
//! 스키마 커넥션도 함께 측정해 채택 판정 근거로 쓴다.

use super::mem::MemSnapshot;
use anemone_rs::{BenchmarkCacheKey, BenchmarkTranslationCacheStore};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static CACHE_BENCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 테스트마다 독립 temp 디렉터리를 만들고 drop 시 정리한다.
struct BenchCacheDirectory(PathBuf);

impl BenchCacheDirectory {
    fn new() -> Self {
        let sequence = CACHE_BENCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "anemone-cache-bench-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for BenchCacheDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn key(i: usize) -> BenchmarkCacheKey {
    BenchmarkCacheKey {
        engine_id: "google".to_owned(),
        source_lang: "ja",
        target_lang: "ko",
        original: format!("원문 {i:08}"),
    }
}

fn report(label: &str, samples: &[Duration], units: usize) {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let avg = sorted.iter().map(Duration::as_secs_f64).sum::<f64>() / n as f64 * 1_000_000.0;
    let p50 = sorted[n / 2].as_secs_f64() * 1_000_000.0;
    let p99 = sorted[(n * 99 / 100).min(n - 1)].as_secs_f64() * 1_000_000.0;
    let min = sorted[0].as_secs_f64() * 1_000_000.0;
    let max = sorted[n - 1].as_secs_f64() * 1_000_000.0;
    let line = format!(
        "[bench {label}] n={n} avg={avg:.1}us p50={p50:.1}us p99={p99:.1}us min={min:.1}us max={max:.1}us (units={units})"
    );
    eprintln!("{line}");
}

/// `TranslationCacheStore` (synchronous=FULL)와 NORMAL 대조군의 put 지연을
/// 각각 10,000회 측정한다.
#[test]
#[ignore = "performance benchmark that measures sqlite put latency on the UI-thread path"]
fn measures_translation_cache_put_latency() {
    assert!(
        !std::hint::black_box(cfg!(debug_assertions)),
        "performance measurements must run with cargo test --release"
    );
    let directory = BenchCacheDirectory::new();
    let db_path = directory.0.join("translation_cache.sqlite3");

    const ITERS: usize = 10_000;

    // 실제 앱 경로와 동일한 TranslationCacheStore (WAL + 기본 synchronous=FULL).
    let store = BenchmarkTranslationCacheStore::open(&db_path);
    let mut samples = Vec::with_capacity(ITERS);
    let mem_before = MemSnapshot::now();
    for i in 0..ITERS {
        let k = key(i);
        let started = Instant::now();
        store.put(&k, "번역 결과");
        samples.push(started.elapsed());
    }
    let mem = MemSnapshot::now().delta(mem_before);
    report("cache_put_full", &samples, ITERS);
    mem.report_per("cache_put_full", ITERS);

    // 대조군: 같은 스키마, synchronous=NORMAL. WAL에서 fsync가 checkpoint로
    // 미뤄지는 효과만 분리한다 (나머지 SQL/바인딩 비용은 동일).
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "synchronous", "NORMAL").unwrap();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS translation_cache (
            engine_id TEXT NOT NULL,
            source_lang TEXT NOT NULL,
            target_lang TEXT NOT NULL,
            original TEXT NOT NULL,
            translation TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (engine_id, source_lang, target_lang, original)
        )",
        [],
    )
    .unwrap();

    let mut samples_normal = Vec::with_capacity(ITERS);
    let mut counter = ITERS;
    for _ in 0..ITERS {
        counter += 1;
        let k = key(counter);
        let started = Instant::now();
        conn.execute(
            "INSERT INTO translation_cache (engine_id, source_lang, target_lang, original, translation, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(engine_id, source_lang, target_lang, original)
             DO UPDATE SET translation = excluded.translation, updated_at = excluded.updated_at",
            rusqlite::params![
                k.engine_id,
                k.source_lang,
                k.target_lang,
                k.original,
                "번역 결과",
                0_i64,
            ],
        )
        .unwrap();
        samples_normal.push(started.elapsed());
    }
    report("cache_put_normal", &samples_normal, ITERS);
}
