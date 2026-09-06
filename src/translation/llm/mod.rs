//! OpenAI 호환, Anthropic, Gemini 기반 LLM 번역 engine.

pub mod anthropic;
pub mod gemini;
pub mod openai_compat;
pub mod usage;

use super::{EnumParseError, Language, lang_utils};
use serde::{Deserialize, Serialize};
use std::fmt;

/// OpenAI 추론 모델의 추론 강도.
///
/// 모델마다 지원하는 값의 부분집합이 다르므로 여기서는 API 전체 열거형을
/// 표현하고, 구체적인 모델 조합의 검증은 OpenAI API에 맡긴다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl ReasoningEffort {
    pub const ALL: &'static [Self] = &[
        Self::None,
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Xhigh,
        Self::Max,
    ];

    pub const fn to_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

/// LLM 제공자
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LlmProvider {
    #[default]
    OpenAi = 0,
    Anthropic = 1,
    Gemini = 2,
    Grok = 3,
    OpenRouter = 4,
}

impl LlmProvider {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::OpenAi),
            1 => Some(Self::Anthropic),
            2 => Some(Self::Gemini),
            3 => Some(Self::Grok),
            4 => Some(Self::OpenRouter),
            _ => None,
        }
    }

    pub fn to_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Grok => "grok",
            Self::OpenRouter => "openrouter",
        }
    }

    /// 모델 ID가 비어 있을 때 사용할 기본값
    pub fn default_model(self) -> &'static str {
        match self {
            Self::OpenAi => "gpt-5.4-nano",
            Self::Anthropic => "claude-opus-4-7",
            Self::Gemini => "gemini-2.5-flash",
            Self::Grok => "grok-4.3",
            Self::OpenRouter => "",
        }
    }

    /// 설정값이 비어 있을 때 UI와 API 호출에서 공통으로 사용할 모델 ID.
    pub fn model_or_default(self, configured_model: &str) -> &str {
        if configured_model.is_empty() {
            self.default_model()
        } else {
            configured_model
        }
    }

    /// 설정 UI에 제안할 모델 ID 목록.
    ///
    /// 이 목록은 선택을 돕는 프리셋일 뿐 허용 목록이 아니다. 계정별 모델 접근
    /// 권한이나 이후 출시 모델은 다를 수 있으므로 UI와 `config.toml` 모두 임의의
    /// 모델 ID를 계속 허용한다.
    pub fn model_presets(self) -> &'static [&'static str] {
        match self {
            Self::OpenAi => OPENAI_MODEL_PRESETS,
            Self::Anthropic => ANTHROPIC_MESSAGES_MODELS,
            Self::Gemini => GEMINI_GENERATE_CONTENT_MODELS,
            Self::Grok => GROK_CHAT_COMPLETION_MODELS,
            Self::OpenRouter => &[],
        }
    }

    /// 기본 base_url (사용자가 비워두면 사용)
    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            Self::Grok => "https://api.x.ai/v1",
            Self::OpenRouter => "https://openrouter.ai/api/v1",
        }
    }

    /// 사용자에게 보여줄 표시 이름. UI 콤보 항목과 라벨에 공통 사용한다.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI API",
            Self::Anthropic => "Claude API",
            Self::Gemini => "AI Studio",
            Self::Grok => "Grok",
            Self::OpenRouter => "OpenRouter",
        }
    }

    /// UI 표시 순서로 나열한 전체 제공자
    pub const ALL: &'static [LlmProvider] = &[
        Self::OpenAi,
        Self::Anthropic,
        Self::Gemini,
        Self::Grok,
        Self::OpenRouter,
    ];
}

impl std::str::FromStr for LlmProvider {
    type Err = EnumParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "openai" => Ok(Self::OpenAi),
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "gemini" | "google_ai" => Ok(Self::Gemini),
            "grok" | "xai" => Ok(Self::Grok),
            "openrouter" => Ok(Self::OpenRouter),
            _ => Err(EnumParseError::new("LLM 제공자", value)),
        }
    }
}

/// 글로서리 한 항목 (캐릭터 이름/고유명사 고정 번역)
#[derive(Clone, Debug, Default)]
pub struct GlossaryEntry {
    pub source: String,
    pub target: String,
}

