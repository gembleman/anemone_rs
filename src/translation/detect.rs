//! 언어 감지 모듈
//!
//! whatlang 크레이트와 Unicode 범위 기반 휴리스틱을 사용하여
//! 텍스트의 언어를 감지합니다.

use isolang::Language;

/// whatlang 기반 언어 감지
pub fn detect_language(text: &str) -> Option<Language> {
    // whatlang으로 언어 감지
    if let Some(info) = whatlang::detect(text) {
        let lang = match info.lang() {
            whatlang::Lang::Jpn => Language::Jpn,
            whatlang::Lang::Kor => Language::Kor,
            whatlang::Lang::Eng => Language::Eng,
            whatlang::Lang::Cmn => Language::Zho,
            whatlang::Lang::Spa => Language::Spa,
            whatlang::Lang::Fra => Language::Fra,
            whatlang::Lang::Deu => Language::Deu,
            whatlang::Lang::Ita => Language::Ita,
            whatlang::Lang::Por => Language::Por,
            whatlang::Lang::Rus => Language::Rus,
            whatlang::Lang::Ara => Language::Ara,
            whatlang::Lang::Hin => Language::Hin,
            whatlang::Lang::Tha => Language::Tha,
            whatlang::Lang::Vie => Language::Vie,
            whatlang::Lang::Ind => Language::Ind,
            whatlang::Lang::Nld => Language::Nld,
            whatlang::Lang::Pol => Language::Pol,
            whatlang::Lang::Tur => Language::Tur,
            whatlang::Lang::Ukr => Language::Ukr,
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
        return Some(Language::Jpn);
    }

    // 한글이 있으면 한국어
    if hangul > threshold {
        return Some(Language::Kor);
    }

    // CJK 한자만 있으면 중국어
    if cjk > threshold && hiragana_katakana == 0 && hangul == 0 {
        return Some(Language::Zho);
    }

    // 라틴 알파벳이 대다수면 영어
    if latin > total / 2 {
        return Some(Language::Eng);
    }

    None
}

/// 텍스트가 소스 언어인지 확인
///
/// 자동 번역 시 소스 언어가 아닌 텍스트는 번역하지 않음
pub fn is_source_language(text: &str, source_lang: Language) -> bool {
    // detect_language는 내부에서 이미 detect_language_heuristic을 폴백으로 호출하므로
    // 별도의 재시도가 필요 없음
    detect_language(text)
        .is_some_and(|detected| detected == source_lang)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_japanese() {
        let text = "こんにちは世界";
        assert_eq!(detect_language(text), Some(Language::Jpn));
    }

    #[test]
    fn test_detect_korean() {
        let text = "안녕하세요 세계";
        assert_eq!(detect_language(text), Some(Language::Kor));
    }

    #[test]
    fn test_detect_english() {
        let text = "This is a simple English sentence for language detection";
        assert_eq!(detect_language(text), Some(Language::Eng));
    }

    #[test]
    fn test_detect_chinese() {
        let text = "你好世界";
        assert_eq!(detect_language(text), Some(Language::Zho));
    }

    #[test]
    fn test_is_source_language() {
        assert!(is_source_language("こんにちは", Language::Jpn));
        assert!(!is_source_language("안녕하세요", Language::Jpn));
    }
}
