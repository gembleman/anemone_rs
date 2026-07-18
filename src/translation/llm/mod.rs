//! OpenAI 호환, Anthropic, Gemini 기반 LLM 번역 engine.

pub mod anthropic;
pub mod gemini;
pub mod openai_compat;
pub mod usage;

use super::{EnumParseError, Language, lang_utils};
use std::fmt;

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
            Self::OpenAi => "gpt-5",
            Self::Anthropic => "claude-opus-4-7",
            Self::Gemini => "gemini-2.5-flash",
            Self::Grok => "grok-3",
            Self::OpenRouter => "openai/gpt-5",
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
    pub max_tokens: u32,
    /// 고정 번역 사전 (캐릭터 이름 등). 비어 있으면 프롬프트에 추가되지 않음.
    pub glossary: Vec<GlossaryEntry>,
}

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
            .field("max_tokens", &self.max_tokens)
            .field("glossary", &self.glossary)
            .finish()
    }
}

impl LlmCallParams {
    pub fn effective_model(&self) -> &str {
        if self.model.is_empty() {
            self.provider.default_model()
        } else {
            self.model.as_str()
        }
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
        lang_utils::to_korean_name(source),
        lang_utils::to_korean_name(target),
    );

    let mut active = glossary
        .iter()
        .filter(|e| !e.source.is_empty() && !e.target.is_empty())
        .peekable();

    if active.peek().is_some() {
        use std::fmt::Write;
        s.push_str("\n\n[고정 번역 사전 — 반드시 이대로 옮길 것]");
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
pub const DEFAULT_SYSTEM_PROMPT: &str = "당신은 게임/소설 텍스트 번역기입니다. {source}를 {target}로 번역하세요.\n- 캐릭터 이름과 고유명사는 자연스럽게 음차하거나 유지하세요.\n- 한국 사용자에게 자연스러운 어투를 사용하세요.\n- 의역을 허용합니다.\n- 번역 결과 텍스트만 출력하세요. 설명·접두어·따옴표 없이.";

#[cfg(test)]
#[path = "../../../tests/unit/translation/llm/mod.rs"]
mod tests;
