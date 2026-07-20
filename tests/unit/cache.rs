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
fn unwritable_path_degrades_to_disabled_cache_without_panicking() {
    // 존재하지 않는 디렉터리는 sqlite가 열 수 없으므로 캐시가 조용히 비활성화되어야 한다.
    let path = std::path::Path::new("Z:\\definitely\\missing\\dir\\cache.sqlite3");
    let store = TranslationCacheStore::open(path);

    let key = sample_key("test");
    assert_eq!(store.get(&key), None);
    store.put(&key, "value");
    store.clear();
}
