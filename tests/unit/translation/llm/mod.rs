use super::*;

#[test]
fn blank_system_prompt_falls_back_to_default() {
    for blank in ["", "   \r\n\t"] {
        let prompt = build_system_prompt_with_glossary(blank, Language::Jpn, Language::Kor, &[]);
        assert!(prompt.contains("일본어를 한국어로 번역"), "{prompt}");
        assert!(prompt.contains("번역 결과 텍스트만 출력"), "{prompt}");
    }
}

#[test]
fn custom_system_prompt_is_preserved_and_expanded() {
    let prompt = build_system_prompt_with_glossary(
        "{source} => {target}",
        Language::Eng,
        Language::Kor,
        &[],
    );
    assert_eq!(prompt, "영어 => 한국어");
}
