//! Revision-bound SQLite WAL store with compressed event and recording payloads.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::replay::{Event, Recording};

const MIGRATION_V1: &str = r"
CREATE TABLE IF NOT EXISTS schema_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
INSERT OR REPLACE INTO schema_metadata(key, value) VALUES ('schema_version', '1');

CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    revision TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT
);

CREATE TABLE IF NOT EXISTS tests (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    module TEXT NOT NULL,
    last_status TEXT
);

CREATE TABLE IF NOT EXISTS dependencies (
    test_id TEXT NOT NULL REFERENCES tests(id) ON DELETE CASCADE,
    subject TEXT NOT NULL,
    kind TEXT NOT NULL,
    PRIMARY KEY(test_id, subject, kind)
);
CREATE INDEX IF NOT EXISTS dependencies_subject ON dependencies(subject);

CREATE TABLE IF NOT EXISTS events (
    event_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    test_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    payload_zstd BLOB NOT NULL,
    UNIQUE(run_id, sequence)
);
CREATE INDEX IF NOT EXISTS events_run_sequence ON events(run_id, sequence);

CREATE TABLE IF NOT EXISTS failures (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    test_id TEXT NOT NULL,
    exception_type TEXT NOT NULL,
    message TEXT NOT NULL,
    event_id TEXT,
    frames_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS failures_test ON failures(test_id);

CREATE TABLE IF NOT EXISTS coverage (
    test_id TEXT NOT NULL REFERENCES tests(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    line INTEGER NOT NULL,
    symbol TEXT NOT NULL,
    PRIMARY KEY(test_id, path, line, symbol)
);
CREATE INDEX IF NOT EXISTS coverage_path ON coverage(path, line);

CREATE TABLE IF NOT EXISTS recordings (
    id TEXT PRIMARY KEY,
    revision TEXT NOT NULL,
    test_id TEXT NOT NULL,
    backend TEXT NOT NULL,
    created_at TEXT NOT NULL,
    payload_zstd BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS verification_cache (
    revision TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    payload_zstd BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS revision_manifests (
    revision TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    payload_zstd BLOB NOT NULL
);
";

type FailureRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
);

/// Store failures that preserve their source category.
#[derive(Debug, Error)]
pub enum StoreError {
    /// SQLite operation failed.
    #[error("SQLite store error: {0}")]
    Sql(#[from] rusqlite::Error),
    /// Store directory or compression operation failed.
    #[error("store I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Stored JSON was invalid or incompatible.
    #[error("store JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// Mutex was poisoned by an earlier panic.
    #[error("store connection lock was poisoned")]
    LockPoisoned,
    /// Stored timestamp was not RFC 3339.
    #[error("invalid stored timestamp `{0}`")]
    InvalidTimestamp(String),
    /// A public unsigned size cannot fit SQLite's signed integer representation.
    #[error("integer `{0}` exceeds SQLite's supported range")]
    IntegerRange(u64),
}

/// Default age and size bounds for a worktree store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionPolicy {
    /// Maximum age of a run and its events.
    pub max_age: Duration,
    /// Approximate compressed payload ceiling.
    pub max_bytes: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_age: Duration::days(7),
            max_bytes: 1024 * 1024 * 1024,
        }
    }
}

/// New in-progress run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewRun {
    /// Run ID.
    pub id: String,
    /// Revision captured before worker launch.
    pub revision: String,
    /// Digest of source/resource/build input bytes.
    pub source_digest: String,
    /// Start time.
    pub started_at: DateTime<Utc>,
}

/// Persisted run state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    /// Worker is active.
    Running,
    /// Compilation and selected tests passed.
    Passed,
    /// Compilation or selected tests failed.
    Failed,
    /// Workspace changed while the worker was active.
    Stale,
    /// Worker or daemon failed.
    Error,
}

