//! 클립보드 원문/번역문을 sqlite에 캐싱해 재번역 API 호출을 줄이는 저장소.
//!
//! Win32 UI와 독립적이며, 동일 (엔진, 언어쌍, 원문) 조합의 조회 결과를 재사용한다.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

use crate::translation::CacheKey;

/// updated_at 기준 보존 기간. 이보다 오래된 항목은 prune에서 삭제된다.
const CACHE_TTL_SECS: i64 = 30 * 24 * 60 * 60;
/// 최대 캐시 행 수. 초과분은 updated_at 오래된 순(LRU)으로 삭제된다.
const CACHE_MAX_ROWS: i64 = 10_000;
/// put N회마다 prune을 실행한다. put 경로마다 실행할 필요는 없다.
const PUTS_PER_PRUNE: u64 = 100;
/// 백그라운드 VACUUM 연결이 UI 스레드의 get/put을 기다리는 최대 시간.
/// WAL은 쓰기 1개만 허용하므로 VACUUM이 잠깐 대기할 수 있다.
const VACUUM_BUSY_TIMEOUT: Duration = Duration::from_secs(30);
/// UI 스레드 연결의 busy_timeout. SQLite 기본값은 0(즉시 SQLITE_BUSY 실패)이라
/// 백그라운드 VACUUM이 write lock을 쥔 순간 UI의 put은 실패하고 get은 에러를
/// `None`으로 변환해 캐시 미스로 위장한다 — 캐시 미스는 유료 엔진 API 재호출
/// (재과금)로 이어지므로 VACUUM_BUSY_TIMEOUT보다 짧게라도 대기를 준다.
/// UI 체감 지연을 과하게 늘리지 않도록 VACUUM_BUSY_TIMEOUT(30초)보다 짧게 잡는다.
const UI_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("캐시 데이터베이스를 열 수 없습니다: {0}")]
    Open(rusqlite::Error),
    #[error("캐시 쿼리 실패: {0}")]
    Query(rusqlite::Error),
}

/// 클립보드 번역 이력을 영속 저장하고, 동일 요청에 대해 캐시 히트를 제공한다.
///
/// `Connection`은 UI 스레드에서만 접근하므로 `RefCell`로 감싸고,
/// `prepare_cached`로 get/put 문장을 재사용한다. 수 초가 걸릴 수 있는
/// VACUUM만 별도 연결의 백그라운드 스레드로 격리한다.
pub struct TranslationCacheStore {
    conn: Option<RefCell<Connection>>,
    db_path: Option<PathBuf>,
    puts_since_prune: Cell<u64>,
    /// 이 저장소의 VACUUM이 백그라운드에서 실행 중인지. 프로세스 전역이 아니라
    /// 인스턴스별로 두어야 한다 — 전역 플래그는 서로 다른 DB 파일의 VACUUM끼리도
    /// 막아버린다. 백그라운드 스레드와 공유하므로 `Arc`.
    vacuum_in_progress: Arc<AtomicBool>,
}

impl TranslationCacheStore {
    /// 연결 실패 시에도 앱이 캐시 없이 계속 동작하도록 `None`을 담은 채로 반환한다.
    /// 앱 시작 시 만료/상한 초과 항목을 1회 정리한다.
    pub fn open(path: &Path) -> Self {
        match Self::open_inner(path) {
            Ok(conn) => {
                let store = Self {
                    conn: Some(RefCell::new(conn)),
                    db_path: Some(path.to_path_buf()),
                    puts_since_prune: Cell::new(0),
                    vacuum_in_progress: Arc::new(AtomicBool::new(false)),
                };
                store.prune();
                store
            }
            Err(error) => {
                tracing::warn!("번역 캐시를 열 수 없어 캐싱 없이 진행합니다: {error}");
                Self {
                    conn: None,
                    db_path: None,
                    puts_since_prune: Cell::new(0),
                    vacuum_in_progress: Arc::new(AtomicBool::new(false)),
                }
            }
        }
    }

    fn now_secs() -> i64 {
        time::OffsetDateTime::now_utc().unix_timestamp()
    }

    /// 만료(TTL 초과) 항목과 행 수 상한 초과분을 삭제한다.
    /// get 지연과 clear() 프리즈의 근원인 무한 누적을 막는 유일한 회수 경로다.
    fn prune(&self) {
        self.prune_at(Self::now_secs());
    }

