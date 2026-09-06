use super::error::TranslationError;
use thiserror::Error;

pub mod lang_utils;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("알 수 없는 {kind} 값: {value}")]
pub struct EnumParseError {
    kind: &'static str,
    value: String,
}

impl EnumParseError {
    pub(crate) fn new(kind: &'static str, value: &str) -> Self {
        Self {
            kind,
            value: value.to_string(),
        }
    }
}

/// 번역 API에 전달할 언어 태그.
///
/// 일반 ISO 언어 열거형과 달리 중국어 문자 체계를 보존한다. 설정/CLI/UI에서 선택한
/// `zh-CN`과 `zh-TW`가 제공자 어댑터까지 손실 없이 전달된다.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Language {
    Jpn,
    Kor,
    Eng,
    ZhoHans,
    ZhoHant,
    Spa,
    Fra,
    Deu,
    Ita,
    Por,
    Rus,
    Ara,
    Hin,
    Tha,
    Vie,
    Ind,
    Msa,
    Nld,
    Pol,
    Tur,
    Ukr,
}

/// 번역 엔진 종류
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TranslationEngine {
    #[default]
    EzTrans = 0,
    Google = 1,
    DeepL = 2,
    Papago = 3,
    /// LLM 기반 번역 (제공자/모델은 LlmConfig에서 지정)
    Llm = 4,
    /// 자체 호스팅 translate_server 클라이언트
    MysTranslater = 5,
    /// 사용자 정의 JSON REST API
    Custom = 6,
}

impl TranslationEngine {
    pub const ALL: [Self; 7] = [
        Self::EzTrans,
        Self::Google,
        Self::DeepL,
        Self::Papago,
        Self::Llm,
        Self::MysTranslater,
        Self::Custom,
    ];

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::EzTrans => "EzTrans64",
            Self::Google => "Google",
            Self::DeepL => "DeepL",
            Self::Papago => "Papago API",
            Self::Llm => "LLM",
            Self::MysTranslater => "MyS Translater",
            Self::Custom => "Custom API",
        }
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::EzTrans),
            1 => Some(Self::Google),
            2 => Some(Self::DeepL),
            3 => Some(Self::Papago),
            4 => Some(Self::Llm),
            5 => Some(Self::MysTranslater),
            6 => Some(Self::Custom),
            _ => None,
        }
    }

    pub fn to_str(self) -> &'static str {
        match self {
            Self::EzTrans => "eztrans",
            Self::Google => "google",
            Self::DeepL => "deepl",
            Self::Papago => "papago",
            Self::Llm => "llm",
            Self::MysTranslater => "mys_translater",
            Self::Custom => "custom",
        }
    }

    /// 해당 엔진이 지원하는 소스 언어 목록
    pub fn supported_source_languages(&self) -> &'static [Language] {
        match self {
            // EzTrans와 마찬가지로 번역 서버 엔진도 일본어 소스만 지원한다.
            Self::EzTrans | Self::MysTranslater => &[Language::Jpn],
            Self::Google => super::GOOGLE_SUPPORTED_LANGUAGES,
            Self::DeepL => super::DEEPL_SUPPORTED_LANGUAGES,
            Self::Papago => super::PAPAGO_SUPPORTED_LANGUAGES,
            Self::Llm | Self::Custom => super::GOOGLE_SUPPORTED_LANGUAGES,
        }
    }

    /// 해당 엔진이 지원하는 타겟 언어 목록
    pub fn supported_target_languages(&self) -> &'static [Language] {
        match self {
            // 번역 서버 엔진은 일본어→한국어 방향만 허용된다.
            Self::EzTrans | Self::MysTranslater => &[Language::Kor],
            Self::Google => super::GOOGLE_SUPPORTED_LANGUAGES,
            Self::DeepL => super::DEEPL_SUPPORTED_LANGUAGES,
            Self::Papago => super::PAPAGO_SUPPORTED_LANGUAGES,
            Self::Llm | Self::Custom => super::GOOGLE_SUPPORTED_LANGUAGES,
        }
    }

    /// 제공자 계약상 실제로 번역 가능한 언어 방향인지 검사한다.
    pub fn supports_pair(&self, source: Language, target: Language) -> bool {
        if source == target
            || !self.supported_source_languages().contains(&source)
            || !self.supported_target_languages().contains(&target)
        {
            return false;
        }

        match self {
            Self::Papago => papago_supports_pair(source, target),
            _ => true,
        }
    }

    /// 단일 요청의 보수적인 문자 수 상한. 긴 파일은 호출자가 이 경계로 나눠야 한다.
    pub fn max_input_chars(self) -> usize {
        match self {
            Self::Google | Self::Papago => 5_000,
            Self::DeepL | Self::Llm | Self::Custom | Self::EzTrans | Self::MysTranslater => 100_000,
        }
    }

    pub fn supported_targets_for(self, source: Language) -> Vec<Language> {
        self.supported_target_languages()
            .iter()
            .copied()
            .filter(|&target| self.supports_pair(source, target))
            .collect()
    }
}

