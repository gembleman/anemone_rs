use super::*;

#[test]
fn blank_system_prompt_falls_back_to_default() {
    for blank in ["", "   \r\n\t"] {
        let prompt = build_system_prompt_with_glossary(blank, Language::Jpn, Language::Kor, &[]);
        assert!(prompt.contains("Translate Japanese into Korean"), "{prompt}");
        assert!(prompt.contains("Output only the translated text"), "{prompt}");
    }
}

#[test]
fn custom_system_prompt_is_preserved_and_expanded() {
    let prompt = build_system_prompt_with_glossary(
        "{source} => {target}",
        Language::Eng,
        Language::Kor,
        &[],
    );
    assert_eq!(prompt, "English => Korean");
}

#[test]
fn prompt_expands_repeated_placeholders_and_skips_blank_glossary_entries() {
    let prompt = build_system_prompt_with_glossary(
        "{source}/{target}/{source}",
        Language::Eng,
        Language::Kor,
        &[
            GlossaryEntry {
                source: String::new(),
                target: "ignored".into(),
            },
            GlossaryEntry {
                source: "Alice".into(),
                target: "앨리스".into(),
            },
        ],
    );
    assert!(prompt.starts_with("English/Korean/English"));
    assert!(prompt.contains("Alice → 앨리스"));
    assert!(!prompt.contains("ignored"));
}

#[test]
fn openai_model_presets_include_current_families_and_default() {
    let presets = LlmProvider::OpenAi.model_presets();
    assert!(presets.contains(&LlmProvider::OpenAi.default_model()));
    for model in ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
        assert!(
            presets.contains(&model),
            "missing OpenAI model preset: {model}"
        );
    }

    let unique: std::collections::HashSet<_> = presets.iter().collect();
    assert_eq!(unique.len(), presets.len());
}

#[test]
fn openai_default_model_stays_gpt_5_4_nano() {
    assert_eq!(LlmProvider::OpenAi.default_model(), "gpt-5.4-nano");
}

#[test]
fn openrouter_has_no_default_model() {
    assert_eq!(LlmProvider::OpenRouter.default_model(), "");
    assert_eq!(LlmProvider::OpenRouter.model_or_default(""), "");
}

#[test]
fn provider_model_presets_include_current_models_and_default() {
    let cases = [
        (
            LlmProvider::Anthropic,
            &["claude-fable-5", "claude-opus-4-8", "claude-sonnet-5"][..],
        ),
        (
            LlmProvider::Gemini,
            &[
                "gemini-3.5-flash",
                "gemini-3.1-pro-preview",
                "gemini-2.5-pro",
            ][..],
        ),
        (
            LlmProvider::Grok,
            &["grok-4.5", "grok-4.3", "grok-4.20-0309-reasoning"][..],
        ),
    ];

    for (provider, expected) in cases {
        let presets = provider.model_presets();
        assert!(presets.contains(&provider.default_model()));
        for model in expected {
            assert!(
                presets.contains(model),
                "missing {provider:?} model: {model}"
            );
        }

        let unique: std::collections::HashSet<_> = presets.iter().collect();
        assert_eq!(unique.len(), presets.len(), "duplicate {provider:?} model");
    }
}

#[test]
fn blank_model_uses_provider_default_for_ui_and_api_calls() {
    assert_eq!(
        LlmProvider::OpenAi.model_or_default(""),
        LlmProvider::OpenAi.default_model()
    );
    assert_eq!(
        LlmProvider::OpenAi.model_or_default("future-or-private-model-id"),
        "future-or-private-model-id"
    );
}
