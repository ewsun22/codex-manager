-- Source namespaces prevent ownership transfer between unrelated files. These
-- fingerprints are a separate read-time identity: exact byte-equivalent
-- rollout copies can collapse in dashboards without sharing mutable ownership.
ALTER TABLE turns ADD COLUMN logical_fingerprint TEXT;
ALTER TABLE model_calls ADD COLUMN logical_fingerprint TEXT;

CREATE INDEX IF NOT EXISTS idx_turns_logical_fingerprint
  ON turns(logical_fingerprint, session_id, turn_id);
CREATE INDEX IF NOT EXISTS idx_model_calls_logical_fingerprint
  ON model_calls(logical_fingerprint, event_key);
