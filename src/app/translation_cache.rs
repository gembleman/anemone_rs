//! 클립보드 원문/번역문을 sqlite에 캐싱해 재번역 API 호출을 줄이는 저장소.
//!
//! Win32 UI와 독립적이며, 동일 (엔진, 언어쌍, 원문) 조합의 조회 결과를 재사용한다.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::translation::CacheKey;

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("캐시 데이터베이스를 열 수 없습니다: {0}")]
    Open(rusqlite::Error),
    #[error("캐시 쿼리 실패: {0}")]
    Query(rusqlite::Error),
}

/// 클립보드 번역 이력을 영속 저장하고, 동일 요청에 대해 캐시 히트를 제공한다.
pub struct TranslationCacheStore {
    conn: Option<Connection>,
}

impl TranslationCacheStore {
    /// 연결 실패 시에도 앱이 캐시 없이 계속 동작하도록 `None`을 담은 채로 반환한다.
    pub fn open(path: &Path) -> Self {
        match Self::open_inner(path) {
            Ok(conn) => Self { conn: Some(conn) },
            Err(error) => {
                tracing::warn!("번역 캐시를 열 수 없어 캐싱 없이 진행합니다: {error}");
                Self { conn: None }
            }
        }
    }

    fn open_inner(path: &Path) -> Result<Connection, CacheError> {
        let conn = Connection::open(path).map_err(CacheError::Open)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(CacheError::Query)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS translation_cache (
                engine_id TEXT NOT NULL,
                source_lang TEXT NOT NULL,
                target_lang TEXT NOT NULL,
                original TEXT NOT NULL,
                translation TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (engine_id, source_lang, target_lang, original)
            )",
            [],
        )
        .map_err(CacheError::Query)?;
        Ok(conn)
    }

    /// 캐시에 저장된 번역문을 찾는다. 캐시가 비활성 상태(연결 실패)면 항상 `None`.
    pub fn get(&self, key: &CacheKey) -> Option<String> {
        let conn = self.conn.as_ref()?;
        let result = conn
            .query_row(
                "SELECT translation FROM translation_cache
                 WHERE engine_id = ?1 AND source_lang = ?2 AND target_lang = ?3 AND original = ?4",
                params![
                    key.engine_id,
                    key.source_lang,
                    key.target_lang,
                    key.original
                ],
                |row| row.get::<_, String>(0),
            )
            .optional();
        match result {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!("번역 캐시 조회 실패: {error}");
                None
            }
        }
    }

    /// 번역 결과를 캐시에 저장(갱신)한다.
    pub fn put(&self, key: &CacheKey, translation: &str) {
        let Some(conn) = self.conn.as_ref() else {
            return;
        };
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let result = conn.execute(
            "INSERT INTO translation_cache (engine_id, source_lang, target_lang, original, translation, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(engine_id, source_lang, target_lang, original)
             DO UPDATE SET translation = excluded.translation, updated_at = excluded.updated_at",
            params![
                key.engine_id,
                key.source_lang,
                key.target_lang,
                key.original,
                translation,
                now
            ],
        );
        if let Err(error) = result {
            tracing::warn!("번역 캐시 저장 실패: {error}");
        }
    }

    /// 캐시에 저장된 모든 항목을 비운다.
    pub fn clear(&self) {
        let Some(conn) = self.conn.as_ref() else {
            return;
        };
        if let Err(error) = conn.execute("DELETE FROM translation_cache", []) {
            tracing::warn!("번역 캐시 비우기 실패: {error}");
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/cache.rs"]
mod tests;
