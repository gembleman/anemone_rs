use super::persist_if_pending;
use std::cell::Cell;

#[test]
fn failed_save_preserves_retry_flag() {
    let pending = Cell::new(true);

    let result = persist_if_pending(&pending, || Err::<(), _>("disk full"));

    assert_eq!(result, Err("disk full"));
    assert!(pending.get());
}

#[test]
fn no_pending_save_skips_persistence() {
    let pending = Cell::new(false);
    let called = Cell::new(false);

    let result = persist_if_pending(&pending, || {
        called.set(true);
        Ok::<(), ()>(())
    });

    assert_eq!(result, Ok(false));
    assert!(!called.get());
}
