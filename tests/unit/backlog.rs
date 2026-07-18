use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

fn sample_entry() -> LogEntry {
    LogEntry {
        name: Some("화자".into()),
        original: "원문".into(),
        translation: Some("번역".into()),
    }
}

fn rendered_text(store: &BacklogStore, filter: BacklogFilter, add_linefeed: bool) -> String {
    store
        .render(filter, add_linefeed)
        .into_iter()
        .map(|part| part.text)
        .collect()
}

#[test]
fn filters_rendered_entries() {
    let mut store = BacklogStore::new();
    let _ = store.push(sample_entry());
    assert_eq!(
        rendered_text(&store, BacklogFilter::All, true),
        "[화자] 원문\r\n번역\r\n\r\n"
    );
    assert_eq!(
        rendered_text(&store, BacklogFilter::Original, false),
        "[화자] 원문"
    );
    assert_eq!(
        rendered_text(&store, BacklogFilter::Translation, true),
        "번역\r\n"
    );
}

#[test]
fn export_writes_utf8_bom_and_all_entries() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "anemone-backlog-{}-{}.txt",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut store = BacklogStore::new();
    let _ = store.push(sample_entry());
    store.export_utf8(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
    assert_eq!(
        String::from_utf8(bytes[3..].to_vec()).unwrap(),
        "[화자] 원문\r\n번역\r\n\r\n"
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn evicts_oldest_entries_and_resets_accounting_on_clear() {
    let mut store = BacklogStore::new();
    for index in 0..=MAX_BACKLOG_ENTRIES {
        let _ = store.push(LogEntry::new(format!("entry-{index}")));
    }

    assert_eq!(store.entries.len(), MAX_BACKLOG_ENTRIES);
    assert_eq!(store.entries.front().unwrap().original, "entry-1");
    assert!(store.text_bytes > 0);

    store.clear();
    assert!(store.entries.is_empty());
    assert_eq!(store.text_bytes, 0);
}

#[test]
fn text_budget_keeps_the_latest_entry_even_when_it_is_large() {
    let mut store = BacklogStore::new();
    let _ = store.push(LogEntry::new("old".into()));
    let latest = "x".repeat(MAX_BACKLOG_TEXT_BYTES + 1);
    assert!(store.push(LogEntry::new(latest.clone())));

    assert_eq!(store.entries.len(), 1);
    assert_eq!(store.entries.front().unwrap().original, latest);
}
