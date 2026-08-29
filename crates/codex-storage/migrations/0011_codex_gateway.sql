CREATE TABLE IF NOT EXISTS codex_providers (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  base_url TEXT NOT NULL,
  model TEXT NOT NULL,
  reasoning_effort TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_codex_providers_updated ON codex_providers(updated_at DESC, id);

ALTER TABLE settings ADD COLUMN codex_gateway_port INTEGER NOT NULL DEFAULT 8318
  CHECK(codex_gateway_port BETWEEN 1024 AND 65535);
