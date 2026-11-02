//! Crash-safe session journal — SQLite in WAL mode (pattern #6: Motrix's
//! session logs + FDM's `AbstractDownloadPersistentInfo`, rebuilt for uploads).
//!
//! Rule: the journal is the single source of truth. `bytes_confirmed` written
//! here must come from a server acknowledgment, never from client optimism.
//!
//! Durability mechanics:
//! * WAL + `synchronous=NORMAL` — commits survive process crash (power-loss
//!   safety is `FULL`; NORMAL is the standard trade-off for UI apps and what
//!   every download manager we studied actually ships).
//! * Schema migrations are versioned with `PRAGMA user_version` and applied
//!   inside a transaction: a crash mid-migration rolls back, and reopening
//!   replays only the migrations that never landed.

use rusqlite::{params, Connection};
use std::path::Path;

use crate::chunk::ChunkState;
use crate::state::JobState;

/// Ordered schema migrations. `MIGRATIONS[i]` brings the journal from
/// version `i` to version `i + 1`. Never edit an entry — append new ones.
const MIGRATIONS: &[&str] = &[
    // v1 — spec §6: jobs / chunks / events
    "CREATE TABLE IF NOT EXISTS meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', '1');

    CREATE TABLE IF NOT EXISTS jobs (
        id               TEXT PRIMARY KEY,
        source_path      TEXT NOT NULL,
        source_size      INTEGER NOT NULL,
        source_sha256    TEXT,
        adapter          TEXT NOT NULL,
        endpoint         TEXT NOT NULL,
        account_id       TEXT,
        state            TEXT NOT NULL,
        capability_badge TEXT NOT NULL,
        created_at       INTEGER NOT NULL,
        updated_at       INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS chunks (
        job_id          TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
        idx             INTEGER NOT NULL,
        offset_begin    INTEGER NOT NULL,
        offset_end      INTEGER NOT NULL,
        state           TEXT NOT NULL,
        bytes_confirmed INTEGER NOT NULL DEFAULT 0,
        adapter_token   TEXT,
        PRIMARY KEY (job_id, idx)
    );

    CREATE TABLE IF NOT EXISTS events (
        seq     INTEGER PRIMARY KEY AUTOINCREMENT,
        job_id  TEXT,
        ts      INTEGER NOT NULL,
        kind    TEXT NOT NULL,
        payload TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_events_job ON events(job_id, seq);",
];

#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("job {0} already exists")]
    DuplicateJob(String),
    #[error("job {0} not found in journal")]
    UnknownJob(String),
    #[error("job {0}: chunk {1} not found in journal")]
    UnknownChunk(String, u32),
    #[error("job {0} already has chunks")]
    ChunksExist(String),
    #[error("job {0} has no chunk rows in journal")]
    ChunksMissing(String),
    #[error("empty chunk plan for job {0}")]
    EmptyPlan(String),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct JobRecord {
    pub id: String,
    pub source_path: String,
    pub source_size: u64,
    pub adapter: String,
    pub endpoint: String,
    pub state: JobState,
    pub capability_badge: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChunkRow {
    pub idx: u32,
    pub begin: u64,
    pub end: u64,
    pub state: ChunkState,
    pub bytes_confirmed: u64,
    pub adapter_token: Option<String>,
}

/// Flat, borrow-heavy description of a job row to insert.
pub struct NewJob<'a> {
    pub id: &'a str,
    pub source_path: &'a str,
    pub source_size: u64,
    pub adapter: &'a str,
    pub endpoint: &'a str,
    pub capability_badge: &'a str,
    pub chunks: &'a [ChunkRow],
}

pub struct SessionJournal {
    conn: Connection,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

const JOB_COLUMNS: &str = "id, source_path, source_size, adapter, endpoint, state, capability_badge, created_at, updated_at";

impl SessionJournal {
    /// Open (creating or replaying) the journal at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        Self::migrate(conn)
    }

    pub fn open_in_memory() -> Result<Self, JournalError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        Self::migrate(conn)
    }

    /// Apply pending migrations, each in its own transaction, tracked by
    /// `PRAGMA user_version`. A crash mid-migration rolls back its whole
    /// transaction, so a partially-migrated journal can never be observed.
    fn migrate(conn: Connection) -> Result<Self, JournalError> {
        let mut version: u32 =
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as u32;
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let target = (i + 1) as u32;
            if version >= target {
                continue;
            }
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", target as i64)?;
            tx.commit()?;
            version = target;
            tracing::info!(schema_version = target, "journal migration applied");
        }
        tracing::debug!(schema_version = version, "journal opened");
        Ok(Self { conn })
    }

    pub fn schema_version(&self) -> Result<String, JournalError> {
        Ok(self.conn.query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |r| r.get(0),
        )?)
    }

    /// Persist a brand-new job in `Created` state together with its planned
    /// chunks. Atomic: a crash between the job row and its chunk plan leaves
    /// neither.
    pub fn insert_job(&self, job: NewJob<'_>) -> Result<JobRecord, JournalError> {
        // Issue #5 invariant: a job is committed WITH its plan, atomically —
        // an empty plan is a caller bug, refused before anything is journaled.
        if job.chunks.is_empty() {
            return Err(JournalError::EmptyPlan(job.id.to_string()));
        }
        let ts = now_ms();
        let tx = self.conn.unchecked_transaction()?;
        let exists: i64 = tx.query_row(
            "SELECT COUNT(*) FROM jobs WHERE id=?1",
            params![job.id],
            |r| r.get(0),
        )?;
        if exists > 0 {
            return Err(JournalError::DuplicateJob(job.id.to_string()));
        }
        tx.execute(
            "INSERT INTO jobs (id, source_path, source_size, adapter, endpoint, state, capability_badge, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![job.id, job.source_path, job.source_size as i64, job.adapter, job.endpoint, JobState::Created.as_str(), job.capability_badge, ts],
        )?;
        insert_chunks(&tx, job.id, job.chunks)?;
        tx.commit()?;
        Ok(JobRecord {
            id: job.id.to_string(),
            source_path: job.source_path.to_string(),
            source_size: job.source_size,
            adapter: job.adapter.to_string(),
            endpoint: job.endpoint.to_string(),
            state: JobState::Created,
            capability_badge: job.capability_badge.to_string(),
            created_at: ts,
            updated_at: ts,
        })
    }

    /// Journal a state transition — called BEFORE the engine acts on it.
    /// Fails loudly if the job row vanished (journal corruption / bad id).
    pub fn set_job_state(&self, id: &str, state: JobState) -> Result<(), JournalError> {
        let n = self.conn.execute(
            "UPDATE jobs SET state=?2, updated_at=?3 WHERE id=?1",
            params![id, state.as_str(), now_ms()],
        )?;
        if n == 0 {
            return Err(JournalError::UnknownJob(id.to_string()));
        }
        tracing::debug!(job = id, state = state.as_str(), "state journaled");
        Ok(())
    }

    /// Replace the chunk plan for a job (used after resplit / rebuild).
    /// Fails loudly for: an empty replacement plan (`EmptyPlan` — caller bug,
    /// checked before job lookup), unknown jobs, and jobs whose existing plan
    /// is missing (`ChunksMissing` — job + plan commit atomically, so 0 rows
    /// removed is journal corruption or an out-of-band deletion; issue #2,
    /// its documented special case collapsed by the #5 invariant).
    pub fn replace_chunks(&self, job_id: &str, chunks: &[ChunkRow]) -> Result<(), JournalError> {
        if chunks.is_empty() {
            return Err(JournalError::EmptyPlan(job_id.to_string()));
        }
        let job_exists: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE id=?1",
            params![job_id],
            |r| r.get(0),
        )?;
        if job_exists == 0 {
            return Err(JournalError::UnknownJob(job_id.to_string()));
        }
        let tx = self.conn.unchecked_transaction()?;
        let removed = tx.execute("DELETE FROM chunks WHERE job_id=?1", params![job_id])?;
        if removed == 0 {
            return Err(JournalError::ChunksMissing(job_id.to_string()));
        }
        insert_chunks(&tx, job_id, chunks)?;
        tx.commit()?;
        Ok(())
    }

    /// Persist a server-confirmed chunk checkpoint (the crash-safety heart).
    pub fn update_chunk(
        &self,
        job_id: &str,
        idx: u32,
        state: ChunkState,
        bytes_confirmed: u64,
        adapter_token: Option<&str>,
    ) -> Result<(), JournalError> {
        let n = self.conn.execute(
            "UPDATE chunks SET state=?3, bytes_confirmed=?4, adapter_token=?5 WHERE job_id=?1 AND idx=?2",
            params![job_id, idx as i64, chunk_state_str(state), bytes_confirmed as i64, adapter_token],
        )?;
        if n == 0 {
            return Err(JournalError::UnknownChunk(job_id.to_string(), idx));
        }
        Ok(())
    }

    /// Append a journal event (state transitions, confirms, retries...).
    pub fn append_event(
        &self,
        job_id: &str,
        kind: &str,
        payload: &serde_json::Value,
    ) -> Result<i64, JournalError> {
        self.conn.execute(
            "INSERT INTO events (job_id, ts, kind, payload) VALUES (?1, ?2, ?3, ?4)",
            params![job_id, now_ms(), kind, payload.to_string()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn load_job(&self, id: &str) -> Result<Option<JobRecord>, JournalError> {
        let mut stmt = self
            .conn
            .prepare(format!("SELECT {JOB_COLUMNS} FROM jobs WHERE id=?1").as_str())?;
        let mut rows = stmt.query_map(params![id], row_to_record)?;
        match rows.next() {
            Some(rec) => Ok(Some(rec?)),
            None => Ok(None),
        }
    }

    pub fn load_chunks(&self, job_id: &str) -> Result<Vec<ChunkRow>, JournalError> {
        let mut stmt = self.conn.prepare(
            "SELECT idx, offset_begin, offset_end, state, bytes_confirmed, adapter_token
             FROM chunks WHERE job_id=?1 ORDER BY idx",
        )?;
        let rows = stmt.query_map(params![job_id], |r| {
            Ok(ChunkRow {
                idx: r.get::<_, i64>(0)? as u32,
                begin: r.get::<_, i64>(1)? as u64,
                end: r.get::<_, i64>(2)? as u64,
                state: parse_chunk_state(&r.get::<_, String>(3)?),
                bytes_confirmed: r.get::<_, i64>(4)? as u64,
                adapter_token: r.get(5)?,
            })
        })?;
        let out: Vec<ChunkRow> = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(out)
    }

    pub fn list_jobs(&self) -> Result<Vec<JobRecord>, JournalError> {
        let mut stmt = self.conn.prepare(
            format!("SELECT {JOB_COLUMNS} FROM jobs ORDER BY created_at DESC, id").as_str(),
        )?;
        let rows = stmt.query_map([], row_to_record)?;
        let out: Vec<JobRecord> = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(out)
    }

    pub fn events_for(&self, job_id: &str) -> Result<Vec<(i64, String, String)>, JournalError> {
        let mut stmt = self
            .conn
            .prepare("SELECT seq, kind, payload FROM events WHERE job_id=?1 ORDER BY seq")?;
        let rows = stmt.query_map(params![job_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let out: Vec<(i64, String, String)> = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(out)
    }

    /// Flush WAL on graceful shutdown (spec §6).
    pub fn checkpoint_wal(&self) -> Result<(), JournalError> {
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }
}

fn insert_chunks(conn: &Connection, job_id: &str, chunks: &[ChunkRow]) -> Result<(), JournalError> {
    for c in chunks {
        conn.execute(
            "INSERT INTO chunks (job_id, idx, offset_begin, offset_end, state, bytes_confirmed, adapter_token)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                job_id,
                c.idx as i64,
                c.begin as i64,
                c.end as i64,
                chunk_state_str(c.state),
                c.bytes_confirmed as i64,
                c.adapter_token
            ],
        )?;
    }
    Ok(())
}

fn row_to_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<JobRecord> {
    Ok(JobRecord {
        id: r.get(0)?,
        source_path: r.get(1)?,
        source_size: r.get::<_, i64>(2)? as u64,
        adapter: r.get(3)?,
        endpoint: r.get(4)?,
        // Issue #15: an unknown state string is JOURNAL CORRUPTION, never a
        // silent Created (the old `_ => Created` fallback would have quietly
        // resurrected a canceled job as fresh — fail loudly instead).
        state: parse_state(&r.get::<_, String>(5)?).map_err(|bad| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                format!("unknown job state in the journal: {bad}").into(),
            )
        })?,
        capability_badge: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

fn chunk_state_str(s: ChunkState) -> &'static str {
    match s {
        ChunkState::Pending => "pending",
        ChunkState::Leased => "leased",
        ChunkState::Uploaded => "uploaded",
        ChunkState::Verified => "verified",
    }
}

fn parse_chunk_state(s: &str) -> ChunkState {
    match s {
        "leased" => ChunkState::Leased,
        "uploaded" => ChunkState::Uploaded,
        "verified" => ChunkState::Verified,
        _ => ChunkState::Pending,
    }
}

fn parse_state(s: &str) -> Result<JobState, String> {
    match s {
        "created" => Ok(JobState::Created),
        "probing" => Ok(JobState::Probing),
        "running" => Ok(JobState::Running),
        "paused" => Ok(JobState::Paused),
        "finishing" => Ok(JobState::Finishing),
        "done" => Ok(JobState::Done),
        "unresumable" => Ok(JobState::Unresumable),
        "failed" => Ok(JobState::Failed),
        "canceled" => Ok(JobState::Canceled),
        other => Err(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_chunks() -> Vec<ChunkRow> {
        vec![
            ChunkRow {
                idx: 0,
                begin: 0,
                end: 100,
                state: ChunkState::Pending,
                bytes_confirmed: 0,
                adapter_token: None,
            },
            ChunkRow {
                idx: 1,
                begin: 100,
                end: 200,
                state: ChunkState::Pending,
                bytes_confirmed: 0,
                adapter_token: None,
            },
        ]
    }

    #[test]
    fn job_and_chunks_roundtrip_replays_identically() {
        let j = SessionJournal::open_in_memory().unwrap();
        j.insert_job(NewJob {
            id: "JOB1",
            source_path: "/tmp/a.iso",
            source_size: 200,
            adapter: "s3_multipart",
            endpoint: "s3://bucket",
            capability_badge: "resumable",
            chunks: &sample_chunks(),
        })
        .unwrap();

        j.update_chunk("JOB1", 0, ChunkState::Uploaded, 100, Some("part-0"))
            .unwrap();
        j.set_job_state("JOB1", JobState::Running).unwrap();

        let job = j.load_job("JOB1").unwrap().unwrap();
        assert_eq!(job.state, JobState::Running);
        assert_eq!(job.capability_badge, "resumable");

        let chunks = j.load_chunks("JOB1").unwrap();
        assert_eq!(chunks[0].state, ChunkState::Uploaded);
        assert_eq!(chunks[0].bytes_confirmed, 100);
        assert_eq!(chunks[0].adapter_token.as_deref(), Some("part-0"));
        assert_eq!(chunks[1].bytes_confirmed, 0);
    }

    #[test]
    fn duplicate_job_is_rejected() {
        let j = SessionJournal::open_in_memory().unwrap();
        let chunks = sample_chunks();
        let job = || NewJob {
            id: "JOB1",
            source_path: "p",
            source_size: 200,
            adapter: "tus",
            endpoint: "https://t",
            capability_badge: "resumable",
            chunks: &chunks,
        };
        j.insert_job(job()).unwrap();
        assert!(matches!(
            j.insert_job(job()),
            Err(JournalError::DuplicateJob(_))
        ));
    }

    #[test]
    fn events_are_appended_in_order() {
        let j = SessionJournal::open_in_memory().unwrap();
        j.insert_job(NewJob {
            id: "JOB1",
            source_path: "p",
            source_size: 200,
            adapter: "tus",
            endpoint: "https://t",
            capability_badge: "resumable",
            chunks: &sample_chunks(),
        })
        .unwrap();
        let a = j
            .append_event("JOB1", "state", &serde_json::json!({"to": "running"}))
            .unwrap();
        let b = j
            .append_event("JOB1", "chunk", &serde_json::json!({"idx": 0}))
            .unwrap();
        assert!(a < b);
        let evs = j.events_for("JOB1").unwrap();
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].1, "state");
    }

    #[test]
    fn writes_to_missing_rows_fail_loudly() {
        let j = SessionJournal::open_in_memory().unwrap();
        assert!(matches!(
            j.set_job_state("GHOST", JobState::Running),
            Err(JournalError::UnknownJob(_))
        ));
        assert!(matches!(
            j.update_chunk("GHOST", 0, ChunkState::Uploaded, 1, None),
            Err(JournalError::UnknownChunk(_, 0))
        ));
    }

    #[test]
    fn migrations_are_versioned_and_reopen_safe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.ium.db");

        let first = {
            let j = SessionJournal::open(&path).unwrap();
            j.insert_job(NewJob {
                id: "JOB1",
                source_path: "p",
                source_size: 10,
                adapter: "tus",
                endpoint: "https://t",
                capability_badge: "resumable",
                chunks: &sample_chunks(),
            })
            .unwrap();
            j.update_chunk("JOB1", 0, ChunkState::Uploaded, 10, None)
                .unwrap();
            j.checkpoint_wal().unwrap();
            j.schema_version().unwrap()
        };
        assert_eq!(first, "1");

        // Reopen: migration must not re-run, data must survive.
        let j = SessionJournal::open(&path).unwrap();
        assert_eq!(j.schema_version().unwrap(), "1");
        let job = j.load_job("JOB1").unwrap().unwrap();
        assert_eq!(job.source_size, 10);
        assert_eq!(j.load_chunks("JOB1").unwrap()[0].bytes_confirmed, 10);
        // journal stays writable after reopen
        j.insert_job(NewJob {
            id: "JOB2",
            source_path: "p",
            source_size: 200,
            adapter: "tus",
            endpoint: "https://t",
            capability_badge: "resumable",
            chunks: &sample_chunks(),
        })
        .unwrap();
        assert_eq!(j.list_jobs().unwrap().len(), 2);
    }

    #[test]
    fn replace_chunks_rejects_unknown_job() {
        let j = SessionJournal::open_in_memory().unwrap();
        // Non-empty plan: an empty one is rejected as EmptyPlan BEFORE the
        // job lookup (issue #5 precedence — caller bug first), and a real
        // resplit always carries a plan.
        assert!(matches!(
            j.replace_chunks("GHOST", &sample_chunks()),
            Err(JournalError::UnknownJob(_))
        ));
    }

    #[test]
    fn insert_job_rejects_empty_plan() {
        let j = SessionJournal::open_in_memory().unwrap();
        // Issue #5: a job is committed with its plan, atomically — an empty
        // plan is a caller bug and must be refused, not journaled.
        assert!(matches!(
            j.insert_job(NewJob {
                id: "JOB1",
                source_path: "p",
                source_size: 0,
                adapter: "tus",
                endpoint: "https://t",
                capability_badge: "resumable",
                chunks: &[],
            }),
            Err(JournalError::EmptyPlan(_))
        ));
        assert!(j.load_job("JOB1").unwrap().is_none(), "nothing committed");
    }

    #[test]
    fn replace_chunks_rejects_empty_replacement_plan() {
        let j = SessionJournal::open_in_memory().unwrap();
        j.insert_job(NewJob {
            id: "JOB1",
            source_path: "p",
            source_size: 200,
            adapter: "tus",
            endpoint: "https://t",
            capability_badge: "resumable",
            chunks: &sample_chunks(),
        })
        .unwrap();
        // Issue #5: the #2 special case collapses — an empty replacement plan
        // is refused outright; and with a non-empty plan, 0 rows removed is
        // always ChunksMissing (covered by the plan-vanished test below).
        assert!(matches!(
            j.replace_chunks("JOB1", &[]),
            Err(JournalError::EmptyPlan(_))
        ));
        // The existing plan must be untouched by the refused call.
        assert_eq!(j.load_chunks("JOB1").unwrap().len(), 2);
    }

    #[test]
    fn replace_chunks_fails_loudly_when_plan_vanished() {
        let j = SessionJournal::open_in_memory().unwrap();
        j.insert_job(NewJob {
            id: "JOB1",
            source_path: "p",
            source_size: 200,
            adapter: "tus",
            endpoint: "https://t",
            capability_badge: "resumable",
            chunks: &sample_chunks(),
        })
        .unwrap();

        // Simulate out-of-band deletion / journal corruption: the job row
        // survives but its chunk plan does not (unreachable via journal paths,
        // which never delete plan rows outside replace_chunks).
        j.conn.execute("DELETE FROM chunks", []).unwrap();

        // Replacing a plan that no longer exists must be loud, not a silent
        // no-op that lets a resplit proceed on a corrupted journal (#2).
        // Non-empty replacement plan + 0 rows removed => ChunksMissing.
        assert!(matches!(
            j.replace_chunks("JOB1", &sample_chunks()),
            Err(JournalError::ChunksMissing(_))
        ));
    }
}
