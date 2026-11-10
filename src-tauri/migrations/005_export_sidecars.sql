BEGIN IMMEDIATE;
CREATE TABLE export_sidecar_items (
  job_id TEXT NOT NULL,
  asset_id TEXT NOT NULL,
  dest_relative_path TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('queued','copying','copied','skipped','failed')),
  copied_bytes INTEGER NOT NULL DEFAULT 0 CHECK(copied_bytes >= 0),
  error_code TEXT,
  verified_sha256 TEXT,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(job_id,asset_id),
  FOREIGN KEY(job_id,asset_id) REFERENCES export_run_items(job_id,asset_id) ON DELETE CASCADE
);
PRAGMA user_version=5;
COMMIT;
