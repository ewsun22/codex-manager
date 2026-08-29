ALTER TABLE settings ADD COLUMN update_check_interval_hours INTEGER NOT NULL DEFAULT 12;

CREATE TABLE IF NOT EXISTS app_update_status (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  checked_at TEXT NOT NULL,
  current_version TEXT NOT NULL,
  available INTEGER NOT NULL CHECK(available IN (0, 1)),
  version TEXT,
  release_date TEXT,
  notes TEXT
);

CREATE TABLE IF NOT EXISTS app_update_check_attempt (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  last_attempt_at TEXT NOT NULL,
  current_version TEXT NOT NULL
);
