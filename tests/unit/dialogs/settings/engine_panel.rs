use super::{EngineGroup, SettingsDialog};
use crate::translation::TranslationEngine;

#[cfg(mys_private)]
#[path = "engine_panel_mys.rs"]
mod mys;

#[test]
fn each_selectable_engine_maps_to_its_own_panel_group() {
    assert_eq!(
        SettingsDialog::engine_group(TranslationEngine::DeepL),
        Some(EngineGroup::DeepL)
    );
    assert_eq!(
        SettingsDialog::engine_group(TranslationEngine::Papago),
        Some(EngineGroup::Papago)
    );
    assert_eq!(
        SettingsDialog::engine_group(TranslationEngine::Custom),
        Some(EngineGroup::Custom)
    );
    assert_eq!(
        SettingsDialog::engine_group(TranslationEngine::Google),
        None
    );
}
