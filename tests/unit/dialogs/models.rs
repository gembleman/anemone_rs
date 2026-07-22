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
fn eztrans_dictionary_draft_is_separate_from_the_llm_glossary() {
    let mut config = Config::default();
    config.translation.llm.glossary = vec![crate::config::LlmGlossaryEntry {
        source: "LLM".into(),
        target: "엘엘엠".into(),
    }];
    let mut draft = GlossaryDraft::from_eztrans_config(&config);
    draft.add_or_update("이지트랜스".into(), "EzTrans".into());
    draft.commit(&mut config);

    assert_eq!(config.translation.llm.glossary[0].source, "LLM");
    assert_eq!(
        config.translation.eztrans_postprocess_dictionary[0].source,
        "이지트랜스"
    );
}
