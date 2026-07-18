use super::take_unapplied_changes;
use std::cell::Cell;

#[test]
fn apply_consumes_unapplied_changes() {
    let pending = Cell::new(true);

    assert!(take_unapplied_changes(&pending));
    assert!(!pending.get());
}

#[test]
fn apply_without_changes_is_a_noop() {
    let pending = Cell::new(false);

    assert!(!take_unapplied_changes(&pending));
}
