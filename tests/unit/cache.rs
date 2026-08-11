use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

fn temp_db_path() -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "anemone_cache_test_{}_{}.sqlite3",
        std::process::id(),
        id
    ))
}

fn sample_key(original: &str) -> CacheKey {
    CacheKey {
        engine_id: "llm:gpt-test".to_string(),
        source_lang: "ja",
        target_lang: "ko",
        original: original.to_string(),
    }
}

#[test]
fn miss_then_hit_after_put() {
    let path = temp_db_path();
    let store = TranslationCacheStore::open(&path);

    let key = sample_key("こんにちは");
    assert_eq!(store.get(&key), None);

    store.put(&key, "안녕하세요");
    assert_eq!(store.get(&key).as_deref(), Some("안녕하세요"));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn put_overwrites_existing_translation() {
    let path = temp_db_path();
    let store = TranslationCacheStore::open(&path);

    let key = sample_key("おはよう");
    store.put(&key, "좋은 아침");
    store.put(&key, "좋은 아침이에요");
    assert_eq!(store.get(&key).as_deref(), Some("좋은 아침이에요"));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn different_engine_id_is_a_separate_cache_entry() {
    let path = temp_db_path();
    let store = TranslationCacheStore::open(&path);

    let mut key_a = sample_key("こんにちは");
    key_a.engine_id = "llm:model-a".to_string();
    let mut key_b = sample_key("こんにちは");
    key_b.engine_id = "llm:model-b".to_string();

    store.put(&key_a, "A 번역");
    assert_eq!(store.get(&key_b), None);
    assert_eq!(store.get(&key_a).as_deref(), Some("A 번역"));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn clear_removes_all_entries() {
    let path = temp_db_path();
    let store = TranslationCacheStore::open(&path);

    let key = sample_key("さようなら");
    store.put(&key, "안녕히 가세요");
    assert!(store.get(&key).is_some());

    store.clear();
    assert_eq!(store.get(&key), None);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn clear_vacuum_shrinks_the_database_file() {
    let path = temp_db_path();
    let store = TranslationCacheStore::open(&path);

    // 페이지 단위가 섞이도록 여러 번 put/clear를 반복해 단편화된 공간을 만든다.
    for round in 0..4 {
        for i in 0..200 {
            let mut key = sample_key(&format!("エントリ {round}-{i}"));
            key.original = format!("エントリ {round}-{i}");
            store.put(&key, &"번역".repeat(i + 1));
        }
        store.clear();
    }
    assert!(store.get(&sample_key("エントリ 3-199")).is_none());

    // WAL 모드라 실제 데이터는 -wal 파일에 있을 수 있다 — 메인 파일은 VACUUM
    // 이후 체크포인트로 합쳐진다. VACUUM은 백그라운드 스레드에서 실행되므로
    // 완료될 때까지 폴링한다 (실패하면 VACUUM 완료 로그를 기다리지 않아야 한다).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let page_count = loop {
        let count = rusqlite::Connection::open(&path)
            .unwrap()
            .pragma_query_value(None, "page_count", |row| row.get::<_, i64>(0))
            .unwrap();
        if count < 8 || std::time::Instant::now() > deadline {
            break count;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert!(
        page_count < 8,
        "VACUUM 후 페이지가 회수되어야 합니다 (page_count={page_count})"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn unwritable_path_degrades_to_disabled_cache_without_panicking() {
    // 존재하지 않는 디렉터리는 sqlite가 열 수 없으므로 캐시가 조용히 비활성화되어야 한다.
    let path = std::path::Path::new("Z:\\definitely\\missing\\dir\\cache.sqlite3");
    let store = TranslationCacheStore::open(path);

    let key = sample_key("test");
    assert_eq!(store.get(&key), None);
    store.put(&key, "value");
    store.clear();
}

#[test]
fn prune_removes_expired_entries() {
    let path = temp_db_path();
    let store = TranslationCacheStore::open(&path);

    let fresh = sample_key("fresh");
    store.put(&fresh, "새 항목");
    let expired = sample_key("expired");
    store.put(&expired, "오래된 항목");

    // put은 현재 시각으로 기록하므로, 검증용 원시 연결로 직접 과거 시각을 심는다.
    let raw = rusqlite::Connection::open(&path).unwrap();
    let cutoff = TranslationCacheStore::now_secs() - super::CACHE_TTL_SECS - 1;
    raw.execute(
        "UPDATE translation_cache SET updated_at = ?1 WHERE original = 'expired'",
        [cutoff],
    )
    .unwrap();

    store.prune();

    assert_eq!(store.get(&expired), None, "TTL 초과 항목은 제거되어야 합니다");
    assert_eq!(store.get(&fresh).as_deref(), Some("새 항목"));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn prune_caps_row_count_to_most_recent() {
    let path = temp_db_path();
    let store = TranslationCacheStore::open(&path);

    // 상한(10,000)을 넘는 신선한 행을 원시 연결로 한 트랜잭션에 삽입한다.
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch("BEGIN").unwrap();
    for i in 0..(super::CACHE_MAX_ROWS + 1_000) {
        raw.execute(
            "INSERT INTO translation_cache (engine_id, source_lang, target_lang, original, translation, updated_at)
             VALUES ('llm:gpt-test', 'ja', 'ko', ?1, 'v', ?2)",
            params![format!("행 {i}"), TranslationCacheStore::now_secs() + i],
        )
        .unwrap();
    }
    raw.execute_batch("COMMIT").unwrap();

    store.prune();

    let count: i64 = raw
        .query_row("SELECT COUNT(*) FROM translation_cache", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        count, super::CACHE_MAX_ROWS,
        "상한 초과분은 오래된 순으로 삭제되어야 합니다"
    );
    // 최신 상한 내 행은 남는다.
    let newest: Option<String> = raw
        .query_row(
            "SELECT translation FROM translation_cache WHERE original = ?1",
            [format!("행 {}", super::CACHE_MAX_ROWS + 999)],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(newest.as_deref(), Some("v"));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn ui_connection_has_busy_timeout_configured() {
    // 결함 1 회귀: UI 스레드 연결에 busy_timeout이 없으면 백그라운드 VACUUM이
    // write lock을 쥔 순간 get/put이 즉시 실패(캐시 미스로 위장)한다.
    let path = temp_db_path();
    let store = TranslationCacheStore::open(&path);

    let conn = store.conn.as_ref().expect("연결이 열려 있어야 합니다").borrow();
    let timeout_ms: i64 = conn
        .pragma_query_value(None, "busy_timeout", |row| row.get(0))
        .unwrap();
    assert_eq!(timeout_ms, UI_BUSY_TIMEOUT.as_millis() as i64);
    drop(conn);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn updated_at_index_exists() {
    // 결함 2 회귀: 인덱스가 없으면 TTL DELETE와 cap prune의 ORDER BY가 매번
    // 풀스캔 + 전체 정렬로 빠진다.
    let path = temp_db_path();
    let store = TranslationCacheStore::open(&path);

    let conn = store.conn.as_ref().expect("연결이 열려 있어야 합니다").borrow();
    let index_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index' AND name = 'idx_translation_cache_updated_at'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(index_count, 1, "updated_at 인덱스가 생성되어 있어야 합니다");
    drop(conn);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn prune_ttl_boundary_is_inclusive_of_cutoff() {
    // 결함 2 회귀: cap prune을 rowid 기반으로 바꾼 뒤에도 TTL 경계(<) 동작은
    // 그대로 유지되어야 한다 — cutoff와 정확히 같은 시각은 아직 만료 전이다.
    let path = temp_db_path();
    let store = TranslationCacheStore::open(&path);

    let at_cutoff = sample_key("at_cutoff");
    store.put(&at_cutoff, "v");
    let just_expired = sample_key("just_expired");
    store.put(&just_expired, "v");

    // prune과 동일한 `now`를 공유한다. 각자 now_secs()를 부르면 두 호출 사이에
    // 초 경계를 넘길 때 prune 쪽 cutoff만 1초 커져 at_cutoff 행이 삭제된다.
    let now = TranslationCacheStore::now_secs();
    let raw = rusqlite::Connection::open(&path).unwrap();
    let cutoff = now - super::CACHE_TTL_SECS;
    raw.execute(
        "UPDATE translation_cache SET updated_at = ?1 WHERE original = 'at_cutoff'",
        [cutoff],
    )
    .unwrap();
    raw.execute(
        "UPDATE translation_cache SET updated_at = ?1 WHERE original = 'just_expired'",
        [cutoff - 1],
    )
    .unwrap();

    store.prune_at(now);

    assert!(
        store.get(&at_cutoff).is_some(),
        "cutoff와 같은 시각의 항목은 아직 만료 전이어야 합니다"
    );
    assert_eq!(
        store.get(&just_expired),
        None,
        "cutoff보다 오래된 항목은 삭제되어야 합니다"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn prune_runs_every_n_puts() {
    let path = temp_db_path();
    let store = TranslationCacheStore::open(&path);

    // open 시점에 이미 TTL 초과인 행을 심는다.
    let raw = rusqlite::Connection::open(&path).unwrap();
    let old = TranslationCacheStore::now_secs() - super::CACHE_TTL_SECS - 1;
    raw.execute(
        "INSERT INTO translation_cache (engine_id, source_lang, target_lang, original, translation, updated_at)
         VALUES ('llm:gpt-test', 'ja', 'ko', 'stale', 'v', ?1)",
        [old],
    )
    .unwrap();

    // PUTS_PER_PRUNE 미만의 put으로는 아직 prune이 실행되지 않는다.
    for i in 0..(super::PUTS_PER_PRUNE - 1) {
        store.put(&sample_key(&format!("fresh {i}")), "v");
    }
    assert!(store.get(&sample_key("stale")).is_some());

    // PUTS_PER_PRUNE번째 put에서 prune이 실행되어 만료 행이 사라진다.
    store.put(&sample_key("fresh N"), "v");
    assert_eq!(store.get(&sample_key("stale")), None);

    let _ = std::fs::remove_file(&path);
}
