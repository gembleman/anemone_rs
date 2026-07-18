//! 다이얼로그가 소비하는 Win32 비의존 편집 모델.

use crate::config::{Config, LlmGlossaryEntry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftChange {
    Added(usize),
    Updated(usize),
}

/// 적용 전까지 설정을 바꾸지 않는 사전 편집 초안.
#[derive(Clone, Debug, Default)]
pub struct GlossaryDraft {
    entries: Vec<LlmGlossaryEntry>,
}

impl GlossaryDraft {
    pub fn from_config(config: &Config) -> Self {
        Self {
            entries: config.translation.llm.glossary.clone(),
        }
    }

    pub fn entries(&self) -> &[LlmGlossaryEntry] {
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
            self.entries.push(LlmGlossaryEntry { source, target });
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
        config.translation.llm.glossary = self.entries;
    }
}

#[cfg(test)]
#[path = "../../tests/unit/dialogs/models.rs"]
mod tests;
