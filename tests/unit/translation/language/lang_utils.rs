use super::*;
use crate::translation::TranslationError;

const ALL_LANGUAGES: [Language; 21] = [
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

#[test]
fn to_code_returns_a_distinct_iso_code_for_every_language() {
    let codes: Vec<&str> = ALL_LANGUAGES.iter().map(|&l| to_code(l)).collect();
    let mut sorted = codes.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        codes.len(),
        "ISO 코드가 중복되면 언어를 구분할 수 없다"
    );
    assert_eq!(to_code(Language::Kor), "ko");
    assert_eq!(to_code(Language::ZhoHant), "zh-TW");
    assert_eq!(to_code(Language::Ukr), "uk");
}

#[test]
fn to_korean_name_returns_a_distinct_label_for_every_language() {
    let names: Vec<&str> = ALL_LANGUAGES.iter().map(|&l| to_korean_name(l)).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len());
    assert_eq!(to_korean_name(Language::Jpn), "일본어");
    assert_eq!(to_korean_name(Language::Ukr), "우크라이나어");
}

#[test]
fn to_english_name_returns_a_distinct_label_for_every_language() {
    let names: Vec<&str> = ALL_LANGUAGES.iter().map(|&l| to_english_name(l)).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len());
    assert_eq!(to_english_name(Language::Jpn), "Japanese");
    assert_eq!(to_english_name(Language::ZhoHant), "Traditional Chinese");
}

#[test]
fn from_code_round_trips_every_to_code_output() {
    for &lang in &ALL_LANGUAGES {
        let code = to_code(lang);
        assert_eq!(from_code(code), Some(lang), "code={code}");
    }
}

#[test]
fn from_code_is_case_insensitive_and_rejects_unknown_codes() {
    assert_eq!(from_code("JA"), Some(Language::Jpn));
    assert_eq!(from_code("Ko"), Some(Language::Kor));
    assert_eq!(from_code("ZH-CN"), Some(Language::ZhoHans));
    assert_eq!(from_code("xx"), None);
    assert_eq!(from_code(""), None);
}

#[test]
fn from_code_accepts_alternate_three_letter_and_alias_spellings() {
    // 같은 언어를 가리키는 서로 다른 표기가 모두 같은 Language로 모인다.
    assert_eq!(from_code("jpn"), Some(Language::Jpn));
    assert_eq!(from_code("zho"), Some(Language::ZhoHans));
    assert_eq!(from_code("chi"), Some(Language::ZhoHans));
    assert_eq!(from_code("zhs"), Some(Language::ZhoHans));
    assert_eq!(from_code("zht"), Some(Language::ZhoHant));
    assert_eq!(from_code("fre"), Some(Language::Fra));
    assert_eq!(from_code("ger"), Some(Language::Deu));
    assert_eq!(from_code("dut"), Some(Language::Nld));
    assert_eq!(from_code("may"), Some(Language::Msa));
}

#[test]
fn google_code_uses_region_tags_for_chinese_and_iso_codes_otherwise() {
    assert_eq!(to_google_code(Language::ZhoHans).unwrap(), "zh-CN");
    assert_eq!(to_google_code(Language::ZhoHant).unwrap(), "zh-TW");
    assert_eq!(to_google_code(Language::Kor).unwrap(), "ko");
    assert_eq!(to_google_code(Language::Eng).unwrap(), "en");
}

#[test]
fn deepl_code_maps_every_supported_language_to_its_uppercase_tag() {
    assert_eq!(to_deepl_code(Language::Jpn).unwrap(), "JA");
    assert_eq!(to_deepl_code(Language::Kor).unwrap(), "KO");
    assert_eq!(to_deepl_code(Language::Eng).unwrap(), "EN");
    assert_eq!(to_deepl_code(Language::ZhoHans).unwrap(), "ZH-HANS");
    assert_eq!(to_deepl_code(Language::ZhoHant).unwrap(), "ZH-HANT");
    assert_eq!(to_deepl_code(Language::Spa).unwrap(), "ES");
    assert_eq!(to_deepl_code(Language::Fra).unwrap(), "FR");
    assert_eq!(to_deepl_code(Language::Deu).unwrap(), "DE");
    assert_eq!(to_deepl_code(Language::Ita).unwrap(), "IT");
    assert_eq!(to_deepl_code(Language::Por).unwrap(), "PT");
    assert_eq!(to_deepl_code(Language::Rus).unwrap(), "RU");
    assert_eq!(to_deepl_code(Language::Nld).unwrap(), "NL");
    assert_eq!(to_deepl_code(Language::Pol).unwrap(), "PL");
    assert_eq!(to_deepl_code(Language::Tur).unwrap(), "TR");
    assert_eq!(to_deepl_code(Language::Ukr).unwrap(), "UK");
}

