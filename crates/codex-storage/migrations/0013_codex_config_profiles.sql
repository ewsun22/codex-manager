-- Codex configuration profiles contain routing metadata only.  The opaque
-- secret_ref is resolved by the desktop credential helper; bearer/API tokens
-- must never be persisted in SQLite.
CREATE TABLE IF NOT EXISTS codex_config_profiles (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('official-direct', 'local-cliproxy', 'external-compatible')),
  model TEXT NOT NULL,
  reasoning_effort TEXT,
  base_url TEXT,
  secret_ref TEXT,
  active INTEGER NOT NULL DEFAULT 0 CHECK (active IN (0, 1)),
  verified_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_codex_config_profiles_updated
  ON codex_config_profiles(updated_at DESC, id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_codex_config_profiles_one_active
  ON codex_config_profiles(active) WHERE active = 1;
