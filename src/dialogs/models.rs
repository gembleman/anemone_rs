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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionChange {
    pub source_selection: Option<usize>,
    pub target_selection: usize,
}

/// 활성/비활성 목록을 적용 전까지 보관하는 후크 편집 초안.
#[derive(Clone, Debug, Default)]
pub struct HookListDraft {
    active: Vec<String>,
    inactive: Vec<String>,
}

impl HookListDraft {
    pub fn from_config(config: &Config) -> Self {
        Self {
            active: config.hook.active_hooks.clone(),
            inactive: config.hook.inactive_hooks.clone(),
        }
    }

    pub fn active(&self) -> &[String] {
        &self.active
    }
    pub fn inactive(&self) -> &[String] {
        &self.inactive
    }

    pub fn move_to_active(&mut self, index: usize) -> Option<SelectionChange> {
        transfer(&mut self.inactive, &mut self.active, index)
    }

    pub fn move_to_inactive(&mut self, index: usize) -> Option<SelectionChange> {
        transfer(&mut self.active, &mut self.inactive, index)
    }

    pub fn move_active_by(&mut self, index: usize, offset: isize) -> Option<usize> {
        let target = index.checked_add_signed(offset)?;
        if index >= self.active.len() || target >= self.active.len() {
            return None;
        }
        self.active.swap(index, target);
        Some(target)
    }

    pub fn commit(self, config: &mut Config) {
        config.hook.active_hooks = self.active;
        config.hook.inactive_hooks = self.inactive;
    }
}

fn transfer(
    source: &mut Vec<String>,
    target: &mut Vec<String>,
    index: usize,
) -> Option<SelectionChange> {
    if index >= source.len() {
        return None;
    }
    target.push(source.remove(index));
    Some(SelectionChange {
        source_selection: (!source.is_empty()).then(|| index.min(source.len() - 1)),
        target_selection: target.len() - 1,
    })
}

#[cfg(test)]
#[path = "../../tests/unit/dialogs/models.rs"]
mod tests;
