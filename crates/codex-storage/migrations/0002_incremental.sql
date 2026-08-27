ALTER TABLE ingest_files ADD COLUMN resume_state_json TEXT;

CREATE INDEX IF NOT EXISTS idx_sessions_source_path ON sessions(source_path);
CREATE INDEX IF NOT EXISTS idx_model_calls_activity ON model_calls(occurred_at DESC, event_key DESC);
