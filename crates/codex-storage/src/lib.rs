//! SQLite storage with migrations, WAL, transactional upserts, and no content columns.

use anyhow::{Context, Result};
use chrono::Utc;
use codex_core::{ResumeState, Snapshot, TokenUsage};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::Path};

const MAX_DATABASE_PAGES: u32 = 131_072;
const DEFAULT_OTEL_EVENT_LIMIT: i64 = 100_000;
pub const MAX_OTEL_BATCH_EVENTS: usize = 256;
const MAX_OTEL_ATTRIBUTES_BYTES: usize = 8 * 1024;
const MAX_OTEL_TEXT_BYTES: usize = 512;

pub struct Store {
    conn: Connection,
    otel_event_limit: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestCheckpoint {
    pub byte_offset: i64,
    pub file_identity: Option<String>,
    pub resume_state: Option<ResumeState>,
    pub unparsed_events: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestPlan {
    pub checkpoint: IngestCheckpoint,
    pub rebuild_required: bool,
}

pub struct CommitIngest<'a> {
    pub path: &'a str,
    pub source_kind: &'a str,
    pub next_offset: i64,
    pub file_identity: Option<&'a str>,
    pub snapshot: &'a Snapshot,
    pub resume_state: &'a ResumeState,
    pub rebuild: bool,
    pub unparsed_events: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityFilter {
    /// `turns` is the default task-level view. `modelCalls` exposes token
    /// accounting events without pretending they carry task completion timing.
    pub view: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    pub project_path: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub result: Option<String>,
    pub search: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityRow {
    pub id: String,
    pub event_key: Option<String>,
    pub source_kind: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub occurred_at: Option<String>,
    pub project_path: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub provider: Option<String>,
    pub result: String,
    pub parent_turn_result: Option<String>,
    pub activity_kind: String,
    pub timing_scope: String,
    pub model_call_count: i64,
    pub duration_ms: Option<i64>,
    pub first_visible_output_ms: Option<i64>,
    pub usage: TokenUsage,
    pub usage_available: UsageAvailability,
    pub has_model_call: bool,
    pub status_code: Option<i64>,
    pub response_bytes: Option<i64>,
    pub endpoint: Option<String>,
    pub success: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageAvailability {
    pub input_tokens: bool,
    pub cached_input_tokens: bool,
    pub cache_write_input_tokens: bool,
    pub output_tokens: bool,
    pub reasoning_output_tokens: bool,
    pub total_tokens: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostInput {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_write_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub total_input_tokens: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHealthRow {
    pub canonical_path: String,
    pub source_kind: String,
    pub byte_offset: i64,
    pub file_identity: Option<String>,
    pub updated_at: String,
    pub last_error: Option<String>,
    pub unparsed_events: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredTurn {
    pub session_id: String,
    #[serde(skip)]
    cursor_session_id: String,
    pub turn_id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub provider: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub first_visible_output_ms: Option<i64>,
    pub result: String,
    pub usage: TokenUsage,
    pub model_call_count: i64,
    pub usage_confidence: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub total: i64,
}

#[derive(Clone, Debug)]
pub struct ActivitySnapshot {
    pub page: Page<ActivityRow>,
    pub revision: String,
}

#[derive(Clone, Debug)]
pub struct DashboardSnapshot {
    pub records: i64,
    pub successful: i64,
    pub failed: i64,
    pub model_calls: i64,
    pub cost_inputs: Vec<CostInput>,
    pub revision: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRow {
    pub canonical_path: String,
    pub name: String,
    pub source: String,
    pub exists: bool,
    pub is_git: bool,
    pub worktree: bool,
    pub last_seen_at: Option<String>,
    pub last_conversation_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsRow {
    pub codex_homes: Vec<String>,
    pub authorized_roots: Vec<String>,
    pub retention_days: i64,
    pub telemetry_enabled: bool,
    pub price_catalog_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionRow {
    pub id: String,
    pub path: String,
    pub created_at: String,
    pub before_sha256: String,
    pub after_sha256: String,
    pub byte_length: i64,
    pub before_content: Vec<u8>,
    pub after_content: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct RevisionDraft<'a> {
    pub id: &'a str,
    pub path: &'a str,
    pub created_at: &'a str,
    pub before_sha256: &'a str,
    pub after_sha256: &'a str,
    pub byte_length: i64,
    pub before_content: &'a [u8],
    pub after_content: &'a [u8],
}

#[derive(Clone, Debug)]
pub struct OtelMetadata<'a> {
    pub id: &'a str,
    pub signal: &'a str,
    pub event_name: Option<&'a str>,
    pub occurred_at: Option<&'a str>,
    pub thread_id: Option<&'a str>,
    pub turn_id: Option<&'a str>,
    pub model: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub status_code: Option<i64>,
    pub duration_ms: Option<i64>,
    pub response_bytes: Option<i64>,
    pub endpoint: Option<&'a str>,
    pub success: Option<bool>,
    pub input_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub cache_write_input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub attributes_json: &'a str,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OtelBatchResult {
    pub inserted: usize,
    pub duplicates: usize,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("创建应用数据目录失败")?;
            #[cfg(unix)]
            set_mode(parent, 0o700)?;
        };
        let conn = Connection::open(path).context("打开 SQLite 失败")?;
        #[cfg(unix)]
        set_mode(path, 0o600)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // Bound the complete local database, not only the high-volume OTel
        // table. SQLite rejects a write atomically with SQLITE_FULL once this
        // ceiling is reached, keeping existing metadata readable.
        conn.pragma_update(None, "max_page_count", MAX_DATABASE_PAGES)?;
        let store = Self {
            conn,
            otel_event_limit: DEFAULT_OTEL_EVENT_LIMIT,
        };
        store.migrate()?;
        Ok(store)
    }
    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)")?;
        self.apply_migration(1, include_str!("../migrations/0001_init.sql"))?;
        self.apply_migration(2, include_str!("../migrations/0002_incremental.sql"))?;
        self.apply_migration(3, include_str!("../migrations/0003_otel_activity.sql"))?;
        self.apply_migration(4, include_str!("../migrations/0004_revision_status.sql"))?;
        self.apply_migration(5, include_str!("../migrations/0005_source_health.sql"))?;
        self.apply_migration(6, include_str!("../migrations/0006_otel_endpoint.sql"))?;
        self.apply_migration(7, include_str!("../migrations/0007_source_namespace.sql"))?;
        self.apply_migration(
            8,
            include_str!("../migrations/0008_logical_fingerprints.sql"),
        )?;
        self.apply_migration(9, include_str!("../migrations/0009_canonical_activity.sql"))?;
        self.backfill_logical_fingerprints()?;
        Ok(())
    }

    fn backfill_logical_fingerprints(&self) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let turns = {
            let mut statement = tx.prepare(
                "SELECT t.session_id,t.turn_id,coalesce(s.public_session_id,t.session_id) FROM turns t JOIN sessions s ON s.session_id=t.session_id WHERE t.logical_fingerprint IS NULL",
            )?;
            statement
                .query_map([], |row| {
                    let internal_session_id: String = row.get(0)?;
                    let turn_id: String = row.get(1)?;
                    let public_session_id: String = row.get(2)?;
                    let value = serde_json::json!([public_session_id, turn_id]);
                    Ok((internal_session_id, turn_id, value))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (session_id, turn_id, value) in turns {
            tx.execute(
                "UPDATE turns SET logical_fingerprint=?3 WHERE session_id=?1 AND turn_id=?2",
                params![session_id, turn_id, logical_fingerprint("turn", &value)?],
            )?;
        }

        let calls = {
            let mut statement = tx.prepare(
                "SELECT c.event_key,coalesce(s.public_session_id,c.session_id),c.turn_id,coalesce(c.public_event_key,c.event_key),c.input_tokens,c.cached_input_tokens,c.cache_write_input_tokens,c.output_tokens,c.reasoning_output_tokens,c.total_tokens FROM model_calls c JOIN sessions s ON s.session_id=c.session_id WHERE c.logical_fingerprint IS NULL",
            )?;
            statement
                .query_map([], |row| {
                    let internal_event_key: String = row.get(0)?;
                    let value = serde_json::json!([
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i64>(9)?,
                    ]);
                    Ok((internal_event_key, value))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (event_key, value) in calls {
            tx.execute(
                "UPDATE model_calls SET logical_fingerprint=?2 WHERE event_key=?1",
                params![event_key, logical_fingerprint("model-call", &value)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    fn apply_migration(&self, version: i64, sql: &str) -> Result<()> {
        let applied = self
            .conn
            .query_row(
                "SELECT 1 FROM schema_migrations WHERE version=?1",
                params![version],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !applied {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(sql)
                .with_context(|| format!("执行 SQLite migration {version} 失败"))?;
            tx.execute(
                "INSERT INTO schema_migrations(version, applied_at) VALUES(?1, ?2)",
                params![version, Utc::now().to_rfc3339()],
            )?;
            tx.commit()?;
        }
        Ok(())
    }
    pub fn checkpoint(&self, path: &str) -> Result<i64> {
        Ok(self.ingest_checkpoint(path)?.byte_offset)
    }
    pub fn ingest_checkpoint(&self, path: &str) -> Result<IngestCheckpoint> {
        Ok(self
            .conn
            .query_row(
                "SELECT byte_offset,file_identity,resume_state_json,unparsed_events FROM ingest_files WHERE canonical_path=?1",
                params![path],
                |r| Ok(IngestCheckpoint { byte_offset:r.get(0)?, file_identity:r.get(1)?, resume_state:r.get::<_,Option<String>>(2)?.and_then(|value| serde_json::from_str(&value).ok()), unparsed_events:r.get(3)? }),
            )
            .optional()?
            .unwrap_or_default())
    }
    pub fn plan_ingest(
        &self,
        path: &str,
        file_identity: Option<&str>,
        file_size: i64,
    ) -> Result<IngestPlan> {
        let checkpoint = self.ingest_checkpoint(path)?;
        let identity_changed = checkpoint
            .file_identity
            .as_deref()
            .zip(file_identity)
            .map(|(old, new)| old != new)
            .unwrap_or(false);
        Ok(IngestPlan {
            rebuild_required: checkpoint.byte_offset > file_size || identity_changed,
            checkpoint,
        })
    }
    pub fn set_checkpoint(
        &self,
        path: &str,
        source_kind: &str,
        offset: i64,
        file_identity: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        self.conn.execute("INSERT INTO ingest_files(canonical_path,source_kind,byte_offset,file_identity,updated_at,last_error,resume_state_json) VALUES(?1,?2,?3,?4,?5,?6,NULL) ON CONFLICT(canonical_path) DO UPDATE SET byte_offset=excluded.byte_offset,file_identity=excluded.file_identity,updated_at=excluded.updated_at,last_error=excluded.last_error,resume_state_json=NULL",params![path,source_kind,offset,file_identity,Utc::now().to_rfc3339(),error])?;
        Ok(())
    }
    pub fn persist_snapshot(&mut self, source_path: &str, snapshot: &Snapshot) -> Result<()> {
        let tx = self.conn.transaction()?;
        let namespace = source_namespace(source_path, None);
        persist_snapshot_tx(&tx, source_path, &namespace, snapshot)?;
        tx.commit()?;
        Ok(())
    }
    pub fn commit_ingest(&mut self, input: CommitIngest<'_>) -> Result<()> {
        let tx = self.conn.transaction()?;
        if input.rebuild {
            delete_source_namespace(&tx, input.path)?;
            tx.execute(
                "DELETE FROM ingest_files WHERE canonical_path=?1",
                params![input.path],
            )?;
        }
        let namespace = source_namespace(input.path, input.file_identity);
        persist_snapshot_tx(&tx, input.path, &namespace, input.snapshot)?;
        tx.execute("INSERT INTO ingest_files(canonical_path,source_kind,byte_offset,file_identity,updated_at,last_error,resume_state_json,unparsed_events) VALUES(?1,?2,?3,?4,?5,NULL,?6,?7) ON CONFLICT(canonical_path) DO UPDATE SET source_kind=excluded.source_kind,byte_offset=excluded.byte_offset,file_identity=excluded.file_identity,updated_at=excluded.updated_at,last_error=NULL,resume_state_json=excluded.resume_state_json,unparsed_events=excluded.unparsed_events",params![input.path,input.source_kind,input.next_offset,input.file_identity,Utc::now().to_rfc3339(),serde_json::to_string(input.resume_state)?,input.unparsed_events.max(0)])?;
        tx.commit()?;
        Ok(())
    }
    pub fn mark_ingest_error(&self, path: &str, message: &str) -> Result<()> {
        self.conn.execute("INSERT INTO ingest_files(canonical_path,source_kind,byte_offset,file_identity,updated_at,last_error,resume_state_json,unparsed_events) VALUES(?1,'rollout',0,NULL,?3,?2,NULL,1) ON CONFLICT(canonical_path) DO UPDATE SET last_error=excluded.last_error,updated_at=excluded.updated_at,unparsed_events=ingest_files.unparsed_events+1",params![path,message,Utc::now().to_rfc3339()])?;
        Ok(())
    }
    pub fn rebuild_source(&mut self, path: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        delete_source_namespace(&tx, path)?;
        tx.execute(
            "DELETE FROM ingest_files WHERE canonical_path=?1",
            params![path],
        )?;
        tx.commit()?;
        Ok(())
    }
}

fn activity_sql(filter: &ActivityFilter) -> (String, Vec<String>) {
    let canonical = "WITH raw_turns AS (\
SELECT t.*,coalesce(s.public_session_id,t.session_id) AS public_session_id,\
ROW_NUMBER() OVER (PARTITION BY coalesce(t.logical_fingerprint,'legacy:' || t.session_id || ':' || t.turn_id) \
ORDER BY CASE WHEN t.result IN ('completed','failed','aborted') THEN 1 ELSE 0 END DESC,\
t.completed_at IS NOT NULL DESC,t.duration_ms IS NOT NULL DESC,t.model_call_count DESC,t.total_tokens DESC,t.session_id,t.turn_id) AS canonical_rank \
FROM turns t JOIN sessions s ON s.session_id=t.session_id),\
canonical_turns AS (SELECT r.*,CASE WHEN r.result='running' AND (EXISTS(SELECT 1 FROM raw_turns later WHERE later.public_session_id=r.public_session_id AND later.turn_id<>r.turn_id AND coalesce(later.started_at,later.completed_at,'') > coalesce(r.started_at,r.completed_at,'')) OR NOT EXISTS(SELECT 1 FROM sessions source JOIN ingest_files f ON f.canonical_path=source.source_path WHERE source.session_id=r.session_id) OR EXISTS(SELECT 1 FROM sessions source WHERE source.session_id=r.session_id AND lower(replace(source.source_path,char(92),'/')) LIKE '%/archived_sessions/%')) THEN 'unobserved' ELSE r.result END AS canonical_result FROM raw_turns r WHERE r.canonical_rank=1),\
raw_calls AS (SELECT c.*,coalesce(s.public_session_id,c.session_id) AS public_session_id,ROW_NUMBER() OVER (PARTITION BY coalesce(c.logical_fingerprint,'legacy:' || c.event_key) ORDER BY (c.model IS NOT NULL)+(c.effort IS NOT NULL)+(c.provider IS NOT NULL)+(c.occurred_at IS NOT NULL) DESC,c.event_key) AS canonical_rank FROM model_calls c JOIN sessions s ON s.session_id=c.session_id),\
canonical_calls AS (SELECT * FROM raw_calls WHERE canonical_rank=1),\
call_usage AS (SELECT public_session_id,turn_id,sum(input_tokens) AS input_tokens,sum(cached_input_tokens) AS cached_input_tokens,sum(cache_write_input_tokens) AS cache_write_input_tokens,sum(output_tokens) AS output_tokens,sum(reasoning_output_tokens) AS reasoning_output_tokens,sum(total_tokens) AS total_tokens,count(*) AS model_call_count FROM canonical_calls GROUP BY public_session_id,turn_id),\
activity(id,event_key,source_kind,session_id,turn_id,occurred_at,cwd,model,effort,provider,result,parent_turn_result,activity_kind,timing_scope,model_call_count,duration_ms,first_visible_output_ms,input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens,input_available,cached_available,cache_write_available,output_available,reasoning_available,total_available,has_model_call,status_code,response_bytes,endpoint,success) AS (";
    let turn_select = "SELECT 'turn:' || t.public_session_id || ':' || t.turn_id,NULL,'rollout',t.public_session_id,t.turn_id,coalesce(t.completed_at,t.started_at),t.cwd,t.model,t.effort,t.provider,t.canonical_result,NULL,'turn','turn',coalesce(u.model_call_count,0),t.duration_ms,t.first_visible_output_ms,coalesce(u.input_tokens,0),coalesce(u.cached_input_tokens,0),coalesce(u.cache_write_input_tokens,0),coalesce(u.output_tokens,0),coalesce(u.reasoning_output_tokens,0),coalesce(u.total_tokens,0),u.model_call_count IS NOT NULL,u.model_call_count IS NOT NULL,u.model_call_count IS NOT NULL,u.model_call_count IS NOT NULL,u.model_call_count IS NOT NULL,u.model_call_count IS NOT NULL,u.model_call_count IS NOT NULL,NULL,NULL,NULL,NULL FROM canonical_turns t LEFT JOIN call_usage u ON u.public_session_id=t.public_session_id AND u.turn_id=t.turn_id";
    let call_select = "SELECT c.event_key,coalesce(c.public_event_key,c.event_key),'rollout',c.public_session_id,c.turn_id,c.occurred_at,t.cwd,coalesce(c.model,t.model),coalesce(c.effort,t.effort),coalesce(c.provider,t.provider),'accounted',t.canonical_result,'modelCall','unavailable',1,NULL,NULL,c.input_tokens,c.cached_input_tokens,c.cache_write_input_tokens,c.output_tokens,c.reasoning_output_tokens,c.total_tokens,1,1,1,1,1,1,1,NULL,NULL,NULL,NULL FROM canonical_calls c LEFT JOIN canonical_turns t ON t.public_session_id=c.public_session_id AND t.turn_id=c.turn_id";
    let otel_select = "SELECT 'otel:' || o.id,o.id,'otel',o.thread_id,o.turn_id,coalesce(o.occurred_at,o.received_at),NULL,o.model,NULL,o.provider,CASE WHEN o.status_code BETWEEN 200 AND 399 THEN 'completed' WHEN o.status_code IS NOT NULL THEN 'failed' WHEN o.success=1 THEN 'completed' WHEN o.success=0 THEN 'failed' ELSE 'unknown' END,NULL,'otelRequest','request',1,o.duration_ms,NULL,coalesce(o.input_tokens,0),coalesce(o.cached_input_tokens,0),coalesce(o.cache_write_input_tokens,0),coalesce(o.output_tokens,0),coalesce(o.reasoning_output_tokens,0),coalesce(o.total_tokens,0),o.input_tokens IS NOT NULL,o.cached_input_tokens IS NOT NULL,o.cache_write_input_tokens IS NOT NULL,o.output_tokens IS NOT NULL,o.reasoning_output_tokens IS NOT NULL,o.total_tokens IS NOT NULL,0,o.status_code,o.response_bytes,o.endpoint,o.success FROM otel_events o WHERE o.signal='logs' AND o.event_name='codex.api_request'";
    let requested_view = filter.view.as_deref().unwrap_or("turns");
    let mut sql = if matches!(requested_view, "modelCalls" | "model_calls") {
        format!("{canonical}{call_select} UNION ALL {otel_select})")
    } else {
        format!("{canonical}{turn_select})")
    };
    sql.push_str(" SELECT id,event_key,source_kind,session_id,turn_id,occurred_at,cwd,model,effort,provider,result,parent_turn_result,activity_kind,timing_scope,model_call_count,duration_ms,first_visible_output_ms,input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens,input_available,cached_available,cache_write_available,output_available,reasoning_available,total_available,has_model_call,status_code,response_bytes,endpoint,success FROM activity WHERE 1=1");
    let mut values = Vec::new();
    for (clause, value) in [
        (" AND cwd=?", filter.project_path.as_deref()),
        (" AND model=?", filter.model.as_deref()),
        (" AND effort=?", filter.effort.as_deref()),
    ] {
        if let Some(value) = value {
            sql.push_str(clause);
            values.push(value.into());
        }
    }
    match filter.result.as_deref() {
        Some("failure") => {
            sql.push_str(" AND coalesce(parent_turn_result,result) IN ('failed','aborted')")
        }
        Some("success") => sql.push_str(" AND coalesce(parent_turn_result,result)='completed'"),
        Some(value @ ("running" | "unobserved" | "unknown")) => {
            sql.push_str(" AND coalesce(parent_turn_result,result)=?");
            values.push(value.into())
        }
        Some(value) => {
            sql.push_str(" AND result=?");
            values.push(value.into())
        }
        None => {}
    }
    if let Some(search) = filter.search.as_deref().filter(|value| !value.is_empty()) {
        let escaped = like_escape(search);
        sql.push_str(" AND (lower(coalesce(cwd,'')) LIKE ? ESCAPE '\\' OR lower(coalesce(session_id,'')) LIKE ? ESCAPE '\\' OR lower(coalesce(turn_id,'')) LIKE ? ESCAPE '\\' OR lower(coalesce(model,'')) LIKE ? ESCAPE '\\' OR lower(coalesce(effort,'')) LIKE ? ESCAPE '\\' OR lower(coalesce(provider,'')) LIKE ? ESCAPE '\\' OR lower(coalesce(endpoint,'')) LIKE ? ESCAPE '\\')");
        for _ in 0..7 {
            values.push(format!("%{}%", escaped.to_lowercase()));
        }
    }
    if let Some(cursor) = filter.cursor.as_deref() {
        sql.push_str(" AND (coalesce(occurred_at,'') || char(31) || id) < ?");
        values.push(cursor.into());
    }
    (sql, values)
}
fn like_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
fn activity_cursor(row: &ActivityRow) -> String {
    format!(
        "{}\u{1f}{}",
        row.occurred_at.as_deref().unwrap_or(""),
        row.id
    )
}
fn row_activity(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActivityRow> {
    Ok(ActivityRow {
        id: row.get(0)?,
        event_key: row.get(1)?,
        source_kind: row.get(2)?,
        session_id: row.get(3)?,
        turn_id: row.get(4)?,
        occurred_at: row.get(5)?,
        project_path: row.get(6)?,
        model: row.get(7)?,
        effort: row.get(8)?,
        provider: row.get(9)?,
        result: row.get(10)?,
        parent_turn_result: row.get(11)?,
        activity_kind: row.get(12)?,
        timing_scope: row.get(13)?,
        model_call_count: row.get(14)?,
        duration_ms: row.get(15)?,
        first_visible_output_ms: row.get(16)?,
        usage: TokenUsage {
            input_tokens: row.get(17)?,
            cached_input_tokens: row.get(18)?,
            cache_write_input_tokens: row.get(19)?,
            output_tokens: row.get(20)?,
            reasoning_output_tokens: row.get(21)?,
            total_tokens: row.get(22)?,
        },
        usage_available: UsageAvailability {
            input_tokens: row.get(23)?,
            cached_input_tokens: row.get(24)?,
            cache_write_input_tokens: row.get(25)?,
            output_tokens: row.get(26)?,
            reasoning_output_tokens: row.get(27)?,
            total_tokens: row.get(28)?,
        },
        has_model_call: row.get(29)?,
        status_code: row.get(30)?,
        response_bytes: row.get(31)?,
        endpoint: row.get(32)?,
        success: row.get(33)?,
    })
}

fn source_namespace(source_path: &str, file_identity: Option<&str>) -> String {
    let stable = file_identity.unwrap_or(source_path);
    let kind = if file_identity.is_some() {
        "file"
    } else {
        "path"
    };
    format!("{kind}:{}", hex::encode(Sha256::digest(stable.as_bytes())))
}

fn internal_key(namespace: &str, label: &str, payload: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    hasher.update([0]);
    hasher.update(label.as_bytes());
    hasher.update([0]);
    hasher.update(payload.as_bytes());
    format!("{label}:{}", hex::encode(hasher.finalize()))
}

fn logical_fingerprint(label: &str, value: &serde_json::Value) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(b"codex-manager-canonical-activity-v2\0");
    hasher.update(label.as_bytes());
    hasher.update([0]);
    hasher.update(serde_json::to_vec(value)?);
    Ok(format!("{label}:{}", hex::encode(hasher.finalize())))
}

fn turn_fingerprint(turn: &codex_core::Turn) -> Result<String> {
    // A copied rollout can enrich timing/model data at a different offset. A
    // turn's public session and turn id are the only stable task identity.
    logical_fingerprint("turn", &serde_json::json!([turn.session_id, turn.turn_id]))
}

fn model_call_fingerprint(call: &codex_core::ModelCall) -> Result<String> {
    logical_fingerprint(
        "model-call",
        &serde_json::json!([
            call.session_id,
            call.turn_id,
            call.event_key,
            call.usage.input_tokens,
            call.usage.cached_input_tokens,
            call.usage.cache_write_input_tokens,
            call.usage.output_tokens,
            call.usage.reasoning_output_tokens,
            call.usage.total_tokens,
        ]),
    )
}

fn delete_source_namespace(tx: &Transaction<'_>, path: &str) -> Result<()> {
    let previous_identity = tx
        .query_row(
            "SELECT file_identity FROM ingest_files WHERE canonical_path=?1",
            params![path],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let mut namespaces = HashSet::new();
    namespaces.insert(source_namespace(path, previous_identity.as_deref()));
    let mut statement = tx.prepare(
        "SELECT DISTINCT source_namespace FROM sessions WHERE source_path=?1 AND source_namespace IS NOT NULL",
    )?;
    for namespace in statement.query_map(params![path], |row| row.get::<_, String>(0))? {
        namespaces.insert(namespace?);
    }
    drop(statement);
    for namespace in namespaces {
        let mut aliases = tx.prepare(
            "SELECT canonical_path,file_identity FROM ingest_files WHERE canonical_path<>?1",
        )?;
        let mut namespace_is_referenced = false;
        for alias in aliases.query_map(params![path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })? {
            let (alias_path, alias_identity) = alias?;
            if source_namespace(&alias_path, alias_identity.as_deref()) == namespace {
                namespace_is_referenced = true;
                break;
            }
        }
        drop(aliases);
        if namespace_is_referenced {
            continue;
        }
        tx.execute(
            "DELETE FROM sessions WHERE source_namespace=?1",
            params![namespace],
        )?;
    }
    Ok(())
}

fn ensure_usage(usage: &TokenUsage, context: &str) -> Result<()> {
    if usage.is_valid() {
        Ok(())
    } else {
        anyhow::bail!("{context} 包含超出安全范围的 token 计数")
    }
}

fn validate_otel_text(value: Option<&str>, field: &str) -> Result<()> {
    if value.is_some_and(|text| text.len() > MAX_OTEL_TEXT_BYTES) {
        anyhow::bail!("OTel {field} 超过元数据长度上限")
    }
    Ok(())
}

fn validate_otel_event(event: &OtelMetadata<'_>) -> Result<()> {
    if event.id.is_empty() || event.id.len() > MAX_OTEL_TEXT_BYTES {
        anyhow::bail!("OTel event id 无效")
    }
    if event.signal.is_empty() || event.signal.len() > 32 {
        anyhow::bail!("OTel signal 无效")
    }
    for (value, field) in [
        (event.event_name, "event name"),
        (event.occurred_at, "occurred at"),
        (event.thread_id, "thread id"),
        (event.turn_id, "turn id"),
        (event.model, "model"),
        (event.provider, "provider"),
        (event.endpoint, "endpoint"),
    ] {
        validate_otel_text(value, field)?;
    }
    if event.attributes_json.len() > MAX_OTEL_ATTRIBUTES_BYTES
        || !serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
            event.attributes_json,
        )
        .is_ok()
    {
        anyhow::bail!("OTel attributes 不是受限 JSON object")
    }
    let usage = TokenUsage {
        input_tokens: event.input_tokens.unwrap_or_default(),
        cached_input_tokens: event.cached_input_tokens.unwrap_or_default(),
        cache_write_input_tokens: event.cache_write_input_tokens.unwrap_or_default(),
        output_tokens: event.output_tokens.unwrap_or_default(),
        reasoning_output_tokens: event.reasoning_output_tokens.unwrap_or_default(),
        total_tokens: event.total_tokens.unwrap_or_default(),
    };
    if [
        event.input_tokens,
        event.cached_input_tokens,
        event.cache_write_input_tokens,
        event.output_tokens,
        event.reasoning_output_tokens,
        event.total_tokens,
    ]
    .into_iter()
    .flatten()
    .any(|value| !(0..=TokenUsage::MAX_CALL_FIELD_VALUE).contains(&value))
        || !usage.is_valid()
    {
        anyhow::bail!("OTel token 计数超出安全范围")
    }
    if matches!(
        (
            event.input_tokens,
            event.cached_input_tokens,
            event.cache_write_input_tokens,
        ),
        (Some(input), Some(cached), Some(written))
            if cached.checked_add(written).is_none_or(|sum| sum > input)
    ) || matches!(
        (event.output_tokens, event.reasoning_output_tokens),
        (Some(output), Some(reasoning)) if reasoning > output
    ) || matches!(
        (event.input_tokens, event.output_tokens, event.total_tokens),
        (Some(input), Some(output), Some(total))
            if input.checked_add(output) != Some(total)
    ) {
        anyhow::bail!("OTel token 计数向量内部不一致")
    }
    for (value, field) in [
        (event.duration_ms, "duration"),
        (event.response_bytes, "response bytes"),
    ] {
        if value.is_some_and(|number| number < 0) {
            anyhow::bail!("OTel {field} 不能为负数")
        }
    }
    Ok(())
}

fn persist_snapshot_tx(
    tx: &Transaction<'_>,
    source_path: &str,
    namespace: &str,
    snapshot: &Snapshot,
) -> Result<()> {
    for turn in &snapshot.turns {
        ensure_usage(&turn.usage, "turn")?;
    }
    for call in &snapshot.model_calls {
        ensure_usage(&call.usage, "model call")?;
        ensure_usage(&call.cumulative_usage, "model call cumulative")?;
    }
    if let Some(s) = &snapshot.session {
        let internal_session_id = internal_key(namespace, "session", &s.session_id);
        tx.execute("INSERT INTO sessions(session_id,thread_id,cli_version,cwd,provider,started_at,source_path,observed_at,public_session_id,source_namespace) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(session_id) DO UPDATE SET thread_id=excluded.thread_id,cli_version=coalesce(excluded.cli_version,sessions.cli_version),cwd=coalesce(excluded.cwd,sessions.cwd),provider=coalesce(excluded.provider,sessions.provider),observed_at=excluded.observed_at",params![internal_session_id,s.thread_id,s.cli_version,s.cwd,s.model_provider,s.started_at,source_path,Utc::now().to_rfc3339(),s.session_id,namespace])?;
    }
    for t in &snapshot.turns {
        upsert_turn(tx, namespace, t)?;
    }
    for c in &snapshot.model_calls {
        let session_id = internal_key(namespace, "session", &c.session_id);
        let event_key = internal_key(namespace, "model-call", &c.event_key);
        let fingerprint = model_call_fingerprint(c)?;
        tx.execute("INSERT INTO model_calls(event_key,session_id,turn_id,ordinal,occurred_at,model,effort,provider,input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens,usage_confidence,public_event_key,logical_fingerprint) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17) ON CONFLICT(event_key) DO UPDATE SET logical_fingerprint=coalesce(model_calls.logical_fingerprint,excluded.logical_fingerprint)",params![event_key,session_id,c.turn_id,c.ordinal,c.occurred_at,c.model,c.effort,c.provider,c.usage.input_tokens,c.usage.cached_input_tokens,c.usage.cache_write_input_tokens,c.usage.output_tokens,c.usage.reasoning_output_tokens,c.usage.total_tokens,c.usage_confidence,c.event_key,fingerprint])?;
    }
    for i in &snapshot.timeline {
        let session_id = internal_key(namespace, "session", &i.session_id);
        let event_key = internal_key(namespace, "timeline", &i.event_key);
        tx.execute("INSERT OR IGNORE INTO timeline_items(event_key,session_id,turn_id,ordinal,occurred_at,item_type,role,phase,tool_name,content_utf8_bytes,public_event_key) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![event_key,session_id,i.turn_id,i.ordinal,i.occurred_at,i.item_type,i.role,i.phase,i.tool_name,i.content_utf8_bytes,i.event_key])?;
    }
    let mut touched = HashSet::new();
    for turn in &snapshot.turns {
        touched.insert((
            internal_key(namespace, "session", &turn.session_id),
            turn.turn_id.clone(),
        ));
    }
    for call in &snapshot.model_calls {
        touched.insert((
            internal_key(namespace, "session", &call.session_id),
            call.turn_id.clone(),
        ));
    }
    for (session_id, turn_id) in touched {
        recompute_turn_usage(tx, &session_id, &turn_id)?;
    }
    Ok(())
}

fn recompute_turn_usage(tx: &Transaction<'_>, session_id: &str, turn_id: &str) -> Result<()> {
    let confidence: String = tx.query_row(
        "SELECT usage_confidence FROM turns WHERE session_id=?1 AND turn_id=?2",
        params![session_id, turn_id],
        |row| row.get(0),
    )?;
    if confidence == "unavailable" {
        tx.execute(
            "UPDATE turns SET input_tokens=0,cached_input_tokens=0,cache_write_input_tokens=0,output_tokens=0,reasoning_output_tokens=0,total_tokens=0,model_call_count=0 WHERE session_id=?1 AND turn_id=?2",
            params![session_id, turn_id],
        )?;
        return Ok(());
    }
    let mut statement = tx.prepare("SELECT input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens FROM model_calls WHERE session_id=?1 AND turn_id=?2")?;
    let mut usage = TokenUsage::default();
    let mut calls = 0_i64;
    for row in statement.query_map(params![session_id, turn_id], |row| {
        Ok(TokenUsage {
            input_tokens: row.get(0)?,
            cached_input_tokens: row.get(1)?,
            cache_write_input_tokens: row.get(2)?,
            output_tokens: row.get(3)?,
            reasoning_output_tokens: row.get(4)?,
            total_tokens: row.get(5)?,
        })
    })? {
        let call = row?;
        ensure_usage(&call, "stored model call")?;
        let Some(next) = usage.checked_add(&call) else {
            anyhow::bail!("turn token 聚合超出安全范围")
        };
        usage = next;
        calls = calls.checked_add(1).context("模型调用计数超出安全范围")?;
    }
    tx.execute("UPDATE turns SET input_tokens=?3,cached_input_tokens=?4,cache_write_input_tokens=?5,output_tokens=?6,reasoning_output_tokens=?7,total_tokens=?8,model_call_count=?9 WHERE session_id=?1 AND turn_id=?2",params![session_id,turn_id,usage.input_tokens,usage.cached_input_tokens,usage.cache_write_input_tokens,usage.output_tokens,usage.reasoning_output_tokens,usage.total_tokens,calls])?;
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .context("收紧应用数据目录权限失败")
}

impl Store {
    pub fn page_turns(
        &self,
        limit: i64,
        cursor: Option<&str>,
        project: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
        result: Option<&str>,
    ) -> Result<Page<StoredTurn>> {
        let limit = limit.clamp(1, 200);
        let mut sql = String::from(
            "SELECT coalesce(s.public_session_id,t.session_id),t.session_id,t.turn_id,t.model,t.effort,t.provider,t.cwd,t.started_at,t.completed_at,t.duration_ms,t.first_visible_output_ms,t.result,t.input_tokens,t.cached_input_tokens,t.cache_write_input_tokens,t.output_tokens,t.reasoning_output_tokens,t.total_tokens,t.model_call_count,t.usage_confidence FROM (SELECT * FROM (SELECT t.*,ROW_NUMBER() OVER(PARTITION BY coalesce(t.logical_fingerprint,'legacy:' || t.session_id || ':' || t.turn_id) ORDER BY t.session_id,t.turn_id) AS logical_rank FROM turns t) WHERE logical_rank=1) t JOIN sessions s ON s.session_id=t.session_id WHERE 1=1",
        );
        let mut values: Vec<String> = Vec::new();
        for (clause, value) in [
            (" AND t.cwd = ?", project),
            (" AND t.model = ?", model),
            (" AND t.effort = ?", effort),
        ] {
            if let Some(v) = value {
                sql.push_str(clause);
                values.push(v.into())
            }
        }
        match result {
            Some("failure") => sql.push_str(" AND t.result IN ('failed','aborted')"),
            Some(value) => {
                sql.push_str(" AND t.result = ?");
                values.push(value.into());
            }
            None => {}
        }
        if let Some(c) = cursor {
            sql.push_str(" AND (COALESCE(t.completed_at,t.started_at,'') || '|' || t.session_id || '|' || t.turn_id) < ?");
            values.push(c.into())
        }
        sql.push_str(" ORDER BY COALESCE(t.completed_at,t.started_at,'') DESC, t.session_id DESC, t.turn_id DESC LIMIT ?");
        let mut params_vec: Vec<&dyn rusqlite::ToSql> =
            values.iter().map(|x| x as &dyn rusqlite::ToSql).collect();
        params_vec.push(&limit);
        let mut stmt = self.conn.prepare(&sql)?;
        let items = stmt
            .query_map(params_vec.as_slice(), row_turn)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let total = self
            .conn
            .query_row("SELECT count(*) FROM (SELECT ROW_NUMBER() OVER(PARTITION BY coalesce(logical_fingerprint,'legacy:' || session_id || ':' || turn_id) ORDER BY session_id,turn_id) AS logical_rank FROM turns) WHERE logical_rank=1", [], |r| r.get(0))?;
        let more = items.len() == limit as usize;
        let next_cursor = items.last().map(|x| {
            format!(
                "{}|{}|{}",
                x.completed_at
                    .as_ref()
                    .or(x.started_at.as_ref())
                    .map(String::as_str)
                    .unwrap_or(""),
                x.cursor_session_id,
                x.turn_id
            )
        });
        Ok(Page {
            items,
            next_cursor: if more { next_cursor } else { None },
            total,
        })
    }
    pub fn page_activity(&self, filter: &ActivityFilter) -> Result<Page<ActivityRow>> {
        self.query_activity_on(&self.conn, filter, true)
    }
    pub fn refresh_activity(&self, filter: &ActivityFilter) -> Result<Page<ActivityRow>> {
        self.query_activity_on(&self.conn, filter, false)
    }
    pub fn activity_snapshot(
        &self,
        filter: &ActivityFilter,
        include_total: bool,
    ) -> Result<ActivitySnapshot> {
        self.read_snapshot(|conn| {
            Ok(ActivitySnapshot {
                page: self.query_activity_on(conn, filter, include_total)?,
                revision: self.activity_revision_on(conn)?,
            })
        })
    }
    fn query_activity_on(
        &self,
        conn: &Connection,
        filter: &ActivityFilter,
        include_total: bool,
    ) -> Result<Page<ActivityRow>> {
        let limit = filter.limit.unwrap_or(50).clamp(1, 200);
        let (mut sql, values) = activity_sql(filter);
        let total = if include_total {
            let mut count_filter = filter.clone();
            count_filter.cursor = None;
            let (count_select, count_values) = activity_sql(&count_filter);
            let count_sql = format!("SELECT count(*) FROM ({count_select})");
            let count_params: Vec<&dyn rusqlite::ToSql> = count_values
                .iter()
                .map(|v| v as &dyn rusqlite::ToSql)
                .collect();
            conn.query_row(&count_sql, count_params.as_slice(), |row| row.get(0))?
        } else {
            0
        };
        sql.push_str(" ORDER BY COALESCE(occurred_at,'') DESC, id DESC LIMIT ?");
        let fetch_limit = limit.saturating_add(1);
        let mut params: Vec<&dyn rusqlite::ToSql> =
            values.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        params.push(&fetch_limit);
        let mut stmt = conn.prepare(&sql)?;
        let mut items = stmt
            .query_map(params.as_slice(), row_activity)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let more = items.len() > limit as usize;
        items.truncate(limit as usize);
        let next_cursor = items.last().map(activity_cursor);
        Ok(Page {
            items,
            next_cursor: if more { next_cursor } else { None },
            total,
        })
    }
    pub fn activity_revision(&self) -> Result<String> {
        self.activity_revision_on(&self.conn)
    }
    fn activity_revision_on(&self, conn: &Connection) -> Result<String> {
        let rollout = conn.query_row(
            "SELECT coalesce(max(updated_at),'') || ':' || count(*) || ':' || coalesce(sum(byte_offset),0) FROM ingest_files",
            [],
            |row| row.get::<_, String>(0),
        )?;
        let otel = conn.query_row(
            "SELECT coalesce(max(rowid),0) FROM otel_events",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(format!("rollout:{rollout}|otel:{otel}"))
    }
    pub fn activity_totals(&self) -> Result<(i64, i64, i64)> {
        self.activity_totals_on(&self.conn)
    }
    fn activity_totals_on(&self, conn: &Connection) -> Result<(i64, i64, i64)> {
        let (activity, values) = activity_sql(&ActivityFilter::default());
        debug_assert!(values.is_empty());
        let sql = format!(
            "SELECT count(*),coalesce(sum(result='completed'),0),coalesce(sum(result IN ('failed','aborted')),0) FROM ({activity})"
        );
        conn.query_row(&sql, [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(Into::into)
    }
    pub fn cost_inputs(&self) -> Result<Vec<CostInput>> {
        self.cost_inputs_on(&self.conn)
    }
    fn cost_inputs_on(&self, conn: &Connection) -> Result<Vec<CostInput>> {
        // Keep one row per model call: long-context pricing is determined per
        // request and cannot be reconstructed from a provider/model aggregate.
        let mut statement=conn.prepare("SELECT provider,model,input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens FROM (SELECT c.*,ROW_NUMBER() OVER(PARTITION BY coalesce(c.logical_fingerprint,'legacy:' || c.event_key) ORDER BY (c.model IS NOT NULL)+(c.effort IS NOT NULL)+(c.provider IS NOT NULL)+(c.occurred_at IS NOT NULL) DESC,c.event_key) AS canonical_rank FROM model_calls c) WHERE canonical_rank=1 ORDER BY occurred_at,event_key")?;
        Ok(statement
            .query_map([], |row| {
                let input: i64 = row.get(2)?;
                let cached: i64 = row.get(3)?;
                let write: i64 = row.get(4)?;
                Ok(CostInput {
                    provider: row.get(0)?,
                    model: row.get(1)?,
                    input_tokens: input,
                    cached_input_tokens: cached,
                    cache_write_input_tokens: write,
                    output_tokens: row.get(5)?,
                    reasoning_output_tokens: row.get(6)?,
                    total_input_tokens: input,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn dashboard_snapshot(&self) -> Result<DashboardSnapshot> {
        self.read_snapshot(|conn| {
            let (records, successful, failed) = self.activity_totals_on(conn)?;
            let cost_inputs = self.cost_inputs_on(conn)?;
            Ok(DashboardSnapshot {
                records,
                successful,
                failed,
                model_calls: cost_inputs.len() as i64,
                cost_inputs,
                revision: self.activity_revision_on(conn)?,
            })
        })
    }
    fn read_snapshot<T>(&self, read: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let transaction = self.conn.unchecked_transaction()?;
        let value = read(&transaction)?;
        transaction.commit()?;
        Ok(value)
    }
    pub fn source_health(&self) -> Result<Vec<SourceHealthRow>> {
        let mut statement=self.conn.prepare("SELECT canonical_path,source_kind,byte_offset,file_identity,updated_at,last_error,unparsed_events FROM ingest_files ORDER BY updated_at DESC")?;
        Ok(statement
            .query_map([], |row| {
                Ok(SourceHealthRow {
                    canonical_path: row.get(0)?,
                    source_kind: row.get(1)?,
                    byte_offset: row.get(2)?,
                    file_identity: row.get(3)?,
                    updated_at: row.get(4)?,
                    last_error: row.get(5)?,
                    unparsed_events: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn prune_retention(&self, older_than: &str) -> Result<usize> {
        let turns = self.conn.execute(
            "DELETE FROM turns WHERE COALESCE(completed_at,started_at,'') < ?1",
            params![older_than],
        )?;
        self.conn.execute(
            "DELETE FROM sessions WHERE NOT EXISTS(SELECT 1 FROM turns WHERE turns.session_id=sessions.session_id)",
            [],
        )?;
        let otel = self.conn.execute(
            "DELETE FROM otel_events WHERE received_at < ?1",
            params![older_than],
        )?;
        Ok(turns + otel)
    }
    pub fn trim_revisions(&self, path: &str, keep: i64) -> Result<usize> {
        let keep = keep.clamp(1, 1000);
        Ok(self.conn.execute("DELETE FROM agents_revisions WHERE path=?1 AND id IN (SELECT id FROM agents_revisions WHERE path=?1 ORDER BY created_at DESC LIMIT -1 OFFSET ?2)",params![path,keep])?)
    }
    pub fn summary(&self) -> Result<(i64, i64, i64, TokenUsage)> {
        let mut statement = self.conn.prepare("WITH canonical_turns AS (SELECT * FROM (SELECT t.*,coalesce(s.public_session_id,t.session_id) AS public_session_id,ROW_NUMBER() OVER(PARTITION BY coalesce(t.logical_fingerprint,'legacy:' || t.session_id || ':' || t.turn_id) ORDER BY CASE WHEN t.result IN ('completed','failed','aborted') THEN 1 ELSE 0 END DESC,t.completed_at IS NOT NULL DESC,t.duration_ms IS NOT NULL DESC,t.session_id,t.turn_id) AS canonical_rank FROM turns t JOIN sessions s ON s.session_id=t.session_id) WHERE canonical_rank=1),canonical_calls AS (SELECT * FROM (SELECT c.*,coalesce(s.public_session_id,c.session_id) AS public_session_id,ROW_NUMBER() OVER(PARTITION BY coalesce(c.logical_fingerprint,'legacy:' || c.event_key) ORDER BY c.event_key) AS canonical_rank FROM model_calls c JOIN sessions s ON s.session_id=c.session_id) WHERE canonical_rank=1),usage_by_turn AS (SELECT public_session_id,turn_id,sum(input_tokens) AS input_tokens,sum(cached_input_tokens) AS cached_input_tokens,sum(cache_write_input_tokens) AS cache_write_input_tokens,sum(output_tokens) AS output_tokens,sum(reasoning_output_tokens) AS reasoning_output_tokens,sum(total_tokens) AS total_tokens FROM canonical_calls GROUP BY public_session_id,turn_id) SELECT t.result,coalesce(u.input_tokens,0),coalesce(u.cached_input_tokens,0),coalesce(u.cache_write_input_tokens,0),coalesce(u.output_tokens,0),coalesce(u.reasoning_output_tokens,0),coalesce(u.total_tokens,0),t.usage_confidence FROM canonical_turns t LEFT JOIN usage_by_turn u ON u.public_session_id=t.public_session_id AND u.turn_id=t.turn_id")?;
        let mut total = 0_i64;
        let mut completed = 0_i64;
        let mut failed = 0_i64;
        let mut usage = TokenUsage::default();
        for row in statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                TokenUsage {
                    input_tokens: row.get(1)?,
                    cached_input_tokens: row.get(2)?,
                    cache_write_input_tokens: row.get(3)?,
                    output_tokens: row.get(4)?,
                    reasoning_output_tokens: row.get(5)?,
                    total_tokens: row.get(6)?,
                },
                row.get::<_, String>(7)?,
            ))
        })? {
            let (result, turn_usage, confidence) = row?;
            total = total.checked_add(1).context("turn 数量超出安全范围")?;
            if result == "completed" {
                completed = completed
                    .checked_add(1)
                    .context("完成 turn 数量超出安全范围")?;
            }
            if matches!(result.as_str(), "failed" | "aborted") {
                failed = failed
                    .checked_add(1)
                    .context("失败 turn 数量超出安全范围")?;
            }
            if confidence != "unavailable" {
                ensure_usage(&turn_usage, "stored turn")?;
                usage = usage
                    .checked_add(&turn_usage)
                    .context("总 token 聚合超出安全范围")?;
            }
        }
        Ok((total, completed, failed, usage))
    }
    pub fn upsert_project(&self, row: &ProjectRow) -> Result<()> {
        self.conn.execute("INSERT INTO projects(canonical_path,name,source,exists_flag,is_git,worktree,last_seen_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(canonical_path) DO UPDATE SET name=excluded.name,source=excluded.source,exists_flag=excluded.exists_flag,is_git=excluded.is_git,worktree=excluded.worktree,last_seen_at=excluded.last_seen_at,updated_at=excluded.updated_at",params![row.canonical_path,row.name,row.source,row.exists,row.is_git,row.worktree,row.last_seen_at,Utc::now().to_rfc3339()])?;
        Ok(())
    }
    pub fn projects(&self) -> Result<Vec<ProjectRow>> {
        let mut s=self.conn.prepare("WITH canonical_turns AS (SELECT * FROM (SELECT t.*,coalesce(s.public_session_id,t.session_id) AS public_session_id,ROW_NUMBER() OVER(PARTITION BY coalesce(t.logical_fingerprint,'legacy:' || t.session_id || ':' || t.turn_id) ORDER BY CASE WHEN t.result IN ('completed','failed','aborted') THEN 1 ELSE 0 END DESC,t.completed_at IS NOT NULL DESC,t.duration_ms IS NOT NULL DESC,t.session_id,t.turn_id) AS canonical_rank FROM turns t JOIN sessions s ON s.session_id=t.session_id) WHERE canonical_rank=1) SELECT p.canonical_path,p.name,p.source,p.exists_flag,p.is_git,p.worktree,p.last_seen_at,(SELECT max(coalesce(t.completed_at,t.started_at)) FROM canonical_turns t WHERE t.cwd=p.canonical_path OR (substr(t.cwd,1,length(p.canonical_path))=p.canonical_path AND substr(t.cwd,length(p.canonical_path)+1,1) IN ('/',char(92)))) AS last_conversation_at FROM projects p ORDER BY last_conversation_at IS NULL,last_conversation_at DESC,p.name")?;
        Ok(s.query_map([], |r| {
            Ok(ProjectRow {
                canonical_path: r.get(0)?,
                name: r.get(1)?,
                source: r.get(2)?,
                exists: r.get(3)?,
                is_git: r.get(4)?,
                worktree: r.get(5)?,
                last_seen_at: r.get(6)?,
                last_conversation_at: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }
    pub fn observed_cwds(&self) -> Result<Vec<String>> {
        let mut s=self.conn.prepare("SELECT cwd FROM turns WHERE cwd IS NOT NULL AND cwd <> '' GROUP BY cwd ORDER BY max(COALESCE(completed_at,started_at)) DESC, cwd")?;
        Ok(s.query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn settings(&self) -> Result<SettingsRow> {
        Ok(self.conn.query_row("SELECT codex_homes_json,authorized_roots_json,retention_days,telemetry_enabled,price_catalog_version FROM settings WHERE singleton=1",[],|r|Ok(SettingsRow{codex_homes:serde_json::from_str(&r.get::<_,String>(0)?).unwrap_or_default(),authorized_roots:serde_json::from_str(&r.get::<_,String>(1)?).unwrap_or_default(),retention_days:r.get(2)?,telemetry_enabled:r.get(3)?,price_catalog_version:r.get(4)?}))?)
    }
    pub fn save_settings(&self, s: &SettingsRow) -> Result<()> {
        self.conn.execute("UPDATE settings SET codex_homes_json=?1,authorized_roots_json=?2,retention_days=?3,telemetry_enabled=?4,price_catalog_version=?5 WHERE singleton=1",params![serde_json::to_string(&s.codex_homes)?,serde_json::to_string(&s.authorized_roots)?,s.retention_days,s.telemetry_enabled,s.price_catalog_version])?;
        Ok(())
    }
    pub fn add_revision(&self, r: &RevisionRow) -> Result<()> {
        self.conn.execute("INSERT INTO agents_revisions(id,path,created_at,before_sha256,after_sha256,byte_length,before_content,after_content) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![r.id,r.path,r.created_at,r.before_sha256,r.after_sha256,r.byte_length,r.before_content,r.after_content])?;
        Ok(())
    }
    pub fn prepare_revision(&self, draft: RevisionDraft<'_>) -> Result<()> {
        self.conn.execute("INSERT INTO agents_revisions(id,path,created_at,before_sha256,after_sha256,byte_length,before_content,after_content,status) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending')",params![draft.id,draft.path,draft.created_at,draft.before_sha256,draft.after_sha256,draft.byte_length,draft.before_content,draft.after_content])?;
        Ok(())
    }
    pub fn commit_revision(&self, id: &str) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE agents_revisions SET status='applied' WHERE id=?1 AND status='pending'",
            params![id],
        )? == 1)
    }
    pub fn discard_revision(&self, id: &str) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM agents_revisions WHERE id=?1 AND status='pending'",
            params![id],
        )? == 1)
    }
    pub fn cleanup_pending_revisions(&self, older_than: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM agents_revisions WHERE status='pending' AND created_at < ?1",
            params![older_than],
        )?)
    }
    pub fn insert_otel_metadata(&self, event: OtelMetadata<'_>) -> Result<()> {
        self.insert_otel_batch(&[event]).map(|_| ())
    }
    /// Atomically persist an already-projected OTel batch. The batch is
    /// idempotent by event id, has a bounded cardinality, and uses one receipt
    /// timestamp so a client-controlled `occurred_at` can never evade expiry.
    pub fn insert_otel_batch(&self, events: &[OtelMetadata<'_>]) -> Result<OtelBatchResult> {
        if events.len() > MAX_OTEL_BATCH_EVENTS {
            anyhow::bail!("OTel batch 超过 {MAX_OTEL_BATCH_EVENTS} 条上限")
        }
        if events.is_empty() {
            return Ok(OtelBatchResult::default());
        }
        for event in events {
            validate_otel_event(event)?;
        }
        let distinct_ids = events.iter().map(|event| event.id).collect::<HashSet<_>>();
        let tx = self.conn.unchecked_transaction()?;
        let mut new_ids = 0_i64;
        for id in &distinct_ids {
            let exists = tx
                .query_row("SELECT 1 FROM otel_events WHERE id=?1", params![id], |_| {
                    Ok(())
                })
                .optional()?
                .is_some();
            if !exists {
                new_ids = new_ids.checked_add(1).context("OTel 批量计数溢出")?;
            }
        }
        let existing: i64 =
            tx.query_row("SELECT count(*) FROM otel_events", [], |row| row.get(0))?;
        if existing
            .checked_add(new_ids)
            .filter(|count| *count <= self.otel_event_limit)
            .is_none()
        {
            anyhow::bail!("OTel 本地事件配额已满；拒绝整个批次以保留已有数据")
        }
        let received_at = Utc::now().to_rfc3339();
        let mut inserted = 0_usize;
        for event in events {
            let changed = tx.execute("INSERT OR IGNORE INTO otel_events(id,received_at,signal,event_name,occurred_at,thread_id,turn_id,model,provider,status_code,duration_ms,response_bytes,input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens,attributes_json,endpoint,success) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",params![event.id,received_at,event.signal,event.event_name,event.occurred_at,event.thread_id,event.turn_id,event.model,event.provider,event.status_code,event.duration_ms,event.response_bytes,event.input_tokens,event.cached_input_tokens,event.cache_write_input_tokens,event.output_tokens,event.reasoning_output_tokens,event.total_tokens,event.attributes_json,event.endpoint,event.success])?;
            inserted = inserted.checked_add(changed).context("OTel 插入计数溢出")?;
        }
        tx.commit()?;
        Ok(OtelBatchResult {
            inserted,
            duplicates: events.len().saturating_sub(inserted),
        })
    }
    pub fn otel_last_received(&self) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT max(received_at) FROM otel_events", [], |row| {
                row.get(0)
            })?)
    }
    pub fn revisions(&self, path: &str) -> Result<Vec<RevisionRow>> {
        let mut s=self.conn.prepare("SELECT id,path,created_at,before_sha256,after_sha256,byte_length,before_content,after_content FROM agents_revisions WHERE path=?1 AND status='applied' ORDER BY created_at DESC")?;
        Ok(s.query_map(params![path], row_revision)?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn revision(&self, id: &str) -> Result<Option<RevisionRow>> {
        Ok(self.conn.query_row("SELECT id,path,created_at,before_sha256,after_sha256,byte_length,before_content,after_content FROM agents_revisions WHERE id=?1 AND status='applied'",params![id],row_revision).optional()?)
    }
    pub fn pending_revisions(&self) -> Result<Vec<RevisionRow>> {
        let mut statement = self.conn.prepare("SELECT id,path,created_at,before_sha256,after_sha256,byte_length,before_content,after_content FROM agents_revisions WHERE status='pending' ORDER BY created_at")?;
        Ok(statement
            .query_map([], row_revision)?
            .collect::<rusqlite::Result<_>>()?)
    }
}
fn upsert_turn(tx: &Transaction<'_>, namespace: &str, t: &codex_core::Turn) -> Result<()> {
    let session_id = internal_key(namespace, "session", &t.session_id);
    let fingerprint = turn_fingerprint(t)?;
    tx.execute("INSERT INTO turns(session_id,turn_id,model,effort,provider,cwd,started_at,completed_at,duration_ms,first_visible_output_ms,result,input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens,model_call_count,usage_confidence,logical_fingerprint) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20) ON CONFLICT(session_id,turn_id) DO UPDATE SET model=coalesce(excluded.model,turns.model),effort=coalesce(excluded.effort,turns.effort),provider=coalesce(excluded.provider,turns.provider),cwd=coalesce(excluded.cwd,turns.cwd),started_at=coalesce(excluded.started_at,turns.started_at),completed_at=coalesce(excluded.completed_at,turns.completed_at),duration_ms=coalesce(excluded.duration_ms,turns.duration_ms),first_visible_output_ms=coalesce(excluded.first_visible_output_ms,turns.first_visible_output_ms),result=excluded.result,input_tokens=excluded.input_tokens,cached_input_tokens=excluded.cached_input_tokens,cache_write_input_tokens=excluded.cache_write_input_tokens,output_tokens=excluded.output_tokens,reasoning_output_tokens=excluded.reasoning_output_tokens,total_tokens=excluded.total_tokens,model_call_count=excluded.model_call_count,usage_confidence=excluded.usage_confidence,logical_fingerprint=excluded.logical_fingerprint",params![session_id,t.turn_id,t.model,t.effort,t.provider,t.cwd,t.started_at,t.completed_at,t.duration_ms,t.first_visible_output_ms,t.status,t.usage.input_tokens,t.usage.cached_input_tokens,t.usage.cache_write_input_tokens,t.usage.output_tokens,t.usage.reasoning_output_tokens,t.usage.total_tokens,t.model_call_count,t.usage_confidence,fingerprint])?;
    Ok(())
}
fn row_turn(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredTurn> {
    Ok(StoredTurn {
        session_id: r.get(0)?,
        cursor_session_id: r.get(1)?,
        turn_id: r.get(2)?,
        model: r.get(3)?,
        effort: r.get(4)?,
        provider: r.get(5)?,
        cwd: r.get(6)?,
        started_at: r.get(7)?,
        completed_at: r.get(8)?,
        duration_ms: r.get(9)?,
        first_visible_output_ms: r.get(10)?,
        result: r.get(11)?,
        usage: TokenUsage {
            input_tokens: r.get(12)?,
            cached_input_tokens: r.get(13)?,
            cache_write_input_tokens: r.get(14)?,
            output_tokens: r.get(15)?,
            reasoning_output_tokens: r.get(16)?,
            total_tokens: r.get(17)?,
        },
        model_call_count: r.get(18)?,
        usage_confidence: r.get(19)?,
    })
}
fn row_revision(r: &rusqlite::Row<'_>) -> rusqlite::Result<RevisionRow> {
    Ok(RevisionRow {
        id: r.get(0)?,
        path: r.get(1)?,
        created_at: r.get(2)?,
        before_sha256: r.get(3)?,
        after_sha256: r.get(4)?,
        byte_length: r.get(5)?,
        before_content: r.get(6)?,
        after_content: r.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_core::{ModelCall, Session, Turn};
    use tempfile::tempdir;
    fn usage(total: i64) -> TokenUsage {
        TokenUsage {
            input_tokens: total,
            cached_input_tokens: 0,
            cache_write_input_tokens: 0,
            output_tokens: 0,
            reasoning_output_tokens: 0,
            total_tokens: total,
        }
    }
    fn snapshot(
        session_id: &str,
        turn_id: &str,
        cwd: &str,
        result: &str,
        call: Option<(&str, i64, i64)>,
    ) -> Snapshot {
        let call_usage = call.map(|(_, delta, _)| usage(delta)).unwrap_or_default();
        let turn_usage = call.map(|(_, _, total)| usage(total)).unwrap_or_default();
        Snapshot {
            session: Some(Session {
                session_id: session_id.into(),
                thread_id: session_id.into(),
                cli_version: None,
                cwd: Some(cwd.into()),
                model_provider: Some("openai".into()),
                started_at: Some("2026-08-27T00:00:00Z".into()),
            }),
            turns: vec![Turn {
                turn_id: turn_id.into(),
                session_id: session_id.into(),
                model: Some("gpt-test".into()),
                effort: Some("high".into()),
                provider: Some("openai".into()),
                cwd: Some(cwd.into()),
                started_at: Some("2026-08-27T00:00:00Z".into()),
                completed_at: Some("2026-08-27T00:00:01Z".into()),
                duration_ms: Some(10),
                first_visible_output_ms: Some(2),
                status: result.into(),
                usage: turn_usage,
                model_call_count: if call.is_some() { 1 } else { 0 },
                usage_confidence: "derived".into(),
            }],
            model_calls: call
                .map(|(key, _delta, total)| {
                    vec![ModelCall {
                        event_key: key.into(),
                        session_id: session_id.into(),
                        turn_id: turn_id.into(),
                        ordinal: None,
                        occurred_at: Some("2026-08-27T00:00:01Z".into()),
                        model: Some("gpt-test".into()),
                        effort: Some("high".into()),
                        provider: Some("openai".into()),
                        usage: call_usage,
                        cumulative_usage: usage(total),
                        usage_confidence: "derived".into(),
                    }]
                })
                .unwrap_or_default(),
            timeline: vec![],
            diagnostics: vec![],
            unhandled_event_counts: Default::default(),
            ignored_duplicate_events: 0,
            ignored_duplicate_usage_snapshots: 0,
        }
    }

    fn otel_event<'a>(id: &'a str, occurred_at: Option<&'a str>) -> OtelMetadata<'a> {
        OtelMetadata {
            id,
            signal: "logs",
            event_name: Some("codex.api_request"),
            occurred_at,
            thread_id: None,
            turn_id: None,
            model: Some("gpt-test"),
            provider: Some("openai"),
            status_code: Some(200),
            duration_ms: Some(1),
            response_bytes: Some(1),
            endpoint: Some("openai"),
            success: Some(true),
            input_tokens: Some(1),
            cached_input_tokens: Some(0),
            cache_write_input_tokens: Some(0),
            output_tokens: Some(0),
            reasoning_output_tokens: Some(0),
            total_tokens: Some(1),
            attributes_json: "{}",
        }
    }
    #[test]
    fn migrations_and_settings_work() {
        let d = tempdir().unwrap();
        let s = Store::open(&d.path().join("x.db")).unwrap();
        assert_eq!(s.settings().unwrap().retention_days, 30);
        s.set_checkpoint("a", "rollout", 9, None, None).unwrap();
        assert_eq!(s.checkpoint("a").unwrap(), 9);
    }

    #[test]
    fn v9_rewrites_v8_fingerprints_once_and_collapses_copied_rollouts() {
        let d = tempdir().unwrap();
        let database = d.path().join("x.db");
        let mut store = Store::open(&database).unwrap();
        let copied = snapshot(
            "copied-session",
            "copied-turn",
            "/project",
            "completed",
            Some(("copied-call", 7, 7)),
        );
        for (path, identity) in [("original", "inode-a"), ("copy", "inode-b")] {
            store
                .commit_ingest(CommitIngest {
                    path,
                    source_kind: "rollout",
                    next_offset: 10,
                    file_identity: Some(identity),
                    snapshot: &copied,
                    resume_state: &ResumeState::default(),
                    rebuild: false,
                    unparsed_events: 0,
                })
                .unwrap();
        }

        // Simulate an already-migrated v8 database whose mutable enrichment
        // produced a different fingerprint for each filesystem copy.
        store
            .conn
            .execute("UPDATE turns SET logical_fingerprint=session_id", [])
            .unwrap();
        store
            .conn
            .execute("UPDATE model_calls SET logical_fingerprint=event_key", [])
            .unwrap();
        store
            .conn
            .execute("DELETE FROM schema_migrations WHERE version=9", [])
            .unwrap();
        drop(store);

        let migrated = Store::open(&database).unwrap();
        assert_eq!(migrated.activity_totals().unwrap(), (1, 1, 0));
        assert_eq!(
            migrated
                .page_activity(&ActivityFilter {
                    view: Some("modelCalls".into()),
                    ..Default::default()
                })
                .unwrap()
                .total,
            1
        );
        let fingerprints_before = migrated
            .conn
            .query_row(
                "SELECT min(logical_fingerprint),max(logical_fingerprint) FROM model_calls",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(fingerprints_before.0, fingerprints_before.1);
        drop(migrated);

        // A normal subsequent open must preserve the completed v2 backfill.
        let reopened = Store::open(&database).unwrap();
        let fingerprints_after = reopened
            .conn
            .query_row(
                "SELECT min(logical_fingerprint),max(logical_fingerprint) FROM model_calls",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(fingerprints_after, fingerprints_before);
    }

    #[test]
    fn canonical_turn_status_marks_only_stale_or_archived_running_turns_unobserved() {
        let d = tempdir().unwrap();
        let mut store = Store::open(&d.path().join("x.db")).unwrap();
        let state = ResumeState::default();
        let mut old = snapshot(
            "session",
            "old",
            "/project",
            "running",
            Some(("old-call", 2, 2)),
        );
        old.turns[0].completed_at = None;
        old.turns[0].duration_ms = None;
        old.turns[0].first_visible_output_ms = None;
        store
            .commit_ingest(CommitIngest {
                path: "live",
                source_kind: "rollout",
                next_offset: 10,
                file_identity: Some("live"),
                snapshot: &old,
                resume_state: &state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        let mut current = snapshot(
            "session",
            "current",
            "/project",
            "running",
            Some(("current-call", 3, 3)),
        );
        current.turns[0].started_at = Some("2026-08-27T00:01:00Z".into());
        current.turns[0].completed_at = None;
        current.turns[0].duration_ms = None;
        current.turns[0].first_visible_output_ms = None;
        store
            .commit_ingest(CommitIngest {
                path: "live",
                source_kind: "rollout",
                next_offset: 20,
                file_identity: Some("live"),
                snapshot: &current,
                resume_state: &state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        let mut current_copy = current.clone();
        current_copy.turns[0].started_at = Some("2026-08-27T00:02:00Z".into());
        store
            .commit_ingest(CommitIngest {
                path: "live-copy",
                source_kind: "rollout",
                next_offset: 10,
                file_identity: Some("live-copy"),
                snapshot: &current_copy,
                resume_state: &state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        let rows = store.page_activity(&ActivityFilter::default()).unwrap();
        assert!(
            rows.items
                .iter()
                .any(|row| row.turn_id.as_deref() == Some("old") && row.result == "unobserved")
        );
        assert!(
            rows.items
                .iter()
                .any(|row| row.turn_id.as_deref() == Some("current") && row.result == "running")
        );

        let mut archived = snapshot("archive", "tail", "/archive", "running", None);
        archived.turns[0].completed_at = None;
        archived.turns[0].duration_ms = None;
        store
            .commit_ingest(CommitIngest {
                path: "/Users/test/.codex/archived_sessions/rollout-tail.jsonl",
                source_kind: "rollout",
                next_offset: 10,
                file_identity: Some("archive"),
                snapshot: &archived,
                resume_state: &state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        let rows = store.page_activity(&ActivityFilter::default()).unwrap();
        assert!(
            rows.items
                .iter()
                .any(|row| row.turn_id.as_deref() == Some("tail") && row.result == "unobserved")
        );
    }
    #[test]
    fn project_conversation_time_uses_component_safe_path_prefix() {
        let d = tempdir().unwrap();
        let mut store = Store::open(&d.path().join("x.db")).unwrap();
        for path in ["/work/app", "/work/app2", "/work/a_%"] {
            store
                .upsert_project(&ProjectRow {
                    canonical_path: path.into(),
                    name: path.into(),
                    source: "manual".into(),
                    exists: true,
                    is_git: false,
                    worktree: false,
                    last_seen_at: None,
                    last_conversation_at: None,
                })
                .unwrap();
        }
        let child = snapshot(
            "project-session",
            "turn",
            "/work/a_%/child",
            "completed",
            None,
        );
        store
            .commit_ingest(CommitIngest {
                path: "rollout",
                source_kind: "rollout",
                next_offset: 1,
                file_identity: Some("project"),
                snapshot: &child,
                resume_state: &ResumeState::default(),
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        let rows = store.projects().unwrap();
        assert!(
            rows.iter()
                .find(|row| row.canonical_path == "/work/a_%")
                .unwrap()
                .last_conversation_at
                .is_some()
        );
        assert!(
            rows.iter()
                .find(|row| row.canonical_path == "/work/app")
                .unwrap()
                .last_conversation_at
                .is_none()
        );
    }
    #[test]
    fn commit_ingest_is_resumable_idempotent_and_rebuilds_after_truncation() {
        let d = tempdir().unwrap();
        let mut store = Store::open(&d.path().join("x.db")).unwrap();
        let first = snapshot("s", "t", "/one", "completed", Some(("a", 10, 10)));
        let first_state = ResumeState {
            session: first.session.clone(),
            current_turn: first.turns.first().cloned(),
            last_cumulative_usage: Some(usage(10)),
            unavailable_turn_id: None,
        };
        store
            .commit_ingest(CommitIngest {
                path: "rollout",
                source_kind: "rollout",
                next_offset: 100,
                file_identity: Some("inode-a"),
                snapshot: &first,
                resume_state: &first_state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        let second = snapshot("s", "t", "/one", "completed", Some(("b", 6, 16)));
        let second_state = ResumeState {
            session: second.session.clone(),
            current_turn: second.turns.first().cloned(),
            last_cumulative_usage: Some(usage(16)),
            unavailable_turn_id: None,
        };
        store
            .commit_ingest(CommitIngest {
                path: "rollout",
                source_kind: "rollout",
                next_offset: 120,
                file_identity: Some("inode-a"),
                snapshot: &second,
                resume_state: &second_state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        store
            .commit_ingest(CommitIngest {
                path: "rollout",
                source_kind: "rollout",
                next_offset: 120,
                file_identity: Some("inode-a"),
                snapshot: &second,
                resume_state: &second_state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        assert_eq!(store.summary().unwrap().3.total_tokens, 16);
        assert_eq!(
            store
                .ingest_checkpoint("rollout")
                .unwrap()
                .resume_state
                .unwrap()
                .last_cumulative_usage
                .unwrap()
                .total_tokens,
            16
        );
        assert!(
            store
                .plan_ingest("rollout", Some("inode-a"), 80)
                .unwrap()
                .rebuild_required
        );
        assert!(
            store
                .plan_ingest("rollout", Some("inode-b"), 200)
                .unwrap()
                .rebuild_required
        );
        store
            .commit_ingest(CommitIngest {
                path: "rollout",
                source_kind: "rollout",
                next_offset: 20,
                file_identity: Some("inode-b"),
                snapshot: &first,
                resume_state: &first_state,
                rebuild: true,
                unparsed_events: 0,
            })
            .unwrap();
        assert_eq!(store.summary().unwrap().3.total_tokens, 10);
    }

    #[test]
    fn namespaces_sources_without_changing_display_ids_or_rebuild_scope() {
        let d = tempdir().unwrap();
        let mut store = Store::open(&d.path().join("x.db")).unwrap();
        let state = ResumeState::default();
        let first = snapshot(
            "same-session",
            "same-turn",
            "/one",
            "completed",
            Some(("same-call", 4, 4)),
        );
        let second = snapshot(
            "same-session",
            "same-turn",
            "/two",
            "completed",
            Some(("same-call", 8, 8)),
        );
        store
            .commit_ingest(CommitIngest {
                path: "one",
                source_kind: "rollout",
                next_offset: 10,
                file_identity: Some("inode-one"),
                snapshot: &first,
                resume_state: &state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        store
            .commit_ingest(CommitIngest {
                path: "two",
                source_kind: "rollout",
                next_offset: 10,
                file_identity: Some("inode-two"),
                snapshot: &second,
                resume_state: &state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        assert_eq!(store.summary().unwrap().3.total_tokens, 12);
        let activity = store.page_activity(&ActivityFilter::default()).unwrap();
        let rollout = activity
            .items
            .iter()
            .filter(|item| item.source_kind == "rollout")
            .collect::<Vec<_>>();
        assert_eq!(rollout.len(), 1);
        assert!(
            rollout
                .iter()
                .all(|item| item.session_id.as_deref() == Some("same-session")
                    && item.activity_kind == "turn")
        );
        let calls = store
            .page_activity(&ActivityFilter {
                view: Some("modelCalls".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(calls.total, 2);
        assert!(calls.items.iter().all(|item| {
            item.result == "accounted"
                && item.parent_turn_result.as_deref() == Some("completed")
                && item.duration_ms.is_none()
                && item.first_visible_output_ms.is_none()
        }));

        // A copied rollout gets a different filesystem identity, but its exact
        // logical rows must not double-count activity, turns, or cost.
        store
            .commit_ingest(CommitIngest {
                path: "archive-distinct-inode",
                source_kind: "rollout",
                next_offset: 10,
                file_identity: Some("inode-copy"),
                snapshot: &second,
                resume_state: &state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        assert_eq!(store.summary().unwrap().3.total_tokens, 12);
        assert_eq!(
            store
                .page_activity(&ActivityFilter {
                    view: Some("modelCalls".into()),
                    ..Default::default()
                })
                .unwrap()
                .total,
            2
        );
        assert_eq!(
            store
                .page_turns(50, None, None, None, None, None)
                .unwrap()
                .total,
            1
        );
        assert_eq!(store.cost_inputs().unwrap().len(), 2);
        assert_eq!(store.activity_totals().unwrap(), (1, 1, 0));
        store.rebuild_source("archive-distinct-inode").unwrap();
        assert_eq!(store.summary().unwrap().3.total_tokens, 12);

        store.rebuild_source("one").unwrap();
        assert_eq!(store.summary().unwrap().3.total_tokens, 8);
        assert_eq!(
            store
                .page_activity(&ActivityFilter {
                    view: Some("modelCalls".into()),
                    ..Default::default()
                })
                .unwrap()
                .total,
            1
        );
        assert_eq!(store.activity_totals().unwrap(), (1, 1, 0));

        let duplicate = snapshot(
            "same-session",
            "same-turn",
            "/two",
            "completed",
            Some(("same-call", 8, 8)),
        );
        store
            .commit_ingest(CommitIngest {
                path: "archive-copy",
                source_kind: "rollout",
                next_offset: 10,
                file_identity: Some("inode-two"),
                snapshot: &duplicate,
                resume_state: &state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        assert_eq!(store.summary().unwrap().3.total_tokens, 8);
        store.rebuild_source("archive-copy").unwrap();
        assert_eq!(store.summary().unwrap().3.total_tokens, 8);
    }

    #[test]
    fn otel_batch_is_atomic_bounded_and_retained_by_receipt_time() {
        let d = tempdir().unwrap();
        let mut store = Store::open(&d.path().join("x.db")).unwrap();
        store.otel_event_limit = 1;
        assert_eq!(
            store
                .insert_otel_batch(&[otel_event("one", Some("2999-01-01T00:00:00Z"))])
                .unwrap()
                .inserted,
            1
        );
        assert!(store.insert_otel_batch(&[otel_event("two", None)]).is_err());
        assert_eq!(
            store
                .page_activity(&ActivityFilter {
                    view: Some("modelCalls".into()),
                    ..Default::default()
                })
                .unwrap()
                .total,
            1
        );
        assert_eq!(store.prune_retention("2099-01-01T00:00:00Z").unwrap(), 1);
        assert_eq!(
            store
                .page_activity(&ActivityFilter {
                    view: Some("modelCalls".into()),
                    ..Default::default()
                })
                .unwrap()
                .total,
            0
        );
    }
    #[test]
    fn activity_sql_paginates_filters_and_keeps_failures_without_calls() {
        let d = tempdir().unwrap();
        let mut store = Store::open(&d.path().join("x.db")).unwrap();
        let one = snapshot(
            "one",
            "turn-one",
            "/project-one",
            "completed",
            Some(("call-one", 4, 4)),
        );
        let two = snapshot("two", "turn-two", "/project-two", "failed", None);
        let three = snapshot(
            "three",
            "turn-three",
            "/project-one",
            "completed",
            Some(("call-three", 8, 8)),
        );
        let state = ResumeState::default();
        store
            .commit_ingest(CommitIngest {
                path: "one",
                source_kind: "rollout",
                next_offset: 10,
                file_identity: Some("a"),
                snapshot: &one,
                resume_state: &state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        store
            .commit_ingest(CommitIngest {
                path: "three",
                source_kind: "rollout",
                next_offset: 10,
                file_identity: Some("c"),
                snapshot: &three,
                resume_state: &state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        store
            .commit_ingest(CommitIngest {
                path: "two",
                source_kind: "rollout",
                next_offset: 10,
                file_identity: Some("b"),
                snapshot: &two,
                resume_state: &state,
                rebuild: false,
                unparsed_events: 0,
            })
            .unwrap();
        let page = store
            .page_activity(&ActivityFilter {
                limit: Some(1),
                project_path: Some("/project-one".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.items.len(), 1);
        assert!(page.items[0].has_model_call);
        let following = store
            .page_activity(&ActivityFilter {
                limit: Some(1),
                project_path: Some("/project-one".into()),
                cursor: page.next_cursor.clone(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(following.total, 2);
        assert_eq!(following.items.len(), 1);
        let lightweight = store
            .refresh_activity(&ActivityFilter {
                limit: Some(1),
                project_path: Some("/project-one".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(lightweight.total, 0);
        assert_eq!(lightweight.items[0].id, page.items[0].id);
        assert!(store.activity_revision().unwrap().contains("rollout:"));
        let failures = store
            .page_activity(&ActivityFilter {
                result: Some("failure".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(failures.total, 1);
        assert!(!failures.items[0].has_model_call);
        let search = store
            .page_activity(&ActivityFilter {
                search: Some("project-two".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(search.total, 1);
        store
            .insert_otel_metadata(OtelMetadata {
                id: "otel-row",
                signal: "logs",
                event_name: Some("codex.api_request"),
                occurred_at: Some("2026-08-27T00:00:02Z"),
                thread_id: Some("otel-thread"),
                turn_id: Some("otel-turn"),
                model: Some("gpt-otel"),
                provider: Some("openai"),
                status_code: Some(500),
                duration_ms: Some(12),
                response_bytes: Some(99),
                endpoint: Some("https://api.example.test/v1/responses"),
                // Explicit HTTP status is authoritative if an exporter also
                // emits a contradictory boolean success attribute.
                success: Some(true),
                input_tokens: None,
                cached_input_tokens: None,
                cache_write_input_tokens: None,
                output_tokens: None,
                reasoning_output_tokens: None,
                total_tokens: None,
                attributes_json: "{}",
            })
            .unwrap();
        assert_eq!(store.activity_totals().unwrap(), (3, 2, 1));
        let calls = store
            .page_activity(&ActivityFilter {
                view: Some("modelCalls".into()),
                ..Default::default()
            })
            .unwrap();
        let otel = calls
            .items
            .iter()
            .find(|item| item.source_kind == "otel")
            .unwrap();
        assert_eq!(otel.status_code, Some(500));
        assert_eq!(otel.result, "failed");
        assert_eq!(otel.activity_kind, "otelRequest");
        assert_eq!(otel.timing_scope, "request");
        assert_eq!(otel.duration_ms, Some(12));
        assert_eq!(otel.response_bytes, Some(99));
        assert_eq!(
            otel.endpoint.as_deref(),
            Some("https://api.example.test/v1/responses")
        );
        store
            .insert_otel_metadata(OtelMetadata {
                id: "otel-metric-row",
                signal: "metrics",
                event_name: Some("codex.api_request"),
                occurred_at: Some("2026-08-27T00:00:03Z"),
                thread_id: None,
                turn_id: None,
                model: Some("gpt-aggregate"),
                provider: None,
                status_code: Some(200),
                duration_ms: Some(12),
                response_bytes: None,
                endpoint: None,
                success: Some(true),
                input_tokens: None,
                cached_input_tokens: None,
                cache_write_input_tokens: None,
                output_tokens: None,
                reasoning_output_tokens: None,
                total_tokens: None,
                attributes_json: "{}",
            })
            .unwrap();
        let after_metric = store
            .page_activity(&ActivityFilter {
                view: Some("modelCalls".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            after_metric
                .items
                .iter()
                .filter(|item| item.source_kind == "otel")
                .count(),
            1
        );
        assert_eq!(
            store.observed_cwds().unwrap(),
            vec!["/project-one".to_string(), "/project-two".to_string()]
        );
    }

    #[test]
    fn read_snapshot_stays_consistent_across_an_independent_otel_write() {
        use std::sync::{Arc, Barrier};

        let directory = tempdir().unwrap();
        let database_path = directory.path().join("snapshot.db");
        let reader = Store::open(&database_path).unwrap();
        let writer = Store::open(&database_path).unwrap();
        let write_started = Arc::new(Barrier::new(2));
        let write_finished = Arc::new(Barrier::new(2));
        let writer_started = Arc::clone(&write_started);
        let writer_finished = Arc::clone(&write_finished);
        let handle = std::thread::spawn(move || {
            writer_started.wait();
            writer
                .insert_otel_metadata(otel_event("concurrent-otel", Some("2026-08-27T00:00:02Z")))
                .unwrap();
            writer_finished.wait();
        });

        let (revision_before, revision_after, total_inside) = reader
            .read_snapshot(|conn| {
                let revision_before = reader.activity_revision_on(conn)?;
                let total_before = reader
                    .query_activity_on(
                        conn,
                        &ActivityFilter {
                            view: Some("modelCalls".into()),
                            ..Default::default()
                        },
                        true,
                    )?
                    .total;
                write_started.wait();
                write_finished.wait();
                let revision_after = reader.activity_revision_on(conn)?;
                let total_after = reader
                    .query_activity_on(
                        conn,
                        &ActivityFilter {
                            view: Some("modelCalls".into()),
                            ..Default::default()
                        },
                        true,
                    )?
                    .total;
                assert_eq!(total_before, total_after);
                Ok((revision_before, revision_after, total_after))
            })
            .unwrap();
        handle.join().unwrap();

        assert_eq!(revision_before, revision_after);
        assert_eq!(total_inside, 0);
        let current = reader
            .activity_snapshot(
                &ActivityFilter {
                    view: Some("modelCalls".into()),
                    ..Default::default()
                },
                true,
            )
            .unwrap();
        assert_ne!(current.revision, revision_before);
        assert_eq!(current.page.total, 1);
    }
}
