use super::error::TranslationError;
use thiserror::Error;

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
    /// 사용자 정의 JSON REST API
    Custom = 5,
}

impl TranslationEngine {
    pub const ALL: [Self; 6] = [
        Self::EzTrans,
        Self::Google,
        Self::DeepL,
        Self::Papago,
        Self::Llm,
        Self::Custom,
    ];

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::EzTrans => "EzTrans",
            Self::Google => "Google",
            Self::DeepL => "DeepL",
            Self::Papago => "Papago API",
            Self::Llm => "LLM",
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
            5 => Some(Self::Custom),
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
            Self::Custom => "custom",
        }
    }

    /// 해당 엔진이 지원하는 소스 언어 목록
    pub fn supported_source_languages(&self) -> &'static [Language] {
        match self {
            Self::EzTrans => &[Language::Jpn],
            Self::Google => super::GOOGLE_SUPPORTED_LANGUAGES,
            Self::DeepL => super::DEEPL_SUPPORTED_LANGUAGES,
            Self::Papago => super::PAPAGO_SUPPORTED_LANGUAGES,
            Self::Llm | Self::Custom => super::GOOGLE_SUPPORTED_LANGUAGES,
        }
    }

    /// 해당 엔진이 지원하는 타겟 언어 목록
    pub fn supported_target_languages(&self) -> &'static [Language] {
        match self {
            Self::EzTrans => &[Language::Kor],
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
            Self::DeepL | Self::Llm | Self::Custom | Self::EzTrans => 100_000,
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

/// 언어 코드 헬퍼 함수들
pub mod lang_utils {
    use super::{Language, TranslationError};

    /// ISO 639-1 코드로 Language 가져오기 (예: "ja", "ko", "en")
    pub fn from_code(code: &str) -> Option<Language> {
        match code.to_lowercase().as_str() {
            "ja" | "jpn" => Some(Language::Jpn),
            "ko" | "kor" => Some(Language::Kor),
            "en" | "eng" => Some(Language::Eng),
            "zh" | "zho" | "chi" | "zh-cn" | "zh-hans" | "zhs" => Some(Language::ZhoHans),
            "zh-tw" | "zh-hant" | "zht" => Some(Language::ZhoHant),
            "es" | "spa" => Some(Language::Spa),
            "fr" | "fra" | "fre" => Some(Language::Fra),
            "de" | "deu" | "ger" => Some(Language::Deu),
            "it" | "ita" => Some(Language::Ita),
            "pt" | "por" => Some(Language::Por),
            "ru" | "rus" => Some(Language::Rus),
            "ar" | "ara" => Some(Language::Ara),
            "hi" | "hin" => Some(Language::Hin),
            "th" | "tha" => Some(Language::Tha),
            "vi" | "vie" => Some(Language::Vie),
            "id" | "ind" => Some(Language::Ind),
            "ms" | "msa" | "may" => Some(Language::Msa),
            "nl" | "nld" | "dut" => Some(Language::Nld),
            "pl" | "pol" => Some(Language::Pol),
            "tr" | "tur" => Some(Language::Tur),
            "uk" | "ukr" => Some(Language::Ukr),
            _ => None,
        }
    }

    /// Language를 ISO 639-1 코드로 변환
    pub fn to_code(lang: Language) -> &'static str {
        match lang {
            Language::Jpn => "ja",
            Language::Kor => "ko",
            Language::Eng => "en",
            Language::ZhoHans => "zh-CN",
            Language::ZhoHant => "zh-TW",
            Language::Spa => "es",
            Language::Fra => "fr",
            Language::Deu => "de",
            Language::Ita => "it",
            Language::Por => "pt",
            Language::Rus => "ru",
            Language::Ara => "ar",
            Language::Hin => "hi",
            Language::Tha => "th",
            Language::Vie => "vi",
            Language::Ind => "id",
            Language::Msa => "ms",
            Language::Nld => "nl",
            Language::Pol => "pl",
            Language::Tur => "tr",
            Language::Ukr => "uk",
        }
    }

    /// Language를 한국어 이름으로 변환
    pub fn to_korean_name(lang: Language) -> &'static str {
        match lang {
            Language::Jpn => "일본어",
            Language::Kor => "한국어",
            Language::Eng => "영어",
            Language::ZhoHans => "중국어(간체)",
            Language::ZhoHant => "중국어(번체)",
            Language::Spa => "스페인어",
            Language::Fra => "프랑스어",
            Language::Deu => "독일어",
            Language::Ita => "이탈리아어",
            Language::Por => "포르투갈어",
            Language::Rus => "러시아어",
            Language::Ara => "아랍어",
            Language::Hin => "힌디어",
            Language::Tha => "태국어",
            Language::Vie => "베트남어",
            Language::Ind => "인도네시아어",
            Language::Msa => "말레이어",
            Language::Nld => "네덜란드어",
            Language::Pol => "폴란드어",
            Language::Tur => "터키어",
            Language::Ukr => "우크라이나어",
        }
    }

    /// Google Translate API 언어 코드로 변환
    pub fn to_google_code(lang: Language) -> Result<&'static str, TranslationError> {
        Ok(match lang {
            Language::ZhoHans => "zh-CN",
            Language::ZhoHant => "zh-TW",
            _ => to_code(lang),
        })
    }

    /// DeepL API 언어 코드로 변환
    pub fn to_deepl_code(lang: Language) -> Result<&'static str, TranslationError> {
        match lang {
            Language::Jpn => Ok("JA"),
            Language::Kor => Ok("KO"),
            Language::Eng => Ok("EN"),
            Language::ZhoHans => Ok("ZH-HANS"),
            Language::ZhoHant => Ok("ZH-HANT"),
            Language::Spa => Ok("ES"),
            Language::Fra => Ok("FR"),
            Language::Deu => Ok("DE"),
            Language::Ita => Ok("IT"),
            Language::Por => Ok("PT"),
            Language::Rus => Ok("RU"),
            Language::Nld => Ok("NL"),
            Language::Pol => Ok("PL"),
            Language::Tur => Ok("TR"),
            Language::Ukr => Ok("UK"),
            Language::Ara
            | Language::Hin
            | Language::Tha
            | Language::Vie
            | Language::Ind
            | Language::Msa => Err(TranslationError::UnsupportedLanguage {
                engine: "DeepL",
                language: lang,
            }),
        }
    }

    /// Papago API 언어 코드로 변환
    pub fn to_papago_code(lang: Language) -> Result<&'static str, TranslationError> {
        match lang {
            Language::Kor => Ok("ko"),
            Language::Eng => Ok("en"),
            Language::Jpn => Ok("ja"),
            Language::ZhoHans => Ok("zh-CN"),
            Language::ZhoHant => Ok("zh-TW"),
            Language::Vie => Ok("vi"),
            Language::Tha => Ok("th"),
            Language::Ind => Ok("id"),
            Language::Fra => Ok("fr"),
            Language::Spa => Ok("es"),
            Language::Rus => Ok("ru"),
            Language::Deu => Ok("de"),
            Language::Ita => Ok("it"),
            Language::Ara
            | Language::Hin
            | Language::Por
            | Language::Msa
            | Language::Nld
            | Language::Pol
            | Language::Tur
            | Language::Ukr => Err(TranslationError::UnsupportedLanguage {
                engine: "Papago",
                language: lang,
            }),
        }
    }
}
