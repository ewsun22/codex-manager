ALTER TABLE agents_revisions ADD COLUMN status TEXT NOT NULL DEFAULT 'applied';
CREATE INDEX IF NOT EXISTS idx_revisions_status_created ON agents_revisions(status, created_at);
