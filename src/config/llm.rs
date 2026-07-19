use serde::{Deserialize, Serialize};

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
    0.3
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
        self.provider = provider.to_str().to_string();
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
