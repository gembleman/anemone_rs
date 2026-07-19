use super::{record_last_applied, restore_last_applied, take_unapplied_changes};
use crate::config::Config;
use crate::dialogs::models::SettingsDraft;
use std::cell::{Cell, RefCell};

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

#[test]
fn closing_restores_the_most_recently_applied_draft() {
    let pending = Cell::new(true);
    let last_applied = RefCell::new(SettingsDraft::new(Config::default()));

    let applied = Config {
        window_topmost: true,
        ..Config::default()
    };
    let draft = RefCell::new(SettingsDraft::new(applied));
    record_last_applied(&draft, &last_applied);

    // 적용 뒤 다시 조절했지만 적용하지 않고 닫는 상황.
    draft.borrow_mut().window_topmost = false;

    let restored = restore_last_applied(&pending, &draft, &last_applied).unwrap();

    assert!(restored.window_topmost);
    assert!(draft.borrow().window_topmost);
    assert!(!pending.get());
}

#[test]
fn closing_without_unapplied_changes_keeps_the_draft() {
    let pending = Cell::new(false);
    let last_applied = RefCell::new(SettingsDraft::new(Config::default()));
    let current = Config {
        window_topmost: true,
        ..Config::default()
    };
    let draft = RefCell::new(SettingsDraft::new(current));

    assert!(restore_last_applied(&pending, &draft, &last_applied).is_none());
    assert!(draft.borrow().window_topmost);
}
