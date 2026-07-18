use super::*;

#[test]
fn glossary_draft_adds_updates_removes_and_commits() {
    let mut config = Config::default();
    let mut draft = GlossaryDraft::from_config(&config);
    assert_eq!(
        draft.add_or_update("Alice".into(), "앨리스".into()),
        Some(DraftChange::Added(0))
    );
    assert_eq!(
        draft.add_or_update("Alice".into(), "알리스".into()),
        Some(DraftChange::Updated(0))
    );
    assert_eq!(draft.entries()[0].target, "알리스");
    assert!(draft.remove(0));
    assert!(!draft.remove(0));
    draft.add_or_update("Bob".into(), "밥".into());
    draft.commit(&mut config);
    assert_eq!(config.translation.llm.glossary[0].source, "Bob");
}

#[test]
fn hook_draft_moves_reorders_and_commits() {
    let mut config = Config::default();
    config.hook.active_hooks = vec!["A".into(), "B".into()];
    config.hook.inactive_hooks = vec!["C".into()];
    let mut draft = HookListDraft::from_config(&config);
    assert_eq!(
        draft.move_to_inactive(0),
        Some(SelectionChange {
            source_selection: Some(0),
            target_selection: 1
        })
    );
    assert_eq!(
        draft.move_to_active(0),
        Some(SelectionChange {
            source_selection: Some(0),
            target_selection: 1
        })
    );
    assert_eq!(draft.move_active_by(1, -1), Some(0));
    assert_eq!(draft.active(), ["C", "B"]);
    assert_eq!(draft.move_active_by(0, -1), None);
    draft.commit(&mut config);
    assert_eq!(config.hook.active_hooks, ["C", "B"]);
    assert_eq!(config.hook.inactive_hooks, ["A"]);
}
