//! 언어 코드 헬퍼 함수들
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

/// 텍스트가 `lang`의 문자 체계로 쓰였다고 볼 만한지 판정한다.
///
/// 해당 문자 체계의 글자가 **하나라도** 있으면 참이다. 문자 체계를 공유하는
/// 언어끼리는(영어/프랑스어, 일본어/중국어 등) 구분하지 못하므로, 이 판정은
/// "이 언어일 수 있다"까지만 말한다. 오탐(다른 언어를 소스 언어로 봄)은 번역을
/// 그대로 시도할 뿐이지만, 미탐은 번역을 통째로 건너뛰어 훨씬 손해라 판정을
/// 관대한 쪽으로 잡았다.
pub fn text_matches_script(lang: Language, text: &str) -> bool {
    let script = script_of(lang);
    text.chars().any(|ch| script.contains(ch))
}

/// 언어를 구분하는 데 쓰는 문자 체계.
#[derive(Clone, Copy)]
enum Script {
    Japanese,
    Hangul,
    Han,
    Latin,
    Cyrillic,
    Arabic,
    Devanagari,
    Thai,
}

fn script_of(lang: Language) -> Script {
    match lang {
        Language::Jpn => Script::Japanese,
        Language::Kor => Script::Hangul,
        Language::ZhoHans | Language::ZhoHant => Script::Han,
        Language::Rus | Language::Ukr => Script::Cyrillic,
        Language::Ara => Script::Arabic,
        Language::Hin => Script::Devanagari,
        Language::Tha => Script::Thai,
        Language::Eng
        | Language::Spa
        | Language::Fra
        | Language::Deu
        | Language::Ita
        | Language::Por
        | Language::Vie
        | Language::Ind
        | Language::Msa
        | Language::Nld
        | Language::Pol
        | Language::Tur => Script::Latin,
    }
}

impl Script {
    fn contains(self, ch: char) -> bool {
        match self {
            // 일본어와 한국어 원문에는 한자가 섞이고, 일본어에는 한자만으로 된
            // 짧은 문장(인명·지명·선택지)도 흔하므로 한자를 함께 인정한다.
            Self::Japanese => is_kana(ch) || is_han(ch),
            Self::Hangul => is_hangul(ch) || is_han(ch),
            Self::Han => is_han(ch),
            Self::Latin => is_latin(ch),
            Self::Cyrillic => matches!(ch, '\u{0400}'..='\u{052F}'),
            Self::Arabic => {
                matches!(ch, '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
            }
            Self::Devanagari => matches!(ch, '\u{0900}'..='\u{097F}'),
            Self::Thai => matches!(ch, '\u{0E00}'..='\u{0E7F}'),
        }
    }
}

fn is_kana(ch: char) -> bool {
    matches!(ch,
        '\u{3041}'..='\u{309F}'   // 히라가나
        | '\u{30A0}'..='\u{30FF}' // 가타카나
        | '\u{31F0}'..='\u{31FF}' // 가타카나 음성 확장
        | '\u{FF66}'..='\u{FF9D}' // 반각 가타카나
    )
}

fn is_han(ch: char) -> bool {
    matches!(ch,
        '\u{3005}'                  // 々 (반복 기호)
        | '\u{3400}'..='\u{4DBF}'   // CJK 확장 A
        | '\u{4E00}'..='\u{9FFF}'   // CJK 통합 한자
        | '\u{F900}'..='\u{FAFF}'   // CJK 호환 한자
        | '\u{20000}'..='\u{2FA1F}' // CJK 확장 B 이상
    )
}

fn is_hangul(ch: char) -> bool {
    matches!(ch,
        '\u{1100}'..='\u{11FF}'   // 한글 자모
        | '\u{3130}'..='\u{318F}' // 호환 자모
        | '\u{A960}'..='\u{A97F}' // 확장 자모 A
        | '\u{AC00}'..='\u{D7A3}' // 완성형 음절
        | '\u{FFA0}'..='\u{FFDC}' // 반각 자모
    )
}

/// 발음 부호가 붙은 유럽어와 베트남어까지 포함하는 라틴 문자.
fn is_latin(ch: char) -> bool {
    ch.is_ascii_alphabetic()
        || matches!(ch,
            '\u{00C0}'..='\u{024F}'   // 라틴-1 보충 + 확장 A/B
            | '\u{1E00}'..='\u{1EFF}' // 라틴 확장 추가 (베트남어)
            | '\u{FF21}'..='\u{FF3A}' // 전각 A-Z
            | '\u{FF41}'..='\u{FF5A}' // 전각 a-z
        )
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

#[cfg(test)]
#[path = "../../../tests/unit/translation/language/lang_utils.rs"]
mod tests;
