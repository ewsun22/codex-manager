PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS ingest_files (
  canonical_path TEXT PRIMARY KEY, source_kind TEXT NOT NULL, byte_offset INTEGER NOT NULL DEFAULT 0,
  file_identity TEXT, updated_at TEXT NOT NULL, last_error TEXT
);
CREATE TABLE IF NOT EXISTS sessions (
  session_id TEXT PRIMARY KEY, thread_id TEXT NOT NULL, cli_version TEXT, cwd TEXT, provider TEXT, started_at TEXT, source_path TEXT NOT NULL, observed_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS turns (
  session_id TEXT NOT NULL, turn_id TEXT NOT NULL, model TEXT, effort TEXT, provider TEXT, cwd TEXT, started_at TEXT, completed_at TEXT, duration_ms INTEGER, first_visible_output_ms INTEGER, result TEXT NOT NULL, input_tokens INTEGER NOT NULL, cached_input_tokens INTEGER NOT NULL, cache_write_input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, reasoning_output_tokens INTEGER NOT NULL, total_tokens INTEGER NOT NULL, model_call_count INTEGER NOT NULL, usage_confidence TEXT NOT NULL, PRIMARY KEY(session_id, turn_id), FOREIGN KEY(session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS model_calls (
  event_key TEXT PRIMARY KEY, session_id TEXT NOT NULL, turn_id TEXT NOT NULL, ordinal INTEGER, occurred_at TEXT, model TEXT, effort TEXT, provider TEXT, input_tokens INTEGER NOT NULL, cached_input_tokens INTEGER NOT NULL, cache_write_input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, reasoning_output_tokens INTEGER NOT NULL, total_tokens INTEGER NOT NULL, usage_confidence TEXT NOT NULL, FOREIGN KEY(session_id, turn_id) REFERENCES turns(session_id, turn_id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS timeline_items (
  event_key TEXT PRIMARY KEY, session_id TEXT NOT NULL, turn_id TEXT, ordinal INTEGER, occurred_at TEXT, item_type TEXT NOT NULL, role TEXT, phase TEXT, tool_name TEXT, content_utf8_bytes INTEGER, FOREIGN KEY(session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS projects (
  canonical_path TEXT PRIMARY KEY, name TEXT NOT NULL, source TEXT NOT NULL, exists_flag INTEGER NOT NULL, is_git INTEGER NOT NULL, worktree INTEGER NOT NULL, last_seen_at TEXT, updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS agents_revisions (
  id TEXT PRIMARY KEY, path TEXT NOT NULL, created_at TEXT NOT NULL, before_sha256 TEXT NOT NULL, after_sha256 TEXT NOT NULL, byte_length INTEGER NOT NULL, before_content BLOB NOT NULL, after_content BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS otel_events (
  id TEXT PRIMARY KEY, received_at TEXT NOT NULL, signal TEXT NOT NULL,
  model TEXT, provider TEXT, status_code INTEGER, duration_ms INTEGER,
  response_bytes INTEGER, attributes_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_otel_events_received ON otel_events(received_at DESC);
CREATE TABLE IF NOT EXISTS settings (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1), codex_homes_json TEXT NOT NULL, authorized_roots_json TEXT NOT NULL, retention_days INTEGER NOT NULL, telemetry_enabled INTEGER NOT NULL, price_catalog_version TEXT NOT NULL
);
INSERT OR IGNORE INTO settings(singleton, codex_homes_json, authorized_roots_json, retention_days, telemetry_enabled, price_catalog_version) VALUES (1, '[]', '[]', 30, 0, 'bundled-0');
CREATE INDEX IF NOT EXISTS idx_turns_completed ON turns(completed_at DESC, session_id, turn_id);
CREATE INDEX IF NOT EXISTS idx_turns_project ON turns(cwd, completed_at DESC);
CREATE INDEX IF NOT EXISTS idx_model_calls_turn ON model_calls(session_id, turn_id);
CREATE INDEX IF NOT EXISTS idx_projects_seen ON projects(last_seen_at DESC);
CREATE INDEX IF NOT EXISTS idx_revisions_path ON agents_revisions(path, created_at DESC);
