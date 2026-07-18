use super::*;

#[test]
fn debounce_is_scoped_to_llm_requests() {
    assert_eq!(debounce_delay_ms(TranslationEngine::Llm, 300), 300);
    assert_eq!(debounce_delay_ms(TranslationEngine::Llm, 0), 0);
    assert_eq!(debounce_delay_ms(TranslationEngine::Google, 300), 0);
    assert_eq!(debounce_delay_ms(TranslationEngine::EzTrans, 300), 0);
}
