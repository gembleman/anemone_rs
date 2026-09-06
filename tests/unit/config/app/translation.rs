//! 번역/LLM 관련 설정 값의 정규화와 TOML 왕복 테스트.
use super::*;

#[test]
fn llm_limits_are_normalized_at_every_config_boundary() {
    let mut raw = Config::default();
    raw.translation.llm.max_tokens = u32::MAX;
    raw.translation.llm.debounce_ms = u32::MAX;
    raw.translation.llm.temperature = f32::INFINITY;
    raw.translation.llm.top_p = 3.0;
    raw.translation.llm.frequency_penalty = -3.0;
    raw.translation.llm.presence_penalty = f32::INFINITY;

    let toml = toml::to_string(&raw).unwrap();
    let normalized = Config::from_toml_str(&toml).unwrap();

    assert_eq!(normalized.translation.llm.max_tokens, 32_000);
    assert_eq!(normalized.translation.llm.debounce_ms, 10_000);
    assert_eq!(normalized.translation.llm.temperature, 1.0);
    assert_eq!(normalized.translation.llm.top_p, 1.0);
    assert_eq!(normalized.translation.llm.frequency_penalty, -2.0);
    assert_eq!(normalized.translation.llm.presence_penalty, 0.0);
}

#[test]
fn openrouter_sampling_options_are_loaded_and_saved_without_ui() {
    let text = r#"
[translation.llm]
provider = "openrouter"
model = "anthropic/claude-sonnet-5"
top_p = 0.8
frequency_penalty = 0.4
presence_penalty = -0.2
"#;

    let loaded = Config::from_toml_str(text).unwrap();
    assert_eq!(loaded.translation.llm.top_p, 0.8);
    assert_eq!(loaded.translation.llm.frequency_penalty, 0.4);
    assert_eq!(loaded.translation.llm.presence_penalty, -0.2);
    let params = loaded.translation.llm.to_call_params().unwrap();
    assert_eq!(params.top_p, 0.8);
    assert_eq!(params.frequency_penalty, 0.4);
    assert_eq!(params.presence_penalty, -0.2);

    let serialized = toml::to_string_pretty(&loaded).unwrap();
    let reloaded = Config::from_toml_str(&serialized).unwrap();
    assert_eq!(reloaded.translation.llm.top_p, 0.8);
    assert_eq!(reloaded.translation.llm.frequency_penalty, 0.4);
    assert_eq!(reloaded.translation.llm.presence_penalty, -0.2);
}

#[test]
fn llm_base_url_is_loaded_from_and_saved_to_toml() {
    let text = r#"
[translation.llm]
base_url = "http://127.0.0.1:1234/v1"
"#;

    let loaded = Config::from_toml_str(text).unwrap();
    assert_eq!(loaded.translation.llm.base_url, "http://127.0.0.1:1234/v1");

    let serialized = toml::to_string_pretty(&loaded).unwrap();
    let reloaded = Config::from_toml_str(&serialized).unwrap();
    assert_eq!(
        reloaded.translation.llm.base_url,
        "http://127.0.0.1:1234/v1"
    );
}

#[test]
fn arbitrary_llm_model_is_loaded_from_and_saved_to_toml() {
    let text = r#"
[translation.llm]
provider = "openai"
model = "future-or-private-model-id"
"#;

    let loaded = Config::from_toml_str(text).unwrap();
    assert_eq!(loaded.translation.llm.model, "future-or-private-model-id");

    let serialized = toml::to_string_pretty(&loaded).unwrap();
    let reloaded = Config::from_toml_str(&serialized).unwrap();
    assert_eq!(reloaded.translation.llm.model, "future-or-private-model-id");
}

#[test]
fn llm_reasoning_effort_is_optional_and_round_trips() {
    use crate::translation::llm::ReasoningEffort;

    let default_config = Config::from_toml_str("[translation.llm]\n").unwrap();
    assert_eq!(default_config.translation.llm.reasoning_effort, None);

    let configured = Config::from_toml_str(
        "[translation.llm]\nprovider = \"openai\"\nreasoning_effort = \"xhigh\"\n",
    )
    .unwrap();
    assert_eq!(
        configured.translation.llm.reasoning_effort,
        Some(ReasoningEffort::Xhigh)
    );

    let serialized = toml::to_string_pretty(&configured).unwrap();
    let reloaded = Config::from_toml_str(&serialized).unwrap();
    assert_eq!(
        reloaded.translation.llm.reasoning_effort,
        Some(ReasoningEffort::Xhigh)
    );
}

#[test]
fn llm_provider_profiles_round_trip_with_the_config() {
    use crate::translation::LlmProvider;

    let mut config = Config::default();
    config.translation.llm.api_key = "openai-key".into();
    config.translation.llm.temperature = 0.24;
    config.translation.llm.set_provider(LlmProvider::Anthropic);
    config.translation.llm.api_key = "anthropic-key".into();
    config.translation.llm.temperature = 0.68;

    let serialized = toml::to_string_pretty(&config).unwrap();
    let mut reloaded = Config::from_toml_str(&serialized).unwrap();
    assert_eq!(reloaded.translation.llm.api_key, "anthropic-key");
    assert_eq!(reloaded.translation.llm.temperature, 0.68);

    reloaded.translation.llm.set_provider(LlmProvider::OpenAi);
    assert_eq!(reloaded.translation.llm.api_key, "openai-key");
    assert_eq!(reloaded.translation.llm.temperature, 0.24);

    reloaded
        .translation
        .llm
        .set_provider(LlmProvider::Anthropic);
    assert_eq!(reloaded.translation.llm.api_key, "anthropic-key");
    assert_eq!(reloaded.translation.llm.temperature, 0.68);
}

#[test]
fn eztrans_process_count_defaults_and_is_clamped() {
    let partial = Config::from_toml_str("[translation]\nengine = \"eztrans\"\n").unwrap();
    assert_eq!(partial.translation.eztrans_process_count, 2);

    let mut raw = Config::default();
    raw.translation.eztrans_process_count = u32::MAX;
    let normalized = Config::from_toml_str(&toml::to_string(&raw).unwrap()).unwrap();
    assert_eq!(normalized.translation.eztrans_process_count, 16);

    raw.translation.eztrans_process_count = 0;
    let normalized = Config::from_toml_str(&toml::to_string(&raw).unwrap()).unwrap();
    assert_eq!(normalized.translation.eztrans_process_count, 1);
}

#[test]
fn eztrans_postprocess_dictionary_round_trips_separately_from_llm_glossary() {
    let mut config = Config::default();
    config.translation.eztrans_postprocess_dictionary =
        vec![crate::config::EzTransPostprocessEntry {
            source: "결과".into(),
            target: "후처리".into(),
        }];
    config.translation.llm.glossary = vec![crate::config::LlmGlossaryEntry {
        source: "prompt".into(),
        target: "프롬프트".into(),
    }];

    let serialized = toml::to_string_pretty(&config).unwrap();
    let reloaded = Config::from_toml_str(&serialized).unwrap();
    assert_eq!(
        reloaded.translation.eztrans_postprocess_dictionary[0].target,
        "후처리"
    );
    assert_eq!(reloaded.translation.llm.glossary[0].target, "프롬프트");
}
