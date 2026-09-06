use super::{Language, lang_utils::text_matches_script};

#[test]
fn japanese_script_detection_accepts_kana_kanji_and_halfwidth() {
    for text in [
        "こんにちは",
        "コンニチハ",
        "ｺﾝﾆﾁﾊ",
        "選択肢",
        "人々",
        "「……そう」と彼は言った。",
        "Hello、世界",
    ] {
        assert!(
            text_matches_script(Language::Jpn, text),
            "일본어로 인정되어야 합니다: {text}"
        );
    }
}

#[test]
fn japanese_script_detection_rejects_text_without_japanese_characters() {
    for text in [
        "",
        "안녕하세요",
        "Press any key to continue",
        "1234-5678",
        "https://example.com/path?query=1",
        "「」……！？",
    ] {
        assert!(
            !text_matches_script(Language::Jpn, text),
            "일본어가 아니라고 판정되어야 합니다: {text}"
        );
    }
}

/// 소스가 영어면 라틴 문자가 하나도 없는 원문이 걸러진다. 발음 부호가 붙은
/// 유럽어와 베트남어도 같은 라틴 문자 판정을 쓴다.
#[test]
fn latin_script_detection_covers_english_and_accented_languages() {
    for text in ["Save / Load", "Café", "Tiếng Việt", "URL: example.com"] {
        assert!(
            text_matches_script(Language::Eng, text),
            "라틴 문자로 인정되어야 합니다: {text}"
        );
    }
    for text in ["こんにちは", "안녕하세요", "Привет", "――……！", "12345"] {
        assert!(
            !text_matches_script(Language::Eng, text),
            "라틴 문자가 없다고 판정되어야 합니다: {text}"
        );
    }
    assert!(text_matches_script(Language::Fra, "déjà vu"));
    assert!(text_matches_script(Language::Vie, "Xin chào"));
}

#[test]
fn each_script_group_accepts_its_own_language_and_rejects_the_others() {
    let samples = [
        (Language::Kor, "안녕하세요"),
        (Language::Jpn, "こんにちは"),
        (Language::ZhoHans, "你好世界"),
        (Language::Eng, "Hello there"),
        (Language::Rus, "Привет"),
        (Language::Ara, "مرحبا"),
        (Language::Hin, "नमस्ते"),
        (Language::Tha, "สวัสดี"),
    ];

    for (lang, text) in samples {
        assert!(
            text_matches_script(lang, text),
            "{text}는 자기 언어의 문자 체계로 인정되어야 합니다"
        );
        for (other, _) in samples {
            // 한자는 일본어·한국어·중국어가 공유하므로 CJK끼리는 교차 판정을
            // 요구하지 않는다.
            let cjk = |lang| {
                matches!(
                    lang,
                    Language::Jpn | Language::Kor | Language::ZhoHans | Language::ZhoHant
                )
            };
            if other == lang || (cjk(lang) && cjk(other)) {
                continue;
            }
            assert!(
                !text_matches_script(other, text),
                "{text}가 다른 문자 체계({other:?})로도 판정되었습니다"
            );
        }
    }
}

#[test]
fn only_the_hook_bound_engine_is_excluded_from_file_translation() {
    use super::TranslationEngine;

    for engine in TranslationEngine::ALL {
        assert_eq!(
            engine.supports_file_translation(),
            !engine.requires_hook_session(),
            "{engine:?}"
        );
    }

    assert!(TranslationEngine::MysTranslater.requires_hook_session());
    assert!(!TranslationEngine::MysTranslater.supports_file_translation());

    for engine in TranslationEngine::ALL {
        if engine == TranslationEngine::MysTranslater {
            continue;
        }
        assert!(engine.supports_file_translation(), "{engine:?}");
    }
}
