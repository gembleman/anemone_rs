//! 번역 창과 파일 번역 창이 공유하는 엔진 콤보 규칙.

use crate::translation::TranslationEngine;

pub(crate) const ENGINES: [TranslationEngine; 6] = [
    TranslationEngine::EzTrans,
    TranslationEngine::Google,
    TranslationEngine::DeepL,
    TranslationEngine::Papago,
    TranslationEngine::Llm,
    TranslationEngine::Custom,
];

pub(crate) const CUSTOM_INDEX: usize = ENGINES.len() - 1;

pub(crate) fn engine_from_index(index: usize) -> Option<TranslationEngine> {
    if index >= CUSTOM_INDEX {
        Some(TranslationEngine::Custom)
    } else {
        ENGINES.get(index).copied()
    }
}

pub(crate) fn index_from_engine(engine: TranslationEngine) -> Option<usize> {
    ENGINES.iter().position(|&candidate| candidate == engine)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_only_engine_is_not_in_non_overlay_dialogs() {
        assert!(!ENGINES.contains(&TranslationEngine::MysTranslater));
        assert_eq!(index_from_engine(TranslationEngine::MysTranslater), None);
    }

    #[test]
    fn every_custom_api_row_maps_to_custom_engine() {
        assert_eq!(
            engine_from_index(CUSTOM_INDEX),
            Some(TranslationEngine::Custom)
        );
        assert_eq!(
            engine_from_index(CUSTOM_INDEX + 3),
            Some(TranslationEngine::Custom)
        );
    }
}