/// LLM 호출 시 사용할 파라미터 묶음
#[derive(Clone)]
pub struct LlmCallParams {
    pub provider: LlmProvider,
    pub model: String,
    pub api_key: String,
    /// 비어 있으면 `provider.default_base_url()` 사용
    pub base_url: String,
    /// `{source}`, `{target}` 플레이스홀더가 치환됨
    pub system_prompt: String,
    pub temperature: f32,
    pub top_p: f32,
    pub frequency_penalty: f32,
    pub presence_penalty: f32,
    pub max_tokens: u32,
    /// `None`이면 모델의 기본 추론 강도를 사용한다.
    pub reasoning_effort: Option<ReasoningEffort>,
    /// 고정 번역 사전 (캐릭터 이름 등). 비어 있으면 프롬프트에 추가되지 않음.
    pub glossary: Vec<GlossaryEntry>,
}

/// OpenAI 텍스트 모델 선택 UI에 표시할 프리셋.
///
/// 실제 허용 목록은 아니며, 각 모델의 Responses API 지원 여부와 계정 권한은
/// OpenAI API가 최종 검증한다.
pub const OPENAI_MODEL_PRESETS: &[&str] = &[
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.4-nano",
    "gpt-5.4-mini-2026-03-17",
    "gpt-5.4-nano-2026-03-17",
    "gpt-5.3-chat-latest",
    "gpt-5.2",
    "gpt-5.2-2025-12-11",
    "gpt-5.2-chat-latest",
    "gpt-5.2-pro",
    "gpt-5.2-pro-2025-12-11",
    "gpt-5.1",
    "gpt-5.1-2025-11-13",
    "gpt-5.1-codex",
    "gpt-5.1-mini",
    "gpt-5.1-chat-latest",
    "gpt-5",
    "gpt-5-mini",
    "gpt-5-nano",
    "gpt-5-2025-08-07",
    "gpt-5-mini-2025-08-07",
    "gpt-5-nano-2025-08-07",
    "gpt-5-chat-latest",
    "gpt-4.1",
    "gpt-4.1-mini",
    "gpt-4.1-nano",
    "gpt-4.1-2025-04-14",
    "gpt-4.1-mini-2025-04-14",
    "gpt-4.1-nano-2025-04-14",
    "o4-mini",
    "o4-mini-2025-04-16",
    "o3",
    "o3-2025-04-16",
    "o3-mini",
    "o3-mini-2025-01-31",
    "o1",
    "o1-2024-12-17",
    "o1-preview",
    "o1-preview-2024-09-12",
    "o1-mini",
    "o1-mini-2024-09-12",
    "gpt-4o",
    "gpt-4o-2024-11-20",
    "gpt-4o-2024-08-06",
    "gpt-4o-2024-05-13",
    "gpt-4o-search-preview",
    "gpt-4o-mini-search-preview",
    "gpt-4o-search-preview-2025-03-11",
    "gpt-4o-mini-search-preview-2025-03-11",
    "chatgpt-4o-latest",
    "codex-mini-latest",
    "gpt-4o-mini",
    "gpt-4o-mini-2024-07-18",
    "gpt-4-turbo",
    "gpt-4-turbo-2024-04-09",
    "gpt-4-0125-preview",
    "gpt-4-turbo-preview",
    "gpt-4-1106-preview",
    "gpt-4",
    "gpt-4-0314",
    "gpt-4-0613",
    "gpt-4-32k",
    "gpt-4-32k-0314",
    "gpt-4-32k-0613",
    "gpt-3.5-turbo",
    "gpt-3.5-turbo-16k",
    "gpt-3.5-turbo-0301",
    "gpt-3.5-turbo-0613",
    "gpt-3.5-turbo-1106",
    "gpt-3.5-turbo-0125",
    "gpt-3.5-turbo-16k-0613",
];

/// Anthropic Messages API에서 사용할 수 있는 현재 공개 Claude 모델.
/// 4.5 모델은 고정 스냅샷과 Claude API 편의 별칭을 함께 제공한다.
pub const ANTHROPIC_MESSAGES_MODELS: &[&str] = &[
    "claude-fable-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-opus-4-5",
    "claude-opus-4-5-20251101",
    "claude-sonnet-5",
    "claude-sonnet-4-6",
    "claude-sonnet-4-5",
    "claude-sonnet-4-5-20250929",
    "claude-haiku-4-5",
    "claude-haiku-4-5-20251001",
];

