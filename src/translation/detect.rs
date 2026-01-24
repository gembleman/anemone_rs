//! 언어 감지 모듈
//!
//! whatlang 크레이트와 Unicode 범위 기반 휴리스틱을 사용하여
//! 텍스트의 언어를 감지합니다.

use super::Language;

/// whatlang 기반 언어 감지
pub fn detect_language(text: &str) -> Option<Language> {
    // whatlang으로 언어 감지
    if let Some(info) = whatlang::detect(text) {
        let lang = match info.lang() {
            whatlang::Lang::Jpn => Language::Japanese,
            whatlang::Lang::Kor => Language::Korean,
            whatlang::Lang::Eng => Language::English,
            whatlang::Lang::Cmn => Language::ChineseSimplified,
            _ => return detect_language_heuristic(text),
        };
        return Some(lang);
    }

    // whatlang 실패 시 휴리스틱 폴백
    detect_language_heuristic(text)
}

/// Unicode 범위 기반 언어 감지 (폴백)
pub fn detect_language_heuristic(text: &str) -> Option<Language> {
    let mut hiragana_katakana = 0;
    let mut hangul = 0;
    let mut cjk = 0;
    let mut latin = 0;
    let mut total = 0;

    for ch in text.chars() {
        if !ch.is_whitespace() && !ch.is_ascii_punctuation() {
            total += 1;

            match ch {
                // 히라가나 (3040-309F)
                '\u{3040}'..='\u{309F}' => hiragana_katakana += 1,
                // 가타카나 (30A0-30FF)
                '\u{30A0}'..='\u{30FF}' => hiragana_katakana += 1,
                // 가타카나 반각 (FF65-FF9F)
                '\u{FF65}'..='\u{FF9F}' => hiragana_katakana += 1,
                // 한글 음절 (AC00-D7AF)
                '\u{AC00}'..='\u{D7AF}' => hangul += 1,
                // 한글 자모 (1100-11FF, 3130-318F)
                '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' => hangul += 1,
                // CJK 통합 한자 (4E00-9FFF)
                '\u{4E00}'..='\u{9FFF}' => cjk += 1,
                // CJK 확장 A (3400-4DBF)
                '\u{3400}'..='\u{4DBF}' => cjk += 1,
                // 라틴 알파벳 (a-z, A-Z)
                'a'..='z' | 'A'..='Z' => latin += 1,
                _ => {}
            }
        }
    }

    if total == 0 {
        return None;
    }

    // 임계값 설정 (20% 이상이면 해당 언어로 판정)
    let threshold = total / 5;

    // 히라가나/가타카나가 있으면 일본어
    if hiragana_katakana > threshold {
        return Some(Language::Japanese);
    }

    // 한글이 있으면 한국어
    if hangul > threshold {
        return Some(Language::Korean);
    }

    // CJK 한자만 있으면 중국어 (간체로 기본 설정)
    if cjk > threshold && hiragana_katakana == 0 && hangul == 0 {
        return Some(Language::ChineseSimplified);
    }

    // 라틴 알파벳이 대다수면 영어
    if latin > total / 2 {
        return Some(Language::English);
    }

    None
}

/// 텍스트가 소스 언어인지 확인
///
/// 자동 번역 시 소스 언어가 아닌 텍스트는 번역하지 않음
pub fn is_source_language(text: &str, source_lang: Language) -> bool {
    if let Some(detected) = detect_language(text) {
        detected == source_lang
    } else {
        // 감지 실패 시 휴리스틱으로 재시도
        if let Some(detected) = detect_language_heuristic(text) {
            detected == source_lang
        } else {
            // 언어를 감지할 수 없으면 번역하지 않음
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_japanese() {
        let text = "こんにちは世界";
        assert_eq!(detect_language(text), Some(Language::Japanese));
    }

    #[test]
    fn test_detect_korean() {
        let text = "안녕하세요 세계";
        assert_eq!(detect_language(text), Some(Language::Korean));
    }

    #[test]
    fn test_detect_english() {
        let text = "Hello World";
        assert_eq!(detect_language(text), Some(Language::English));
    }

    #[test]
    fn test_detect_chinese() {
        let text = "你好世界";
        assert_eq!(detect_language(text), Some(Language::ChineseSimplified));
    }

    #[test]
    fn test_is_source_language() {
        assert!(is_source_language("こんにちは", Language::Japanese));
        assert!(!is_source_language("안녕하세요", Language::Japanese));
    }
}