impl RunStatus {
    fn as_db(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Stale => "stale",
            Self::Error => "error",
        }
    }

    fn from_db(value: &str) -> Result<Self, rusqlite::Error> {
        match value {
            "running" => Ok(Self::Running),
            "passed" => Ok(Self::Passed),
            "failed" => Ok(Self::Failed),
            "stale" => Ok(Self::Stale),
            "error" => Ok(Self::Error),
            other => Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unknown run status `{other}`").into(),
            )),
        }
    }
}

/// Persisted run record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRecord {
    /// Run ID.
    pub id: String,
    /// Starting revision.
    pub revision: String,
    /// Canonical source digest.
    pub source_digest: String,
    /// Terminal or active state.
    pub status: RunStatus,
    /// Start time.
    pub started_at: DateTime<Utc>,
    /// Completion time.
    pub finished_at: Option<DateTime<Utc>>,
}

/// Discovered test metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestRecord {
    /// Public `class#method` ID.
    pub id: String,
    /// Framework display name.
    pub display_name: String,
    /// Gradle project path.
    pub module: String,
    /// Last execution result.
    pub last_status: Option<String>,
    /// Most recent structured failure for this test, when one exists.
    pub last_failure_id: Option<String>,
}

/// Structured failure indexed by stable ID and its nearest recorded event.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureRecord {
    /// Failure ID.
    pub id: String,
    /// Owning run.
    pub run_id: String,
    /// Public test ID.
    pub test_id: String,
    /// Throwable class.
    pub exception_type: String,
    /// Redacted bounded reason.
    pub message: String,
    /// Nearest trace event.
    pub event_id: Option<String>,
    /// Bounded logical stack frames.
    pub frames: Vec<String>,
}

/// One source line covered by a test.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoverageRecord {
    /// Public test ID.
    pub test_id: String,
    /// Workspace-relative source path.
    pub path: String,
    /// One-based line.
    pub line: u32,
    /// Logical symbol.
    pub symbol: String,
}

/// One observed test-to-production dependency.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dependency {
    /// Workspace path or JVM symbol.
    pub subject: String,
    /// `class`, `method`, `field`, `resource`, or `global`.
    pub kind: String,
}

/// Content and conservative source-surface digests for one successful revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevisionManifest {
    /// Full content digest by workspace-relative path.
    pub file_digests: BTreeMap<String, String>,
    /// ABI/global-initializer surface digest by workspace-relative path.
    pub surface_digests: BTreeMap<String, String>,
}

/// Safe impact decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImpactSelection {
    /// Known dependency graph produced a minimal list.
    Exact(Vec<String>),
    /// Unknown/global/build input forced all tests in the module.
    ModuleAll {
        /// Stable sorted test IDs.
        tests: Vec<String>,
        /// Explanation suitable for a diagnostic.
        reason: String,
    },
}

/// Bounded event result.
#[derive(Clone, Debug, PartialEq)]
pub struct EventPage {
    /// Ordered events.
    pub items: Vec<Event>,
    /// Exclusive sequence cursor, if more events remain.
    pub next_cursor: Option<String>,
}

/// Generic cursor page used by test and coverage queries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemPage<T> {
    /// Ordered items.
    pub items: Vec<T>,
    /// Exclusive cursor for the next page.
    pub next_cursor: Option<String>,
}

/// Retention operation summary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PruneResult {
    /// Runs removed, including cascaded events.
    pub runs_removed: usize,
    /// Expired recordings removed.
    pub recordings_removed: usize,
}

/// Worktree-local SQLite store.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    connection: Mutex<Connection>,
    retention: RetentionPolicy,
}

impl Store {
    /// Opens or initializes a store and enables WAL before returning.
    pub fn open(path: impl AsRef<Path>, retention: RetentionPolicy) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(&path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.execute_batch(MIGRATION_V1)?;
        connection.execute(
            "UPDATE runs SET status = ?1, finished_at = COALESCE(finished_at, ?2) WHERE status = ?3",
            params![
                RunStatus::Error.as_db(),
                Utc::now().to_rfc3339(),
                RunStatus::Running.as_db(),
            ],
        )?;
        Ok(Self {
            path,
            connection: Mutex::new(connection),
            retention,
        })
    }

