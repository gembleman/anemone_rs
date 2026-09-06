use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 글로서리(고정 번역 사전) 한 항목
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LlmGlossaryEntry {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub target: String,
}

/// LLM 번역 설정 (OpenAI 호환 / Anthropic / Gemini 공통)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmConfig {
    /// 제공자: "openai", "anthropic", "gemini", "grok", "openrouter"
    #[serde(
        default = "default_llm_provider",
        deserialize_with = "deserialize_llm_provider"
    )]
    pub provider: String,
    /// 모델 ID (비우면 제공자 기본값 사용)
    #[serde(default)]
    pub model: String,
    /// API 키 (Anthropic은 x-api-key, 그 외는 Bearer)
    #[serde(default)]
    pub api_key: String,
    /// API 기본 URL. 설정 UI에는 노출하지 않으며 `config.toml`에서만 변경한다.
    /// 비우면 제공자의 공식 기본 URL을 사용한다.
    #[serde(default)]
    pub base_url: String,
    /// 시스템 프롬프트 (`{source}`, `{target}` 치환)
    #[serde(default = "default_llm_system_prompt")]
    pub system_prompt: String,
    /// 샘플링 온도
    #[serde(default = "default_llm_temperature")]
    pub temperature: f32,
    /// OpenRouter nucleus sampling 비율. 설정 UI에는 노출하지 않는다.
    #[serde(default = "default_llm_top_p")]
    pub top_p: f32,
    /// OpenRouter 토큰 빈도 페널티. 설정 UI에는 노출하지 않는다.
    #[serde(default = "default_llm_penalty")]
    pub frequency_penalty: f32,
    /// OpenRouter 토큰 출현 페널티. 설정 UI에는 노출하지 않는다.
    #[serde(default = "default_llm_penalty")]
    pub presence_penalty: f32,
    /// 응답 최대 토큰
    #[serde(default = "default_llm_max_tokens")]
    pub max_tokens: u32,
    /// OpenAI 추론 모델의 추론 강도. 비어 있으면 모델 기본값을 사용한다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<crate::translation::llm::ReasoningEffort>,
    /// 고정 번역 사전 (캐릭터 이름·고유명사 등)
    #[serde(default)]
    pub glossary: Vec<LlmGlossaryEntry>,
    /// 자동 클립보드 번역 디바운스 (ms). 0이면 비활성.
    /// 직전 요청과의 간격이 이 값 이하면 새 요청만 살아남고 이전 응답은 폐기된다.
    #[serde(default = "default_llm_debounce_ms")]
    pub debounce_ms: u32,
    /// 현재 선택되지 않은 제공자의 설정. 현재 제공자 설정은 기존 필드에 유지한다.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_profiles: BTreeMap<String, LlmProviderProfile>,
}

/// 제공자를 전환해도 보존해야 하는 LLM 설정 묶음.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmProviderProfile {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default = "default_llm_system_prompt")]
    pub system_prompt: String,
    #[serde(default = "default_llm_temperature")]
    pub temperature: f32,
    #[serde(default = "default_llm_top_p")]
    pub top_p: f32,
    #[serde(default = "default_llm_penalty")]
    pub frequency_penalty: f32,
    #[serde(default = "default_llm_penalty")]
    pub presence_penalty: f32,
    #[serde(default = "default_llm_max_tokens")]
    pub max_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<crate::translation::llm::ReasoningEffort>,
    #[serde(default)]
    pub glossary: Vec<LlmGlossaryEntry>,
    #[serde(default = "default_llm_debounce_ms")]
    pub debounce_ms: u32,
}

fn default_llm_provider() -> String {
    "openai".to_string()
}

fn deserialize_llm_provider<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = String::deserialize(deserializer)?;
    value
        .parse::<crate::translation::LlmProvider>()
        .map_err(D::Error::custom)?;
    Ok(value)
}

fn default_llm_system_prompt() -> String {
    crate::translation::llm::DEFAULT_SYSTEM_PROMPT.to_string()
}

fn default_llm_temperature() -> f32 {
    crate::config::limits::LLM_TEMPERATURE_DEFAULT
}

fn default_llm_top_p() -> f32 {
    1.0
}

fn default_llm_penalty() -> f32 {
    0.0
}

fn default_llm_max_tokens() -> u32 {
    1024
}

fn default_llm_debounce_ms() -> u32 {
    300
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: default_llm_provider(),
            model: String::new(),
            api_key: String::new(),
            base_url: String::new(),
            system_prompt: default_llm_system_prompt(),
            temperature: default_llm_temperature(),
            top_p: default_llm_top_p(),
            frequency_penalty: default_llm_penalty(),
            presence_penalty: default_llm_penalty(),
            max_tokens: default_llm_max_tokens(),
            reasoning_effort: None,
            glossary: Vec::new(),
            debounce_ms: default_llm_debounce_ms(),
            provider_profiles: BTreeMap::new(),
        }
    }
}