impl std::str::FromStr for TranslationEngine {
    type Err = EnumParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "eztrans" => Ok(Self::EzTrans),
            "google" => Ok(Self::Google),
            "deepl" => Ok(Self::DeepL),
            "papago" => Ok(Self::Papago),
            "llm" => Ok(Self::Llm),
            "mys_translater" => Ok(Self::MysTranslater),
            "custom" => Ok(Self::Custom),
            _ => Err(EnumParseError::new("번역 엔진", value)),
        }
    }
}

/// Ncloud Papago의 양방향 지원 언어쌍.
fn papago_supports_pair(source: Language, target: Language) -> bool {
    let paired = |anchor, others: &[Language]| {
        (source == anchor && others.contains(&target))
            || (target == anchor && others.contains(&source))
    };

    paired(
        Language::Kor,
        &[
            Language::Eng,
            Language::Jpn,
            Language::ZhoHans,
            Language::ZhoHant,
            Language::Vie,
            Language::Tha,
            Language::Ind,
            Language::Fra,
            Language::Spa,
            Language::Rus,
            Language::Deu,
            Language::Ita,
        ],
    ) || paired(
        Language::Eng,
        &[
            Language::Jpn,
            Language::ZhoHans,
            Language::ZhoHant,
            Language::Vie,
            Language::Tha,
            Language::Ind,
            Language::Fra,
            Language::Spa,
            Language::Rus,
            Language::Deu,
        ],
    ) || paired(
        Language::Jpn,
        &[
            Language::ZhoHans,
            Language::ZhoHant,
            Language::Vie,
            Language::Tha,
            Language::Ind,
            Language::Fra,
        ],
    ) || matches!(
        (source, target),
        (Language::ZhoHans, Language::ZhoHant) | (Language::ZhoHant, Language::ZhoHans)
    )
}

/// Google Translate 지원 언어 (주요 언어)
pub static GOOGLE_SUPPORTED_LANGUAGES: &[Language] = &[
    Language::Jpn,
    Language::Kor,
    Language::Eng,
    Language::ZhoHans,
    Language::ZhoHant,
    Language::Spa,
    Language::Fra,
    Language::Deu,
    Language::Ita,
    Language::Por,
    Language::Rus,
    Language::Ara,
    Language::Hin,
    Language::Tha,
    Language::Vie,
    Language::Ind,
    Language::Msa,
    Language::Nld,
    Language::Pol,
    Language::Tur,
    Language::Ukr,
];

/// DeepL 지원 언어
pub static DEEPL_SUPPORTED_LANGUAGES: &[Language] = &[
    Language::Jpn,
    Language::Kor,
    Language::Eng,
    Language::ZhoHans,
    Language::ZhoHant,
    Language::Spa,
    Language::Fra,
    Language::Deu,
    Language::Ita,
    Language::Por,
    Language::Rus,
    Language::Nld,
    Language::Pol,
    Language::Tur,
    Language::Ukr,
];

/// Ncloud Papago Text Translation 지원 언어
pub static PAPAGO_SUPPORTED_LANGUAGES: &[Language] = &[
    Language::Kor,
    Language::Eng,
    Language::Jpn,
    Language::ZhoHans,
    Language::ZhoHant,
    Language::Vie,
    Language::Tha,
    Language::Ind,
    Language::Fra,
    Language::Spa,
    Language::Rus,
    Language::Deu,
    Language::Ita,
];

#[cfg(test)]
#[path = "../../../tests/unit/translation/language.rs"]
mod tests;
