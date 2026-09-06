use rusqlite::Connection;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct UsageSnapshot {
    pub fresh_prompt_tokens: u64,
    pub fresh_output_tokens: u64,
    pub cached_characters: u64,
    pub fresh_count: u64,
    pub cached_count: u64,
}

impl UsageSnapshot {
    pub(crate) fn fresh_total_tokens(self) -> u64 {
        self.fresh_prompt_tokens
            .saturating_add(self.fresh_output_tokens)
    }

    pub(crate) fn total_count(self) -> u64 {
        self.fresh_count.saturating_add(self.cached_count)
    }
}

pub(crate) fn initialize_schema(_conn: &Connection) -> rusqlite::Result<()> {
    Ok(())
}

pub(crate) fn snapshot(_base_url: &str, _api_token: &str) -> Option<UsageSnapshot> {
    None
}