    /// `prune`의 본체. 기준 시각을 인자로 받아 TTL 경계 테스트가 저장소와
    /// 동일한 `now`를 공유할 수 있게 한다 — 각자 `now_secs()`를 부르면 그 사이
    /// 초 경계를 넘길 때 경계 판정이 1초 밀려 테스트가 간헐적으로 깨진다.
    fn prune_at(&self, now: i64) {
        let Some(conn) = self.conn.as_ref() else {
            return;
        };
        let conn = conn.borrow();
        let cutoff = now - CACHE_TTL_SECS;
        match conn.execute(
            "DELETE FROM translation_cache WHERE updated_at < ?1",
            [cutoff],
        ) {
            Ok(removed) if removed > 0 => {
                tracing::debug!(removed, "translation cache TTL prune");
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!("번역 캐시 TTL 정리 실패: {error}");
                return;
            }
        }
        // 상한 초과분은 최신 updated_at 기준 상위 N개만 남기고 나머지를 삭제한다.
        // rowid 기반 NOT IN은 PK(복합 컬럼) row-value 비교보다 인덱스를 훨씬 잘
        // 타므로 O(N×M) 최악 케이스를 피한다 — 이 테이블은 WITHOUT ROWID가 아니라
        // rowid가 항상 존재한다.
        match conn.execute(
            "DELETE FROM translation_cache
             WHERE rowid NOT IN (
                 SELECT rowid
                 FROM translation_cache
                 ORDER BY updated_at DESC
                 LIMIT ?1
             )",
            [CACHE_MAX_ROWS],
        ) {
            Ok(removed) if removed > 0 => {
                tracing::debug!(removed, "translation cache cap prune");
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!("번역 캐시 상한 정리 실패: {error}");
            }
        }
    }

