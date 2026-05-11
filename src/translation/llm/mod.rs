//! LLM 기반 번역 엔진
//!
//! 백엔드별 모듈:
//! - `openai_compat`: OpenAI 호환 chat completions (OpenAI, Grok, OpenRouter)
//! - `anthropic`: Anthropic Claude messages API
//! - `gemini`: Google AI Studio generateContent

pub mod anthropic;
pub mod gemini;
pub mod openai_compat;

use isolang::Language;

use super::lang_utils;

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
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::OpenAi,
            1 => Self::Anthropic,
            2 => Self::Gemini,
            3 => Self::Grok,
            4 => Self::OpenRouter,
            _ => Self::OpenAi,
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "openai" => Self::OpenAi,
            "anthropic" | "claude" => Self::Anthropic,
            "gemini" | "google_ai" => Self::Gemini,
            "grok" | "xai" => Self::Grok,
            "openrouter" => Self::OpenRouter,
            _ => Self::OpenAi,
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

    pub fn display_name(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Gemini => "Gemini",
            Self::Grok => "xAI Grok",
            Self::OpenRouter => "OpenRouter",
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

    /// OpenAI 호환 chat completions API를 쓰는 제공자인지
    pub fn is_openai_compatible(self) -> bool {
        matches!(self, Self::OpenAi | Self::Grok | Self::OpenRouter)
    }
}

/// 글로서리 한 항목 (캐릭터 이름/고유명사 고정 번역)
#[derive(Clone, Debug, Default)]
pub struct GlossaryEntry {
    pub source: String,
    pub target: String,
}

/// LLM 호출 시 사용할 파라미터 묶음
#[derive(Clone, Debug)]
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
}

/// 시스템 프롬프트의 `{source}` / `{target}` 치환 + 글로서리 부착
pub fn build_system_prompt_with_glossary(
    template: &str,
    source: Language,
    target: Language,
    glossary: &[GlossaryEntry],
) -> String {
    let mut s = template
        .replace("{source}", lang_utils::to_korean_name(source))
        .replace("{target}", lang_utils::to_korean_name(target));

    let active: Vec<&GlossaryEntry> = glossary
        .iter()
        .filter(|e| !e.source.is_empty() && !e.target.is_empty())
        .collect();

    if !active.is_empty() {
        use std::fmt::Write;
        s.push_str("\n\n[고정 번역 사전 — 반드시 이대로 옮길 것]");
        for e in active {
            let _ = write!(s, "\n- {} → {}", e.source, e.target);
        }
    }
    s
}

/// 기본 시스템 프롬프트 — 사용자가 비워두면 이 값을 사용한다
pub const DEFAULT_SYSTEM_PROMPT: &str = "당신은 게임/소설 텍스트 번역기입니다. {source}를 {target}로 번역하세요.\n- 캐릭터 이름과 고유명사는 자연스럽게 음차하거나 유지하세요.\n- 한국 사용자에게 자연스러운 어투를 사용하세요.\n- 의역을 허용합니다.\n- 번역 결과 텍스트만 출력하세요. 설명·접두어·따옴표 없이.";
