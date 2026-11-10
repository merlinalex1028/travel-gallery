BEGIN IMMEDIATE;
CREATE TABLE capture_time_corrections (
  asset_id TEXT PRIMARY KEY REFERENCES media_assets(id) ON DELETE CASCADE,
  original_local TEXT,
  original_utc_ms INTEGER,
  original_tz_min INTEGER,
  original_source TEXT,
  corrected_local TEXT,
  corrected_utc_ms INTEGER,
  corrected_tz_min INTEGER,
  is_corrected INTEGER NOT NULL CHECK(is_corrected IN (0,1)),
  revision INTEGER NOT NULL CHECK(revision >= 1),
  updated_at INTEGER NOT NULL
);
PRAGMA user_version=4;
COMMIT;
