use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

fn temp_db_path() -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "anemone_mys_usage_test_{}_{}.sqlite3",
        std::process::id(),
        id
    ))
}

#[test]
fn accumulates_usage_in_sqlite_per_account_and_month() {
    let path = temp_db_path();
    let conn = open_connection(&path).unwrap();
    record_to_connection(
        &conn,
        "account-a",
        "2026-08",
        UsageDelta {
            fresh_prompt_tokens: 10,
            fresh_output_tokens: 4,
            cached_characters: 20,
            fresh_count: 1,
            cached_count: 2,
        },
    )
    .unwrap();
    record_to_connection(
        &conn,
        "account-a",
        "2026-08",
        UsageDelta {
            fresh_prompt_tokens: 3,
            fresh_output_tokens: 2,
            cached_characters: 5,
            fresh_count: 1,
            cached_count: 1,
        },
    )
    .unwrap();
    record_to_connection(
        &conn,
        "account-b",
        "2026-08",
        UsageDelta {
            cached_characters: 99,
            ..Default::default()
        },
    )
    .unwrap();

    let snapshot = snapshot_from_connection(&conn, "account-a", "2026-08")
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.fresh_prompt_tokens, 13);
    assert_eq!(snapshot.fresh_output_tokens, 6);
    assert_eq!(snapshot.cached_characters, 25);
    assert_eq!(snapshot.total_count(), 5);
    assert_eq!(
        snapshot_from_connection(&conn, "account-b", "2026-08")
            .unwrap()
            .unwrap()
            .cached_characters,
        99
    );

    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn usage_table_shares_a_database_with_translation_cache_data() {
    let path = temp_db_path();
    let cache = crate::app::translation_cache::TranslationCacheStore::open(&path);
    let key = crate::translation::CacheKey {
        engine_id: "mys_translater".into(),
        source_lang: "ja",
        target_lang: "ko",
        original: "こんにちは".into(),
    };
    cache.put(&key, "안녕하세요");

    let conn = open_connection(&path).unwrap();
    record_to_connection(
        &conn,
        "account-a",
        "2026-08",
        UsageDelta {
            fresh_prompt_tokens: 3,
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(cache.get(&key).as_deref(), Some("안녕하세요"));
    let table_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
             AND name IN ('translation_cache', 'mys_translater_usage')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(table_count, 2);

    cache.clear();
    assert_eq!(cache.get(&key), None);
    assert_eq!(
        snapshot_from_connection(&conn, "account-a", "2026-08")
            .unwrap()
            .unwrap()
            .fresh_prompt_tokens,
        3,
        "번역 캐시 비우기는 사용량을 삭제하면 안 됩니다"
    );

    drop(conn);
    drop(cache);
    let _ = std::fs::remove_file(path);
}

#[test]
fn sqlite_counters_saturate_instead_of_overflowing() {
    let path = temp_db_path();
    let conn = open_connection(&path).unwrap();
    for prompt_tokens in [u64::MAX, 1] {
        record_to_connection(
            &conn,
            "account-a",
            "2026-08",
            UsageDelta {
                fresh_prompt_tokens: prompt_tokens,
                ..Default::default()
            },
        )
        .unwrap();
    }

    let snapshot = snapshot_from_connection(&conn, "account-a", "2026-08")
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.fresh_prompt_tokens, i64::MAX as u64);

    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn historical_months_are_never_pruned() {
    let path = temp_db_path();
    let conn = open_connection(&path).unwrap();
    for year in 2000..=2026 {
        record_to_connection(
            &conn,
            "account-a",
            &format!("{year}-01"),
            UsageDelta {
                fresh_count: 1,
                ..Default::default()
            },
        )
        .unwrap();
    }

    let month_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM mys_translater_usage WHERE account_key = 'account-a'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(month_count, 27);

    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn account_identifier_does_not_contain_credentials() {
    let key = account_key("https://example.test/", "secret-token").unwrap();
    assert_eq!(key.len(), 64);
    assert!(!key.contains("example"));
    assert!(!key.contains("secret"));
    assert_eq!(
        key,
        account_key("https://example.test", " secret-token ").unwrap()
    );
}