/// Gemini API `generateContent`를 지원하는 범용 텍스트 생성 모델.
pub const GEMINI_GENERATE_CONTENT_MODELS: &[&str] = &[
    "gemini-3.5-flash",
    "gemini-3.1-pro-preview",
    "gemini-3.1-flash-lite",
    "gemini-3-flash-preview",
    "gemini-2.5-pro",
    "gemini-2.5-flash",
    "gemini-2.5-flash-lite",
];

/// xAI Chat Completions에서 사용할 수 있는 현재 텍스트 모델.
pub const GROK_CHAT_COMPLETION_MODELS: &[&str] = &[
    "grok-4.5",
    "grok-4.5-latest",
    "grok-build-0.1",
    "grok-build-latest",
    "grok-4.3",
    "grok-4.20-multi-agent-0309",
    "grok-4.20-0309-reasoning",
    "grok-4.20-0309-non-reasoning",
];

impl fmt::Debug for LlmCallParams {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LlmCallParams")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("api_key", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .field("system_prompt", &self.system_prompt)
            .field("temperature", &self.temperature)
            .field("top_p", &self.top_p)
            .field("frequency_penalty", &self.frequency_penalty)
            .field("presence_penalty", &self.presence_penalty)
            .field("max_tokens", &self.max_tokens)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("glossary", &self.glossary)
            .finish()
    }
}

impl LlmCallParams {
    pub fn effective_model(&self) -> &str {
        self.provider.model_or_default(&self.model)
    }

    pub fn effective_base_url(&self) -> &str {
        let trimmed = self.base_url.trim_end_matches('/');
        if trimmed.is_empty() {
            self.provider.default_base_url()
        } else {
            trimmed
        }
    }

    pub fn effective_system_prompt(&self) -> &str {
        if self.system_prompt.trim().is_empty() {
            DEFAULT_SYSTEM_PROMPT
        } else {
            self.system_prompt.as_str()
        }
    }
}

/// 시스템 프롬프트의 `{source}` / `{target}` 치환 + 글로서리 부착
pub fn build_system_prompt_with_glossary(
    template: &str,
    source: Language,
    target: Language,
    glossary: &[GlossaryEntry],
) -> String {
    let template = if template.trim().is_empty() {
        DEFAULT_SYSTEM_PROMPT
    } else {
        template
    };
    let mut s = expand_language_placeholders(
        template,
        lang_utils::to_english_name(source),
        lang_utils::to_english_name(target),
    );

    let mut active = glossary
        .iter()
        .filter(|e| !e.source.is_empty() && !e.target.is_empty())
        .peekable();

    if active.peek().is_some() {
        use std::fmt::Write;
        s.push_str("\n\n[Glossary — always translate these exactly as listed]");
        for e in active {
            let _ = write!(s, "\n- {} → {}", e.source, e.target);
        }
    }
    s
}

/// 두 언어 placeholder를 한 번 순회해 확장한다.
fn expand_language_placeholders(template: &str, source: &str, target: &str) -> String {
    const SOURCE: &str = "{source}";
    const TARGET: &str = "{target}";

    let mut output = String::with_capacity(template.len());
    let mut rest = template;
    loop {
        let source_at = rest.find(SOURCE);
        let target_at = rest.find(TARGET);
        let next = match (source_at, target_at) {
            (Some(source_at), Some(target_at)) if source_at <= target_at => {
                Some((source_at, SOURCE, source))
            }
            (Some(_), Some(target_at)) => Some((target_at, TARGET, target)),
            (Some(source_at), None) => Some((source_at, SOURCE, source)),
            (None, Some(target_at)) => Some((target_at, TARGET, target)),
            (None, None) => None,
        };
        let Some((at, token, replacement)) = next else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..at]);
        output.push_str(replacement);
        rest = &rest[at + token.len()..];
    }
    output
}

/// 기본 시스템 프롬프트 — 사용자가 비워두면 이 값을 사용한다
pub const DEFAULT_SYSTEM_PROMPT: &str = "You are a translator for game and novel text. Translate {source} into {target}.\n- Transliterate or keep character names and proper nouns, whichever reads more naturally.\n- Write in a tone that sounds natural to native {target} readers.\n- Liberal translation is allowed.\n- Output only the translated text. No explanations, prefixes, or quotation marks.";

#[cfg(test)]
#[path = "../../../tests/unit/translation/llm/mod.rs"]
mod tests;
