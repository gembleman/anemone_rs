//! Win32 UI와 독립적인 번역 백로그 모델.

use std::collections::VecDeque;
use std::io::Write;
use std::path::Path;

/// 실행 중 백로그가 무제한 성장하지 않도록 하는 보존 상한.
pub const MAX_BACKLOG_ENTRIES: usize = 1_000;
pub const MAX_BACKLOG_TEXT_BYTES: usize = 4 * 1024 * 1024;

/// 백로그에서 표시할 텍스트 종류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKind {
    Name,
    Original,
    Translation,
}

/// RichEdit 등의 UI 어댑터가 스타일을 적용할 수 있는 텍스트 조각.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyledText {
    pub text: String,
    pub kind: TextKind,
}

/// 백로그 필터.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum BacklogFilter {
    Original,
    Translation,
    #[default]
    All,
}

/// 로그 항목.
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub name: Option<String>,
    pub original: String,
    pub translation: Option<String>,
}

impl LogEntry {
    pub fn new(original: String) -> Self {
        Self {
            name: None,
            original,
            translation: None,
        }
    }

    pub fn with_translation(mut self, translation: String) -> Self {
        self.translation = Some(translation);
        self
    }
}

/// 창 수명과 독립적으로 애플리케이션 실행 중 번역 이력을 보관한다.
#[derive(Default)]
pub struct BacklogStore {
    entries: VecDeque<LogEntry>,
    text_bytes: usize,
}

impl BacklogStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// UI가 전체 다시 그리기 여부를 결정할 수 있도록 퇴출 여부를 반환한다.
    pub(crate) fn push(&mut self, entry: LogEntry) -> bool {
        self.text_bytes = self.text_bytes.saturating_add(entry_text_bytes(&entry));
        self.entries.push_back(entry);

        let mut evicted = false;
        while self.entries.len() > MAX_BACKLOG_ENTRIES
            || (self.text_bytes > MAX_BACKLOG_TEXT_BYTES && self.entries.len() > 1)
        {
            let removed = self
                .entries
                .pop_front()
                .expect("backlog limit requires at least one removable entry");
            self.text_bytes = self.text_bytes.saturating_sub(entry_text_bytes(&removed));
            evicted = true;
        }
        evicted
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.text_bytes = 0;
    }

    /// 필터와 줄바꿈 옵션을 반영한 UI 독립 텍스트 조각을 생성한다.
    pub fn render(&self, filter: BacklogFilter, add_linefeed: bool) -> Vec<StyledText> {
        self.entries
            .iter()
            .flat_map(|entry| format_entry(entry, filter, add_linefeed))
            .collect()
    }

    pub fn render_entry(
        entry: &LogEntry,
        filter: BacklogFilter,
        add_linefeed: bool,
    ) -> Vec<StyledText> {
        format_entry(entry, filter, add_linefeed)
    }

    /// UTF-8 BOM이 있는 텍스트 파일로 전체 백로그를 내보낸다.
    pub fn export_utf8(&self, path: &Path) -> Result<(), BacklogExportError> {
        let mut file =
            std::fs::File::create(path).map_err(|source| BacklogExportError::Create {
                path: path.to_path_buf(),
                source,
            })?;
        file.write_all(&[0xEF, 0xBB, 0xBF])
            .and_then(|_| file.write_all(self.export_text().as_bytes()))
            .map_err(|source| BacklogExportError::Write {
                path: path.to_path_buf(),
                source,
            })
    }

    fn export_text(&self) -> String {
        let mut content = String::new();
        for entry in &self.entries {
            if let Some(name) = &entry.name {
                content.push_str(&format!("[{name}] "));
            }
            content.push_str(&entry.original);
            content.push_str("\r\n");
            if let Some(translation) = &entry.translation {
                content.push_str(translation);
                content.push_str("\r\n");
            }
            content.push_str("\r\n");
        }
        content
    }
}

fn entry_text_bytes(entry: &LogEntry) -> usize {
    entry
        .name
        .as_ref()
        .map_or(0, String::len)
        .saturating_add(entry.original.len())
        .saturating_add(entry.translation.as_ref().map_or(0, String::len))
}

#[derive(Debug, thiserror::Error)]
pub enum BacklogExportError {
    #[error("백로그 파일을 만들 수 없습니다: {path}: {source}")]
    Create {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("백로그 파일을 쓸 수 없습니다: {path}: {source}")]
    Write {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn format_entry(entry: &LogEntry, filter: BacklogFilter, add_linefeed: bool) -> Vec<StyledText> {
    let mut segments = Vec::new();
    if let Some(name) = &entry.name
        && filter != BacklogFilter::Translation
    {
        segments.push(StyledText {
            text: format!("[{name}] "),
            kind: TextKind::Name,
        });
    }
    if filter != BacklogFilter::Translation {
        segments.push(StyledText {
            text: entry.original.clone(),
            kind: TextKind::Original,
        });
        if add_linefeed {
            segments.push(StyledText {
                text: "\r\n".into(),
                kind: TextKind::Original,
            });
        }
    }
    if let Some(translation) = &entry.translation
        && filter != BacklogFilter::Original
    {
        segments.push(StyledText {
            text: translation.clone(),
            kind: TextKind::Translation,
        });
        if add_linefeed {
            segments.push(StyledText {
                text: "\r\n".into(),
                kind: TextKind::Translation,
            });
        }
    }
    if filter == BacklogFilter::All && add_linefeed {
        segments.push(StyledText {
            text: "\r\n".into(),
            kind: TextKind::Original,
        });
    }
    segments
}

#[cfg(test)]
#[path = "../tests/unit/backlog.rs"]
mod tests;