    /// Returns SQLite's active journal mode.
    pub fn journal_mode(&self) -> Result<String, StoreError> {
        Ok(self
            .connection()?
            .pragma_query_value(None, "journal_mode", |row| row.get(0))?)
    }

    /// Inserts an active run.
    pub fn begin_run(&self, run: &NewRun) -> Result<(), StoreError> {
        self.connection()?.execute(
            "INSERT INTO runs(id, revision, source_digest, status, started_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                run.id,
                run.revision,
                run.source_digest,
                RunStatus::Running.as_db(),
                run.started_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Finishes a run, overriding the result with `stale` when revisions differ.
    pub fn finish_run(
        &self,
        run_id: &str,
        desired: RunStatus,
        current_revision: &str,
        finished_at: DateTime<Utc>,
    ) -> Result<RunStatus, StoreError> {
        let connection = self.connection()?;
        let starting_revision: String =
            connection.query_row("SELECT revision FROM runs WHERE id = ?1", [run_id], |row| {
                row.get(0)
            })?;
        let actual = if starting_revision == current_revision {
            desired
        } else {
            RunStatus::Stale
        };
        connection.execute(
            "UPDATE runs SET status = ?2, finished_at = ?3 WHERE id = ?1",
            params![run_id, actual.as_db(), finished_at.to_rfc3339()],
        )?;
        Ok(actual)
    }

    /// Reads one run.
    pub fn run(&self, run_id: &str) -> Result<Option<RunRecord>, StoreError> {
        let connection = self.connection()?;
        let raw = connection
            .query_row(
                "SELECT id, revision, source_digest, status, started_at, finished_at FROM runs WHERE id = ?1",
                [run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        RunStatus::from_db(&row.get::<_, String>(3)?)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()?;
        raw.map(
            |(id, revision, source_digest, status, started_at, finished_at)| {
                Ok(RunRecord {
                    id,
                    revision,
                    source_digest,
                    status,
                    started_at: parse_timestamp(&started_at)?,
                    finished_at: finished_at.as_deref().map(parse_timestamp).transpose()?,
                })
            },
        )
        .transpose()
    }

    /// Inserts one compressed event.
    pub fn append_event(
        &self,
        run_id: &str,
        test_id: &str,
        event: &Event,
    ) -> Result<(), StoreError> {
        let json = serde_json::to_vec(event)?;
        let compressed = zstd::stream::encode_all(json.as_slice(), 3)?;
        let sequence = sqlite_i64(event.sequence)?;
        self.connection()?.execute(
            "INSERT INTO events(event_id, run_id, test_id, sequence, payload_zstd) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![event.id, run_id, test_id, sequence, compressed],
        )?;
        Ok(())
    }

    /// Reads an ordered, byte-bounded event page.
    pub fn events(
        &self,
        run_id: &str,
        cursor: Option<&str>,
        limit: usize,
        max_bytes: usize,
    ) -> Result<EventPage, StoreError> {
        self.events_matching(run_id, None, cursor, limit, max_bytes)
    }

    /// Reads the most recent events for one test in chronological order.
    pub fn recent_test_events(
        &self,
        run_id: &str,
        test_id: &str,
        limit: usize,
        max_bytes: usize,
    ) -> Result<Vec<Event>, StoreError> {
        let fetch_limit =
            i64::try_from(limit.max(1)).map_err(|_| StoreError::IntegerRange(u64::MAX))?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT payload_zstd FROM events
             WHERE run_id = ?1 AND test_id = ?2
             ORDER BY sequence DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(params![run_id, test_id, fetch_limit], |row| {
            row.get::<_, Vec<u8>>(0)
        })?;
        let mut events = Vec::new();
        let mut bytes = 0_usize;
        for row in rows {
            let json = zstd::stream::decode_all(row?.as_slice())?;
            if !events.is_empty() && bytes.saturating_add(json.len()) > max_bytes {
                break;
            }
            bytes = bytes.saturating_add(json.len());
            events.push(serde_json::from_slice(&json)?);
        }
        events.reverse();
        Ok(events)
    }

    fn events_matching(
        &self,
        run_id: &str,
        test_id: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
        max_bytes: usize,
    ) -> Result<EventPage, StoreError> {
        let after = cursor
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(-1);
        let fetch_limit = limit.saturating_add(1).max(1);
        let fetch_limit =
            i64::try_from(fetch_limit).map_err(|_| StoreError::IntegerRange(u64::MAX))?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT sequence, payload_zstd FROM events
             WHERE run_id = ?1 AND (?2 IS NULL OR test_id = ?2) AND sequence > ?3
             ORDER BY sequence LIMIT ?4",
        )?;
        let rows = statement.query_map(params![run_id, test_id, after, fetch_limit], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let mut items: Vec<Event> = Vec::new();
        let mut bytes = 0_usize;
        let mut has_more = false;
        for row in rows {
            let (_sequence, compressed) = row?;
            if items.len() >= limit {
                has_more = true;
                break;
            }
            let json = zstd::stream::decode_all(compressed.as_slice())?;
            if !items.is_empty() && bytes.saturating_add(json.len()) > max_bytes {
                has_more = true;
                break;
            }
            bytes = bytes.saturating_add(json.len());
            items.push(serde_json::from_slice(&json)?);
        }
        let next_cursor = has_more
            .then(|| items.last().map(|event| event.sequence.to_string()))
            .flatten();
        Ok(EventPage { items, next_cursor })
    }

    /// Returns compressed payload bytes for storage regression tests and diagnostics.
    pub fn compressed_value_bytes(&self, event_id: &str) -> Result<usize, StoreError> {
        let bytes: i64 = self.connection()?.query_row(
            "SELECT length(payload_zstd) FROM events WHERE event_id = ?1",
            [event_id],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(bytes).unwrap_or(usize::MAX))
    }

    /// Inserts or updates test metadata.
    pub fn upsert_test(&self, test: &TestRecord) -> Result<(), StoreError> {
        self.connection()?.execute(
            "INSERT INTO tests(id, display_name, module, last_status) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, module = excluded.module, last_status = excluded.last_status",
            params![test.id, test.display_name, test.module, test.last_status],
        )?;
        Ok(())
    }

    /// Lists tests in stable ID order.
    pub fn tests(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ItemPage<TestRecord>, StoreError> {
        let after = cursor.unwrap_or("");
        let requested = limit.max(1);
        let fetch = i64::try_from(requested.saturating_add(1))
            .map_err(|_| StoreError::IntegerRange(u64::MAX))?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT tests.id, tests.display_name, tests.module, tests.last_status,
                    (SELECT failures.id FROM failures
                     WHERE failures.test_id = tests.id
                     ORDER BY failures.rowid DESC LIMIT 1)
             FROM tests WHERE tests.id > ?1 ORDER BY tests.id LIMIT ?2",
        )?;
        let mut items = statement
            .query_map(params![after, fetch], |row| {
                Ok(TestRecord {
                    id: row.get(0)?,
                    display_name: row.get(1)?,
                    module: row.get(2)?,
                    last_status: row.get(3)?,
                    last_failure_id: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = items.len() > requested;
        items.truncate(requested);
        let next_cursor = has_more
            .then(|| items.last().map(|test| test.id.clone()))
            .flatten();
        Ok(ItemPage { items, next_cursor })
    }

    /// Inserts or replaces one failure.
    pub fn save_failure(&self, failure: &FailureRecord) -> Result<(), StoreError> {
        self.connection()?.execute(
            "INSERT OR REPLACE INTO failures(id, run_id, test_id, exception_type, message, event_id, frames_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                failure.id,
                failure.run_id,
                failure.test_id,
                failure.exception_type,
                failure.message,
                failure.event_id,
                serde_json::to_string(&failure.frames)?,
            ],
        )?;
        Ok(())
    }

    /// Reads one structured failure.
    pub fn failure(&self, id: &str) -> Result<Option<FailureRecord>, StoreError> {
        let raw: Option<FailureRow> = self
            .connection()?
            .query_row(
                "SELECT id, run_id, test_id, exception_type, message, event_id, frames_json FROM failures WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
            )
            .optional()?;
        raw.map(
            |(id, run_id, test_id, exception_type, message, event_id, frames)| {
                Ok(FailureRecord {
                    id,
                    run_id,
                    test_id,
                    exception_type,
                    message,
                    event_id,
                    frames: serde_json::from_str(&frames)?,
                })
            },
        )
        .transpose()
    }

    /// Atomically replaces source coverage for one test.
    pub fn replace_coverage(
        &self,
        test_id: &str,
        coverage: &[CoverageRecord],
    ) -> Result<(), StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM coverage WHERE test_id = ?1", [test_id])?;
        for item in coverage {
            transaction.execute(
                "INSERT INTO coverage(test_id, path, line, symbol) VALUES (?1, ?2, ?3, ?4)",
                params![item.test_id, item.path, i64::from(item.line), item.symbol],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Queries coverage by test ID, path, or symbol.
    pub fn coverage(
        &self,
        subject: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ItemPage<CoverageRecord>, StoreError> {
        let after = cursor
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);
        let requested = limit.max(1);
        let fetch = i64::try_from(requested.saturating_add(1))
            .map_err(|_| StoreError::IntegerRange(u64::MAX))?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT rowid, test_id, path, line, symbol FROM coverage
             WHERE rowid > ?1 AND (test_id = ?2 OR path = ?2 OR symbol = ?2)
             ORDER BY rowid LIMIT ?3",
        )?;
        let mut rows = statement
            .query_map(params![after, subject, fetch], |row| {
                let line: i64 = row.get(3)?;
                Ok((
                    row.get::<_, i64>(0)?,
                    CoverageRecord {
                        test_id: row.get(1)?,
                        path: row.get(2)?,
                        line: u32::try_from(line).unwrap_or(u32::MAX),
                        symbol: row.get(4)?,
                    },
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = rows.len() > requested;
        rows.truncate(requested);
        let next_cursor = has_more
            .then(|| rows.last().map(|(rowid, _)| rowid.to_string()))
            .flatten();
        Ok(ItemPage {
            items: rows.into_iter().map(|(_, item)| item).collect(),
            next_cursor,
        })
    }

    /// Reads one event by stable ID.
    pub fn event(&self, event_id: &str) -> Result<Option<Event>, StoreError> {
        let compressed: Option<Vec<u8>> = self
            .connection()?
            .query_row(
                "SELECT payload_zstd FROM events WHERE event_id = ?1",
                [event_id],
                |row| row.get(0),
            )
            .optional()?;
        compressed
            .map(|value| {
                let json = zstd::stream::decode_all(value.as_slice())?;
                Ok(serde_json::from_slice(&json)?)
            })
            .transpose()
    }

    /// Reads a run trace, or the latest run containing a public test ID.
    pub fn trace(
        &self,
        subject: &str,
        cursor: Option<&str>,
        limit: usize,
        max_bytes: usize,
    ) -> Result<EventPage, StoreError> {
        let connection = self.connection()?;
        let run_exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE id = ?1)",
            [subject],
            |row| row.get(0),
        )?;
        if run_exists {
            drop(connection);
            return self.events(subject, cursor, limit, max_bytes);
        }
        let run_id: Option<String> = connection
            .query_row(
                "SELECT events.run_id FROM events JOIN runs ON runs.id = events.run_id
                 WHERE events.test_id = ?1 ORDER BY runs.started_at DESC LIMIT 1",
                [subject],
                |row| row.get(0),
            )
            .optional()?;
        drop(connection);
        match run_id {
            Some(run_id) => self.events_matching(&run_id, Some(subject), cursor, limit, max_bytes),
            None => Ok(EventPage {
                items: Vec::new(),
                next_cursor: None,
            }),
        }
    }

    /// Most recent revision that completed successfully and was not stale.
    pub fn latest_passed_revision(&self) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection()?
            .query_row(
                "SELECT revision FROM runs WHERE status = 'passed' ORDER BY finished_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Saves a successful default verification result under its exact content revision.
    pub fn save_verification_cache(
        &self,
        revision: &str,
        run_id: &str,
        payload: &[u8],
    ) -> Result<(), StoreError> {
        let compressed = zstd::stream::encode_all(payload, 3)?;
        self.connection()?.execute(
            "INSERT OR REPLACE INTO verification_cache(revision, run_id, created_at, payload_zstd)
             VALUES (?1, ?2, ?3, ?4)",
            params![revision, run_id, Utc::now().to_rfc3339(), compressed],
        )?;
        Ok(())
    }

    /// Loads a cached verification payload for an exact content revision.
    pub fn verification_cache(&self, revision: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let compressed: Option<Vec<u8>> = self
            .connection()?
            .query_row(
                "SELECT payload_zstd FROM verification_cache WHERE revision = ?1",
                [revision],
                |row| row.get(0),
            )
            .optional()?;
        compressed
            .map(|payload| Ok(zstd::stream::decode_all(payload.as_slice())?))
            .transpose()
    }

    /// Returns the most recently cached successful default verification revision.
    pub fn latest_cached_revision(&self) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection()?
            .query_row(
                "SELECT revision FROM verification_cache ORDER BY created_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Saves the file/surface manifest used by conservative impact selection.
    pub fn save_revision_manifest(
        &self,
        revision: &str,
        manifest: &RevisionManifest,
    ) -> Result<(), StoreError> {
        let json = serde_json::to_vec(manifest)?;
        let compressed = zstd::stream::encode_all(json.as_slice(), 3)?;
        self.connection()?.execute(
            "INSERT OR REPLACE INTO revision_manifests(revision, created_at, payload_zstd)
             VALUES (?1, ?2, ?3)",
            params![revision, Utc::now().to_rfc3339(), compressed],
        )?;
        Ok(())
    }

    /// Loads a successful revision manifest.
    pub fn revision_manifest(
        &self,
        revision: &str,
    ) -> Result<Option<RevisionManifest>, StoreError> {
        let compressed: Option<Vec<u8>> = self
            .connection()?
            .query_row(
                "SELECT payload_zstd FROM revision_manifests WHERE revision = ?1",
                [revision],
                |row| row.get(0),
            )
            .optional()?;
        compressed
            .map(|payload| {
                let json = zstd::stream::decode_all(payload.as_slice())?;
                Ok(serde_json::from_slice(&json)?)
            })
            .transpose()
    }

    /// Atomically replaces dependencies for one test.
    pub fn replace_dependencies(
        &self,
        test_id: &str,
        dependencies: &[Dependency],
    ) -> Result<(), StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM dependencies WHERE test_id = ?1", [test_id])?;
        for dependency in dependencies {
            transaction.execute(
                "INSERT INTO dependencies(test_id, subject, kind) VALUES (?1, ?2, ?3)",
                params![test_id, dependency.subject, dependency.kind],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Selects exact impacted tests, conservatively expanding unknown inputs.
    pub fn select_impact(
        &self,
        subject: &str,
        module: &str,
    ) -> Result<ImpactSelection, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT DISTINCT test_id FROM dependencies WHERE subject = ?1 ORDER BY test_id",
        )?;
        let exact = statement
            .query_map([subject], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        if !exact.is_empty() {
            return Ok(ImpactSelection::Exact(exact));
        }
        let mut statement =
            connection.prepare("SELECT id FROM tests WHERE module = ?1 ORDER BY id")?;
        let tests = statement
            .query_map([module], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ImpactSelection::ModuleAll {
            tests,
            reason: "unknown dependency or build/resource/global change".into(),
        })
    }

    /// Saves an immutable compressed recording.
    pub fn save_recording(&self, recording: &Recording) -> Result<(), StoreError> {
        let json = serde_json::to_vec(recording)?;
        let compressed = zstd::stream::encode_all(json.as_slice(), 6)?;
        self.connection()?.execute(
            "INSERT OR REPLACE INTO recordings(id, revision, test_id, backend, created_at, payload_zstd) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                recording.id,
                recording.revision,
                recording.test_id,
                recording.backend,
                Utc::now().to_rfc3339(),
                compressed,
            ],
        )?;
        Ok(())
    }

    /// Reads one recording.
    pub fn recording(&self, id: &str) -> Result<Option<Recording>, StoreError> {
        let compressed: Option<Vec<u8>> = self
            .connection()?
            .query_row(
                "SELECT payload_zstd FROM recordings WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        compressed
            .map(|value| {
                let json = zstd::stream::decode_all(value.as_slice())?;
                Ok(serde_json::from_slice(&json)?)
            })
            .transpose()
    }

    /// Applies age and compressed-size retention bounds.
    pub fn prune(&self, now: DateTime<Utc>) -> Result<PruneResult, StoreError> {
        let cutoff = now - self.retention.max_age;
        let connection = self.connection()?;
        let runs_removed = connection.execute(
            "DELETE FROM runs WHERE started_at < ?1",
            [cutoff.to_rfc3339()],
        )?;
        let mut recordings_removed = connection.execute(
            "DELETE FROM recordings WHERE created_at < ?1",
            [cutoff.to_rfc3339()],
        )?;
        connection.execute(
            "DELETE FROM revision_manifests WHERE created_at < ?1",
            [cutoff.to_rfc3339()],
        )?;

        let mut removed_for_size = 0;
        while payload_bytes(&connection)? > self.retention.max_bytes {
            let oldest: Option<(String, String)> = connection
                .query_row(
                    "SELECT kind, id FROM (
                        SELECT 'run' AS kind, id, started_at AS created_at FROM runs
                        UNION ALL
                        SELECT 'recording' AS kind, id, created_at FROM recordings
                        UNION ALL
                        SELECT 'manifest' AS kind, revision AS id, created_at FROM revision_manifests
                     ) ORDER BY created_at, kind, id LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((kind, id)) = oldest else { break };
            match kind.as_str() {
                "run" => {
                    removed_for_size +=
                        connection.execute("DELETE FROM runs WHERE id = ?1", [id])?;
                }
                "recording" => {
                    recordings_removed +=
                        connection.execute("DELETE FROM recordings WHERE id = ?1", [id])?;
                }
                "manifest" => {
                    connection
                        .execute("DELETE FROM revision_manifests WHERE revision = ?1", [id])?;
                }
                _ => unreachable!("retention query emits a fixed kind"),
            }
        }
        Ok(PruneResult {
            runs_removed: runs_removed + removed_for_size,
            recordings_removed,
        })
    }

    /// Store file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.connection.lock().map_err(|_| StoreError::LockPoisoned)
    }
}

fn payload_bytes(connection: &Connection) -> Result<u64, rusqlite::Error> {
    connection
        .query_row(
            "SELECT
            COALESCE((SELECT SUM(length(payload_zstd)) FROM events), 0) +
            COALESCE((SELECT SUM(length(payload_zstd)) FROM recordings), 0) +
            COALESCE((SELECT SUM(length(payload_zstd)) FROM verification_cache), 0) +
            COALESCE((SELECT SUM(length(payload_zstd)) FROM revision_manifests), 0)",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| u64::try_from(value).unwrap_or(u64::MAX))
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, StoreError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| StoreError::InvalidTimestamp(value.into()))
}

fn sqlite_i64(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::IntegerRange(value))
}
