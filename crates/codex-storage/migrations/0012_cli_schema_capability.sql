-- The CLI schema probe is a compatibility diagnostic, not an activity source.
-- Persist only its bounded display metadata so application restarts do not
-- regress a verified CLI into an ambiguous "not checked" state.
CREATE TABLE IF NOT EXISTS cli_schema_capability (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  executable_path TEXT NOT NULL,
  version TEXT,
  schema_sha256 TEXT,
  checked_at TEXT NOT NULL,
  available INTEGER NOT NULL CHECK (available IN (0, 1)),
  message TEXT
);
