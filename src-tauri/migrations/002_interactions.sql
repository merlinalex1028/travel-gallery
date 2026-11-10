-- Travel Gallery schema enhancement v1.1; execute exactly once after baseline schema.sql.
-- Migration runner must check PRAGMA user_version / own migration ledger first.
-- Do NOT re-run blindly: SQLite ADD COLUMN doesn't support IF NOT EXISTS.
PRAGMA foreign_keys = ON;
BEGIN IMMEDIATE;

ALTER TABLE source_roots ADD COLUMN scan_generation INTEGER NOT NULL DEFAULT 0;
ALTER TABLE source_roots ADD COLUMN last_seen_at INTEGER;

ALTER TABLE media_assets ADD COLUMN last_seen_scan_generation INTEGER NOT NULL DEFAULT 0;
ALTER TABLE media_assets ADD COLUMN file_identity TEXT;
ALTER TABLE media_assets ADD COLUMN sha256_mtime_ms INTEGER;
ALTER TABLE media_assets ADD COLUMN sha256_byte_size INTEGER;
ALTER TABLE media_assets ADD COLUMN phash_mtime_ms INTEGER;
ALTER TABLE media_assets ADD COLUMN phash_byte_size INTEGER;

ALTER TABLE asset_reviews ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;

ALTER TABLE jobs ADD COLUMN failure_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN total_is_known INTEGER NOT NULL DEFAULT 0 CHECK(total_is_known IN (0,1));

CREATE TABLE selection_snapshots (
  id TEXT PRIMARY KEY,
  filter_json TEXT NOT NULL,
  sort_json TEXT NOT NULL,
  query_fingerprint TEXT NOT NULL,
  initial_count INTEGER NOT NULL CHECK(initial_count >= 0),
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE TABLE selection_snapshot_items (
  snapshot_id TEXT NOT NULL REFERENCES selection_snapshots(id) ON DELETE CASCADE,
  asset_id TEXT NOT NULL REFERENCES media_assets(id) ON DELETE CASCADE,
  PRIMARY KEY(snapshot_id,asset_id)
);
CREATE INDEX idx_selection_snapshot_items_asset ON selection_snapshot_items(asset_id);
CREATE INDEX idx_selection_snapshots_expiry ON selection_snapshots(expires_at);

CREATE TABLE mutation_receipts (
  client_mutation_id TEXT PRIMARY KEY,
  payload_sha256 TEXT NOT NULL,
  result_json TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_mutation_receipts_created ON mutation_receipts(created_at);

CREATE TABLE export_plans (
  token TEXT PRIMARY KEY,
  snapshot_id TEXT REFERENCES selection_snapshots(id) ON DELETE SET NULL,
  plan_json TEXT NOT NULL,
  total_bytes INTEGER NOT NULL CHECK(total_bytes >= 0),
  count_files INTEGER NOT NULL CHECK(count_files >= 0),
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  consumed_by_job_id TEXT REFERENCES jobs(id) ON DELETE SET NULL
);
CREATE INDEX idx_export_plans_expiry ON export_plans(expires_at);

CREATE TABLE export_run_items (
  job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
  asset_id TEXT NOT NULL REFERENCES media_assets(id) ON DELETE RESTRICT,
  dest_relative_path TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('queued','copying','copied','skipped','failed')),
  copied_bytes INTEGER NOT NULL DEFAULT 0 CHECK(copied_bytes >= 0),
  error_code TEXT,
  verified_sha256 TEXT,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(job_id,asset_id)
);
CREATE INDEX idx_export_run_items_state ON export_run_items(job_id,state);
CREATE INDEX idx_assets_seen_generation ON media_assets(source_id,last_seen_scan_generation);

PRAGMA user_version = 2;
COMMIT;
