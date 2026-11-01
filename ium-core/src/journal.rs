//! Crash-safe session journal — SQLite in WAL mode (pattern #6: Motrix's
//! session logs + FDM's `AbstractDownloadPersistentInfo`, rebuilt for uploads).
//!
//! Rule: the journal is the single source of truth. `bytes_confirmed` written
//! here must come from a server acknowledgment, never from client optimism.

use rusqlite::{params, Connection};
use std::path::Path;

use crate::chunk::ChunkState;
use crate::state::JobState;

const MIGRATIONS: &[&str] = &[
    // v1 — spec §6
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

pub struct SessionJournal {
    conn: Connection,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl SessionJournal {
    /// Open (creating or replaying) the journal at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        for m in MIGRATIONS {
            conn.execute_batch(m)?;
        }
        tracing::debug!("journal opened (WAL)");
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self, JournalError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        for m in MIGRATIONS {
            conn.execute_batch(m)?;
        }
        Ok(Self { conn })
    }

    pub fn schema_version(&self) -> Result<String, JournalError> {
        Ok(self.conn.query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |r| r.get(0),
        )?)
    }

    /// Persist a brand-new job in `Created` state together with its planned chunks.
    #[allow(clippy::too_many_arguments)] // flat SQL-shaped insert; called in one place
    pub fn insert_job(
        &self,
        id: &str,
        source_path: &str,
        source_size: u64,
        adapter: &str,
        endpoint: &str,
        capability_badge: &str,
        chunks: &[ChunkRow],
    ) -> Result<JobRecord, JournalError> {
        let ts = now_ms();
        let exists: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM jobs WHERE id=?1", params![id], |r| {
                    r.get(0)
                })?;
        if exists > 0 {
            return Err(JournalError::DuplicateJob(id.to_string()));
        }
        self.conn.execute(
            "INSERT INTO jobs (id, source_path, source_size, adapter, endpoint, state, capability_badge, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![id, source_path, source_size as i64, adapter, endpoint, JobState::Created.as_str(), capability_badge, ts],
        )?;
        self.replace_chunks(id, chunks)?;
        Ok(JobRecord {
            id: id.to_string(),
            source_path: source_path.to_string(),
            source_size,
            adapter: adapter.to_string(),
            endpoint: endpoint.to_string(),
            state: JobState::Created,
            capability_badge: capability_badge.to_string(),
            created_at: ts,
            updated_at: ts,
        })
    }

    /// Journal a state transition — called BEFORE the engine acts on it.
    pub fn set_job_state(&self, id: &str, state: JobState) -> Result<(), JournalError> {
        self.conn.execute(
            "UPDATE jobs SET state=?2, updated_at=?3 WHERE id=?1",
            params![id, state.as_str(), now_ms()],
        )?;
        tracing::debug!(job = id, state = state.as_str(), "state journaled");
        Ok(())
    }

    /// Replace the chunk plan for a job (used after resplit / rebuild).
    pub fn replace_chunks(&self, job_id: &str, chunks: &[ChunkRow]) -> Result<(), JournalError> {
        self.conn
            .execute("DELETE FROM chunks WHERE job_id=?1", params![job_id])?;
        for c in chunks {
            self.conn.execute(
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

    /// Persist a server-confirmed chunk checkpoint (the crash-safety heart).
    pub fn update_chunk(
        &self,
        job_id: &str,
        idx: u32,
        state: ChunkState,
        bytes_confirmed: u64,
        adapter_token: Option<&str>,
    ) -> Result<(), JournalError> {
        self.conn.execute(
            "UPDATE chunks SET state=?3, bytes_confirmed=?4, adapter_token=?5 WHERE job_id=?1 AND idx=?2",
            params![job_id, idx as i64, chunk_state_str(state), bytes_confirmed as i64, adapter_token],
        )?;
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
        let mut stmt = self.conn.prepare(
            "SELECT id, source_path, source_size, adapter, endpoint, state, capability_badge, created_at, updated_at
             FROM jobs WHERE id=?1",
        )?;
        let mut rows = stmt.query_map(params![id], |r| {
            Ok(JobRecord {
                id: r.get(0)?,
                source_path: r.get(1)?,
                source_size: r.get::<_, i64>(2)? as u64,
                adapter: r.get(3)?,
                endpoint: r.get(4)?,
                state: parse_state(&r.get::<_, String>(5)?),
                capability_badge: r.get(6)?,
                created_at: r.get(7)?,
                updated_at: r.get(8)?,
            })
        })?;
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
            "SELECT id, source_path, source_size, adapter, endpoint, state, capability_badge, created_at, updated_at
             FROM jobs ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(JobRecord {
                id: r.get(0)?,
                source_path: r.get(1)?,
                source_size: r.get::<_, i64>(2)? as u64,
                adapter: r.get(3)?,
                endpoint: r.get(4)?,
                state: parse_state(&r.get::<_, String>(5)?),
                capability_badge: r.get(6)?,
                created_at: r.get(7)?,
                updated_at: r.get(8)?,
            })
        })?;
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

fn parse_state(s: &str) -> JobState {
    match s {
        "probing" => JobState::Probing,
        "running" => JobState::Running,
        "paused" => JobState::Paused,
        "finishing" => JobState::Finishing,
        "done" => JobState::Done,
        "unresumable" => JobState::Unresumable,
        "failed" => JobState::Failed,
        _ => JobState::Created,
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
        j.insert_job(
            "JOB1",
            "/tmp/a.iso",
            200,
            "s3_multipart",
            "s3://bucket",
            "resumable",
            &sample_chunks(),
        )
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
        j.insert_job("JOB1", "p", 10, "tus", "https://t", "resumable", &[])
            .unwrap();
        assert!(matches!(
            j.insert_job("JOB1", "p", 10, "tus", "https://t", "resumable", &[]),
            Err(JournalError::DuplicateJob(_))
        ));
    }

    #[test]
    fn events_are_appended_in_order() {
        let j = SessionJournal::open_in_memory().unwrap();
        j.insert_job("JOB1", "p", 10, "tus", "https://t", "resumable", &[])
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
    fn wal_checkpoint_succeeds_on_file_backed_journal() {
        let dir = tempfile::tempdir().unwrap();
        let j = SessionJournal::open(dir.path().join("s.ium.db")).unwrap();
        j.insert_job("JOB1", "p", 10, "tus", "https://t", "resumable", &[])
            .unwrap();
        j.checkpoint_wal().unwrap();
        assert_eq!(j.schema_version().unwrap(), "1");
    }
}
