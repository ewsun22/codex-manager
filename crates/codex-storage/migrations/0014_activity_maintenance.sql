-- Keep the retention boundary across incremental scans without discarding
-- their parser checkpoints. The revision covers deletions, which cannot be
-- observed through ingest offsets or the largest surviving OTel rowid.
CREATE TABLE IF NOT EXISTS activity_maintenance (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  retention_cutoff TEXT,
  deletion_revision INTEGER NOT NULL DEFAULT 0
);
INSERT OR IGNORE INTO activity_maintenance(singleton) VALUES(1);

-- Retention removes timeline metadata by its parent turn before deleting it.
CREATE INDEX IF NOT EXISTS idx_timeline_items_turn
  ON timeline_items(session_id, turn_id);
