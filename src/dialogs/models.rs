//! 다이얼로그가 소비하는 Win32 비의존 편집 모델.

use std::ops::{Deref, DerefMut};

use crate::config::{Config, EzTransPostprocessEntry, LlmGlossaryEntry};

/// 설정 창이 독립적으로 편집하고 AppAction으로 되돌려 보내는 설정 초안.
#[derive(Clone, Debug)]
pub(crate) struct SettingsDraft(Config);

impl SettingsDraft {
    pub(crate) fn new(config: Config) -> Self {
        Self(config)
    }

    pub(crate) fn into_config(self) -> Config {
        self.0
    }
}

impl Deref for SettingsDraft {
    type Target = Config;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for SettingsDraft {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftChange {
    Added(usize),
    Updated(usize),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DictionaryTarget {
    #[default]
    Llm,
    EzTransPostprocess,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DictionaryDraftEntry {
    pub source: String,
    pub target: String,
}

/// 적용 전까지 설정을 바꾸지 않는 사전 편집 초안.
#[derive(Clone, Debug, Default)]
pub struct GlossaryDraft {
    target: DictionaryTarget,
    entries: Vec<DictionaryDraftEntry>,
}

impl GlossaryDraft {
    pub fn from_config(config: &Config) -> Self {
        Self {
            target: DictionaryTarget::Llm,
            entries: config
                .translation
                .llm
                .glossary
                .iter()
                .map(|entry| DictionaryDraftEntry {
                    source: entry.source.clone(),
                    target: entry.target.clone(),
                })
                .collect(),
        }
    }

    pub fn from_eztrans_config(config: &Config) -> Self {
        Self {
            target: DictionaryTarget::EzTransPostprocess,
            entries: config
                .translation
                .eztrans_postprocess_dictionary
                .iter()
                .map(|entry| DictionaryDraftEntry {
                    source: entry.source.clone(),
                    target: entry.target.clone(),
                })
                .collect(),
        }
    }

    pub fn target(&self) -> DictionaryTarget {
        self.target
    }

    pub fn entries(&self) -> &[DictionaryDraftEntry] {
        &self.entries
    }

    pub fn add_or_update(&mut self, source: String, target: String) -> Option<DraftChange> {
        let source = source.trim().to_string();
        if source.is_empty() {
            return None;
        }
        let target = target.trim().to_string();
        if let Some((index, entry)) = self
            .entries
            .iter_mut()
            .enumerate()
            .find(|(_, entry)| entry.source == source)
        {
            entry.target = target;
            Some(DraftChange::Updated(index))
        } else {
            self.entries.push(DictionaryDraftEntry { source, target });
            Some(DraftChange::Added(self.entries.len() - 1))
        }
    }

    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.entries.len() {
            return false;
        }
        self.entries.remove(index);
        true
    }

    pub fn commit(self, config: &mut Config) {
        match self.target {
            DictionaryTarget::Llm => {
                config.translation.llm.glossary = self
                    .entries
                    .into_iter()
                    .map(|entry| LlmGlossaryEntry {
                        source: entry.source,
                        target: entry.target,
                    })
                    .collect();
            }
            DictionaryTarget::EzTransPostprocess => {
                config.translation.eztrans_postprocess_dictionary = self
                    .entries
                    .into_iter()
                    .map(|entry| EzTransPostprocessEntry {
                        source: entry.source,
                        target: entry.target,
                    })
                    .collect();
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/dialogs/models.rs"]
mod tests;
