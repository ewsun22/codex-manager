-- v2 fingerprints identify public events rather than mutable display
-- enrichment. Clearing once makes the normal NULL-only backfill recompute
-- historical values after this migration while keeping later startups cheap.
UPDATE turns SET logical_fingerprint = NULL;
UPDATE model_calls SET logical_fingerprint = NULL;

CREATE INDEX IF NOT EXISTS idx_turns_canonical_lookup
  ON turns(logical_fingerprint, result, completed_at, session_id, turn_id);
CREATE INDEX IF NOT EXISTS idx_model_calls_canonical_lookup
  ON model_calls(logical_fingerprint, event_key);
