-- `session_id` and `event_key` used to be raw rollout payload identifiers.
-- Keep those values for display, but use namespaced internal primary keys for
-- every newly ingested source so copied or unrelated rollout files cannot
-- merge one another's rows.
ALTER TABLE sessions ADD COLUMN public_session_id TEXT;
ALTER TABLE sessions ADD COLUMN source_namespace TEXT;
ALTER TABLE model_calls ADD COLUMN public_event_key TEXT;
ALTER TABLE timeline_items ADD COLUMN public_event_key TEXT;

UPDATE sessions
SET public_session_id = session_id,
    source_namespace = 'legacy:' || source_path
WHERE public_session_id IS NULL OR source_namespace IS NULL;
UPDATE model_calls SET public_event_key = event_key WHERE public_event_key IS NULL;
UPDATE timeline_items SET public_event_key = event_key WHERE public_event_key IS NULL;

CREATE INDEX IF NOT EXISTS idx_sessions_namespace ON sessions(source_namespace, public_session_id);
CREATE INDEX IF NOT EXISTS idx_model_calls_public_event ON model_calls(public_event_key);
CREATE INDEX IF NOT EXISTS idx_timeline_public_event ON timeline_items(public_event_key);