#[test]
fn deepl_code_rejects_languages_the_api_does_not_support() {
    for lang in [
        Language::Ara,
        Language::Hin,
        Language::Tha,
        Language::Vie,
        Language::Ind,
        Language::Msa,
    ] {
        match to_deepl_code(lang) {
            Err(TranslationError::UnsupportedLanguage { engine, language }) => {
                assert_eq!(engine, "DeepL");
                assert_eq!(language, lang);
            }
            other => panic!("expected UnsupportedLanguage for {lang:?}, got {other:?}"),
        }
    }
}

#[test]
fn papago_code_maps_every_supported_language_to_its_lowercase_tag() {
    assert_eq!(to_papago_code(Language::Kor).unwrap(), "ko");
    assert_eq!(to_papago_code(Language::Eng).unwrap(), "en");
    assert_eq!(to_papago_code(Language::Jpn).unwrap(), "ja");
    assert_eq!(to_papago_code(Language::ZhoHans).unwrap(), "zh-CN");
    assert_eq!(to_papago_code(Language::ZhoHant).unwrap(), "zh-TW");
    assert_eq!(to_papago_code(Language::Vie).unwrap(), "vi");
    assert_eq!(to_papago_code(Language::Tha).unwrap(), "th");
    assert_eq!(to_papago_code(Language::Ind).unwrap(), "id");
    assert_eq!(to_papago_code(Language::Fra).unwrap(), "fr");
    assert_eq!(to_papago_code(Language::Spa).unwrap(), "es");
    assert_eq!(to_papago_code(Language::Rus).unwrap(), "ru");
    assert_eq!(to_papago_code(Language::Deu).unwrap(), "de");
    assert_eq!(to_papago_code(Language::Ita).unwrap(), "it");
}

#[test]
fn papago_code_rejects_languages_the_api_does_not_support() {
    for lang in [
        Language::Ara,
        Language::Hin,
        Language::Por,
        Language::Msa,
        Language::Nld,
        Language::Pol,
        Language::Tur,
        Language::Ukr,
    ] {
        match to_papago_code(lang) {
            Err(TranslationError::UnsupportedLanguage { engine, language }) => {
                assert_eq!(engine, "Papago");
                assert_eq!(language, lang);
            }
            other => panic!("expected UnsupportedLanguage for {lang:?}, got {other:?}"),
        }
    }
}

#[test]
fn text_matches_script_detects_each_writing_system() {
    assert!(text_matches_script(Language::Jpn, "こんにちは"));
    assert!(text_matches_script(Language::Jpn, "漢字")); // 일본어는 한자만으로도 인정
    assert!(text_matches_script(Language::Kor, "안녕"));
    assert!(text_matches_script(Language::Kor, "漢字")); // 한국어도 한자를 인정
    assert!(text_matches_script(Language::ZhoHans, "汉字"));
    assert!(text_matches_script(Language::Eng, "hello"));
    assert!(text_matches_script(Language::Fra, "\u{00C0}")); // À: 라틴-1 보충
    assert!(text_matches_script(Language::Rus, "привет"));
    assert!(text_matches_script(Language::Ukr, "привіт"));
    assert!(text_matches_script(Language::Ara, "مرحبا"));
    assert!(text_matches_script(Language::Hin, "नमस्ते"));
    assert!(text_matches_script(Language::Tha, "สวัสดี"));
    assert!(!text_matches_script(Language::Jpn, "hello 123"));
}

#[test]
fn text_matches_script_recognizes_fullwidth_and_extended_latin_forms() {
    assert!(text_matches_script(Language::Eng, "\u{FF21}")); // 전각 A
    assert!(text_matches_script(Language::Eng, "\u{FF41}")); // 전각 a
    assert!(text_matches_script(Language::Vie, "\u{1EA1}")); // ạ: 라틴 확장 추가
    assert!(text_matches_script(Language::Jpn, "\u{FF76}")); // ｶ: 반각 가타카나
}

#[test]
fn text_matches_script_recognizes_cjk_extension_b_and_compatibility_ranges() {
    assert!(text_matches_script(Language::Kor, "\u{20000}")); // CJK 확장 B
    assert!(text_matches_script(Language::Jpn, "\u{F900}")); // CJK 호환 한자
    assert!(text_matches_script(Language::Kor, "\u{FFA0}")); // 반각 한글 자모 범위
    assert!(text_matches_script(Language::Ara, "\u{FB50}")); // 아랍 문자 표현형 A
    assert!(text_matches_script(Language::Ara, "\u{FE70}")); // 아랍 문자 표현형 B
}