impl Default for LlmProviderProfile {
    fn default() -> Self {
        Self {
            model: String::new(),
            api_key: String::new(),
            base_url: String::new(),
            system_prompt: default_llm_system_prompt(),
            temperature: default_llm_temperature(),
            top_p: default_llm_top_p(),
            frequency_penalty: default_llm_penalty(),
            presence_penalty: default_llm_penalty(),
            max_tokens: default_llm_max_tokens(),
            reasoning_effort: None,
            glossary: Vec::new(),
            debounce_ms: default_llm_debounce_ms(),
        }
    }
}

impl LlmConfig {
    pub fn get_provider(
        &self,
    ) -> Result<crate::translation::LlmProvider, crate::translation::EnumParseError> {
        self.provider.parse()
    }

    pub fn set_provider(&mut self, provider: crate::translation::LlmProvider) {
        let provider_name = provider.to_str();
        let current_provider = self.get_provider().ok();
        if current_provider == Some(provider) {
            self.provider = provider_name.to_string();
            return;
        }

        if let Some(current_provider) = current_provider {
            self.provider_profiles
                .insert(current_provider.to_str().to_string(), self.active_profile());
        }
        let profile = self
            .provider_profiles
            .remove(provider_name)
            .unwrap_or_default();
        self.provider = provider.to_str().to_string();
        self.apply_profile(profile);
    }

    fn active_profile(&self) -> LlmProviderProfile {
        LlmProviderProfile {
            model: self.model.clone(),
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            system_prompt: self.system_prompt.clone(),
            temperature: self.temperature,
            top_p: self.top_p,
            frequency_penalty: self.frequency_penalty,
            presence_penalty: self.presence_penalty,
            max_tokens: self.max_tokens,
            reasoning_effort: self.reasoning_effort,
            glossary: self.glossary.clone(),
            debounce_ms: self.debounce_ms,
        }
    }

    fn apply_profile(&mut self, profile: LlmProviderProfile) {
        self.model = profile.model;
        self.api_key = profile.api_key;
        self.base_url = profile.base_url;
        self.system_prompt = profile.system_prompt;
        self.temperature = profile.temperature;
        self.top_p = profile.top_p;
        self.frequency_penalty = profile.frequency_penalty;
        self.presence_penalty = profile.presence_penalty;
        self.max_tokens = profile.max_tokens;
        self.reasoning_effort = profile.reasoning_effort;
        self.glossary = profile.glossary;
        self.debounce_ms = profile.debounce_ms;
    }

    pub fn normalize(&mut self) {
        self.max_tokens = crate::config::limits::llm_max_tokens(self.max_tokens);
        self.debounce_ms = crate::config::limits::llm_debounce_ms(self.debounce_ms);
        self.temperature = crate::config::limits::llm_temperature(self.temperature);
        self.top_p = crate::config::limits::llm_top_p(self.top_p);
        self.frequency_penalty = crate::config::limits::llm_penalty(self.frequency_penalty);
        self.presence_penalty = crate::config::limits::llm_penalty(self.presence_penalty);
        for profile in self.provider_profiles.values_mut() {
            normalize_profile(profile);
        }
    }

    /// 워커에 전달할 호출 파라미터 빌드
    pub fn to_call_params(
        &self,
    ) -> Result<crate::translation::llm::LlmCallParams, crate::translation::EnumParseError> {
        Ok(crate::translation::llm::LlmCallParams {
            provider: self.get_provider()?,
            model: self.model.clone(),
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            system_prompt: self.system_prompt.clone(),
            temperature: self.temperature,
            top_p: self.top_p,
            frequency_penalty: self.frequency_penalty,
            presence_penalty: self.presence_penalty,
            max_tokens: self.max_tokens,
            reasoning_effort: self.reasoning_effort,
            glossary: self
                .glossary
                .iter()
                .map(|e| crate::translation::llm::GlossaryEntry {
                    source: e.source.clone(),
                    target: e.target.clone(),
                })
                .collect(),
        })
    }
}

fn normalize_profile(profile: &mut LlmProviderProfile) {
    profile.max_tokens = crate::config::limits::llm_max_tokens(profile.max_tokens);
    profile.debounce_ms = crate::config::limits::llm_debounce_ms(profile.debounce_ms);
    profile.temperature = crate::config::limits::llm_temperature(profile.temperature);
    profile.top_p = crate::config::limits::llm_top_p(profile.top_p);
    profile.frequency_penalty = crate::config::limits::llm_penalty(profile.frequency_penalty);
    profile.presence_penalty = crate::config::limits::llm_penalty(profile.presence_penalty);
}
