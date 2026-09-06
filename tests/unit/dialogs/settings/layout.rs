use super::SettingsDialog;
use crate::translation::TranslationEngine;

#[cfg(mys_private)]
#[path = "layout_mys.rs"]
mod mys;

#[test]
fn translation_panel_height_follows_the_selected_engine() {
    assert!(
        SettingsDialog::translation_height_for_engine(TranslationEngine::Papago)
            < SettingsDialog::translation_height_for_engine(TranslationEngine::DeepL)
    );
    assert!(
        SettingsDialog::translation_height_for_engine(TranslationEngine::Papago)
            < SettingsDialog::translation_height_for_engine(TranslationEngine::EzTrans)
    );
    assert!(
        SettingsDialog::translation_height_for_engine(TranslationEngine::DeepL)
            < SettingsDialog::translation_height_for_engine(TranslationEngine::Llm)
    );
    assert_eq!(
        SettingsDialog::translation_height_for_engine(TranslationEngine::DeepL),
        365
    );
    assert!(
        SettingsDialog::translation_group_height_for_engine(TranslationEngine::Papago)
            < SettingsDialog::translation_group_height_for_engine(TranslationEngine::DeepL)
    );
    assert!(
        SettingsDialog::translation_group_height_for_engine(TranslationEngine::DeepL)
            < SettingsDialog::translation_group_height_for_engine(TranslationEngine::Llm)
    );
    assert_eq!(
        SettingsDialog::translation_group_height_for_engine(TranslationEngine::DeepL),
        136
    );
    assert_eq!(
        SettingsDialog::translation_height_for_engine(TranslationEngine::Custom),
        SettingsDialog::translation_height_for_engine(TranslationEngine::Papago)
    );
}

/// 그룹박스 테두리가 탭 바닥선을 넘지도, 달라붙지도 않아야 한다.
#[test]
fn translation_group_border_keeps_the_same_bottom_margin_on_every_engine() {
    for engine in TranslationEngine::ALL {
        let Some(_) = SettingsDialog::engine_group(engine) else {
            continue;
        };
        let margin = SettingsDialog::translation_group_bottom_margin(engine);
        assert!(
            (8..=32).contains(&margin),
            "{engine:?} 그룹박스 아래 여백이 {margin}픽셀이다"
        );
    }
}