    fn open_inner(path: &Path) -> Result<Connection, CacheError> {
        let conn = Connection::open(path).map_err(CacheError::Open)?;
        // 백그라운드 VACUUM이 write lock을 쥔 짧은 구간 동안 UI 스레드의 get/put이
        // 즉시 SQLITE_BUSY로 실패해 캐시 미스로 위장되지 않도록 대기 시간을 둔다.
        conn.busy_timeout(UI_BUSY_TIMEOUT)
            .map_err(CacheError::Query)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(CacheError::Query)?;
        // WAL에서 fsync를 checkpoint 시점으로 미룬다. 번역 캐시는 재생성 가능
        // 데이터라 크래시 시 마지막 일부 항목 유실은 수용 가능하다 — UI 스레드
        // put 지연(벤치: FULL 344us vs NORMAL 57us 평균)을 줄이는 것이 우선이다.
        conn.pragma_update(None, "synchronous", "NORMAL")
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
        // TTL DELETE와 cap prune의 ORDER BY updated_at이 매번 풀스캔+전체 정렬로
        // 빠지지 않도록 인덱스를 둔다. 기존 DB에도 적용되도록 IF NOT EXISTS.
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_translation_cache_updated_at
             ON translation_cache(updated_at)",
            [],
        )
        .map_err(CacheError::Query)?;
        // 번역 서버 사용량도 같은 DB 파일을 공유한다. 캐시를 한 번도 쓰지 않은 상태에서
        // 설정창을 먼저 열어도 스키마가 준비되어 있도록 앱 시작 연결에서 생성한다.
        crate::translation::mys_usage::initialize_schema(&conn).map_err(CacheError::Query)?;
        Ok(conn)
    }

    /// 캐시에 저장된 번역문을 찾는다. 캐시가 비활성 상태(연결 실패)면 항상 `None`.
    /// 준비된 Statement를 재사용한다 (단순 PK 조회의 prepare는 수십 us지만
    /// 호출당 반복은 무의미한 비용이다).
    pub fn get(&self, key: &CacheKey) -> Option<String> {
        let conn = self.conn.as_ref()?;
        // prepare_cached는 &self지만 내부 StatementCache를 쓰므로 RefCell이 필요하다.
        // CachedStatement는 클로저 안에서 즉시 소비돼 borrow를 넘기지 않는다.
        let conn = conn.borrow();
        let result = conn
            .prepare_cached(
                "SELECT translation FROM translation_cache
                 WHERE engine_id = ?1 AND source_lang = ?2 AND target_lang = ?3 AND original = ?4",
            )
            .and_then(|mut stmt| {
                stmt.query_row(
                    params![
                        key.engine_id,
                        key.source_lang,
                        key.target_lang,
                        key.original
                    ],
                    |row| row.get::<_, String>(0),
                )
            })
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
        let now = Self::now_secs();
        // borrow는 이 블록으로 한정한다 — 이후 실행되는 prune()이 같은 RefCell에
        // 다시 접근해도 안전하도록 CachedStatement가 블록을 넘지 않게 한다.
        let result = {
            let conn = conn.borrow();
            conn.prepare_cached(
                "INSERT INTO translation_cache (engine_id, source_lang, target_lang, original, translation, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(engine_id, source_lang, target_lang, original)
                 DO UPDATE SET translation = excluded.translation, updated_at = excluded.updated_at",
            ).and_then(|mut stmt| {
                stmt.execute(params![
                    key.engine_id,
                    key.source_lang,
                    key.target_lang,
                    key.original,
                    translation,
                    now
                ])
            })
        };
        if let Err(error) = result {
            tracing::warn!("번역 캐시 저장 실패: {error}");
            return;
        }
        // put 경로마다 prune하지 않고 N회마다 한 번만 실행한다.
        let puts = self.puts_since_prune.get() + 1;
        if puts >= PUTS_PER_PRUNE {
            self.puts_since_prune.set(0);
            self.prune();
        } else {
            self.puts_since_prune.set(puts);
        }
    }

    /// 캐시에 저장된 모든 항목을 비운다. DELETE는 UI 스레드에서 끝내고,
    /// 파일 크기 회수를 위한 VACUUM(캐시가 크면 수 초)만 백그라운드 스레드로
    /// 미룬다 — clear() 프리즈의 원인이 바로 VACUUM이다.
    pub fn clear(&self) {
        self.puts_since_prune.set(0);
        let Some(conn) = self.conn.as_ref() else {
            return;
        };
        if let Err(error) = conn.borrow().execute("DELETE FROM translation_cache", []) {
            tracing::warn!("번역 캐시 비우기 실패: {error}");
            return;
        }
        let Some(path) = self.db_path.as_ref() else {
            return;
        };
        Self::vacuum_in_background(path, Arc::clone(&self.vacuum_in_progress));
    }

    /// VACUUM은 DB 전체를 재작성하므로 별도 연결의 백그라운드 스레드에서 실행한다.
    /// WAL 모드에서 쓰기는 하나뿐이므로 get/put과 경합하면 busy_timeout으로
    /// 대기한다. 결과를 기다리는 호출자는 없어 실패해도 로그만 남긴다.
    ///
    /// `clear()`가 연달아 호출되면(예: 사용자가 버튼을 연타) 매번 새 스레드 +
    /// 새 연결의 VACUUM이 만들어져 서로 busy 경합을 벌인다. 이미 VACUUM이
    /// 진행 중이면 이번 호출은 건너뛴다 — DELETE는 이미 끝났으므로 논리적
    /// 정합성엔 문제가 없고, 다음 clear()가 다시 VACUUM을 예약해 준다.
    /// 이 게이트는 저장소 인스턴스별이다: 전역이면 서로 다른 DB 파일의 VACUUM도
    /// 부당하게 막힌다.
    fn vacuum_in_background(path: &Path, in_progress: Arc<AtomicBool>) {
        if in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            tracing::debug!("translation cache VACUUM already running; skipping");
            return;
        }

        // 패닉 경로에서도 플래그가 반드시 풀리도록 Drop으로 해제한다 — 스레드
        // 안에서 앞으로 코드가 늘어나도 unlock 누락을 걱정할 필요가 없다.
        struct ResetOnDrop(Arc<AtomicBool>);
        impl Drop for ResetOnDrop {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }

        let path = path.to_path_buf();
        std::thread::spawn(move || {
            let _guard = ResetOnDrop(in_progress);
            let result = Connection::open(&path)
                .and_then(|conn| {
                    conn.busy_timeout(VACUUM_BUSY_TIMEOUT)?;
                    conn.execute("VACUUM", [])
                })
                .map(|_| ());
            match result {
                Ok(()) => tracing::debug!("translation cache VACUUM complete"),
                Err(error) => tracing::warn!("번역 캐시 VACUUM 실패: {error}"),
            }
        });
    }
}

#[cfg(test)]
#[path = "../../tests/unit/cache.rs"]
mod tests;
