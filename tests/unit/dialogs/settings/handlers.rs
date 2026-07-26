use super::{
    commits_on_enter, numeric_binding_for_edit, numeric_binding_for_trackbar, record_last_applied,
    restore_last_applied, take_unapplied_changes,
};
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

#[test]
fn every_appearance_numeric_input_is_bound_to_its_trackbar() {
    use super::ctrl_id;

    let pairs = [
        (ctrl_id::BACKGROUND_TRACKBAR, ctrl_id::BACKGROUND_EDIT),
        (ctrl_id::TEXTSIZE_TRACKBAR, ctrl_id::TEXTSIZE_EDIT),
        (ctrl_id::OUTLINE1_TRACKBAR, ctrl_id::OUTLINE1_EDIT),
        (ctrl_id::OUTLINE2_TRACKBAR, ctrl_id::OUTLINE2_EDIT),
        (ctrl_id::SHADOW_X_TRACKBAR, ctrl_id::SHADOW_X_EDIT),
        (ctrl_id::SHADOW_Y_TRACKBAR, ctrl_id::SHADOW_Y_EDIT),
        (ctrl_id::MARGIN_X_TRACKBAR, ctrl_id::MARGIN_X_EDIT),
        (ctrl_id::MARGIN_Y_TRACKBAR, ctrl_id::MARGIN_Y_EDIT),
        (ctrl_id::MARGIN_NAME_TRACKBAR, ctrl_id::MARGIN_NAME_EDIT),
        (ctrl_id::BORDER_SIZE_TRACKBAR, ctrl_id::BORDER_SIZE_EDIT),
    ];

    for (trackbar_id, edit_id) in pairs {
        let from_trackbar = numeric_binding_for_trackbar(trackbar_id).expect("trackbar binding");
        let from_edit = numeric_binding_for_edit(edit_id).expect("edit binding");
        assert_eq!(from_trackbar.trackbar_id, trackbar_id);
        assert_eq!(from_trackbar.edit_id, edit_id);
        assert_eq!(from_edit.trackbar_id, trackbar_id);
        assert_eq!(from_edit.edit_id, edit_id);
        assert_eq!(from_edit.setting, from_trackbar.setting);
    }
}

#[test]
fn enter_commits_numeric_edits_without_invoking_a_dialog_button() {
    use super::ctrl_id;

    for id in [
        ctrl_id::BACKGROUND_EDIT,
        ctrl_id::TEXTSIZE_EDIT,
        ctrl_id::OUTLINE1_EDIT,
        ctrl_id::OUTLINE2_EDIT,
        ctrl_id::SHADOW_X_EDIT,
        ctrl_id::SHADOW_Y_EDIT,
        ctrl_id::MARGIN_X_EDIT,
        ctrl_id::MARGIN_Y_EDIT,
        ctrl_id::MARGIN_NAME_EDIT,
        ctrl_id::BORDER_SIZE_EDIT,
        ctrl_id::LLM_MAX_TOKENS_EDIT,
        ctrl_id::LLM_TEMPERATURE_EDIT,
        ctrl_id::LLM_DEBOUNCE_EDIT,
    ] {
        assert!(commits_on_enter(id), "numeric edit {id} must consume Enter");
    }

    assert!(!commits_on_enter(ctrl_id::LLM_SYSTEM_PROMPT_EDIT));
    assert!(!commits_on_enter(ctrl_id::APPLY));
}
