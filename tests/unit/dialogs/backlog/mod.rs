use super::*;

#[test]
fn deferred_entries_are_preserved_in_the_open_view_snapshot() {
    PENDING_BACKLOG_ENTRIES.with(|pending| {
        let mut pending = pending.borrow_mut();
        pending.clear();
        pending.push_back(LogEntry::new("first".into()).with_translation("첫째".into()));
        pending.push_back(LogEntry::new("second".into()).with_translation("둘째".into()));
    });

    let mut store = BacklogStore::new();
    assert!(drain_pending_entries(&mut store));
    let rendered = store
        .render(BacklogFilter::All, true)
        .into_iter()
        .map(|segment| segment.text)
        .collect::<String>();

    assert!(rendered.contains("first"));
    assert!(rendered.contains("첫째"));
    assert!(rendered.contains("second"));
    assert!(rendered.contains("둘째"));
    assert!(!drain_pending_entries(&mut store));
}
