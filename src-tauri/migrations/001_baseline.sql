-- Travel Gallery / 旅图 - illustrative SQLite baseline schema (v0.1 + reserved P1 tables)
-- SQL is executable on SQLite 3.35+. UUIDs generated in application layer.
-- PRAGMA journal_mode is connection-level; apply at app startup too.
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;

CREATE TABLE IF NOT EXISTS source_roots (
  id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  absolute_path TEXT NOT NULL,
  volume_id TEXT,
  is_enabled INTEGER NOT NULL DEFAULT 1 CHECK(is_enabled IN (0,1)),
  is_online INTEGER NOT NULL DEFAULT 1 CHECK(is_online IN (0,1)),
  last_scan_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS media_assets (
  id TEXT PRIMARY KEY,
  source_id TEXT NOT NULL REFERENCES source_roots(id) ON DELETE RESTRICT,
  relative_path TEXT NOT NULL,
  filename TEXT NOT NULL,
  extension TEXT NOT NULL,
  media_kind TEXT NOT NULL CHECK(media_kind IN ('image','video')),
  byte_size INTEGER NOT NULL CHECK(byte_size >= 0),
  mtime_ms INTEGER,
  capture_local TEXT,
  capture_utc_ms INTEGER,
  capture_tz_min INTEGER,
  capture_time_source TEXT CHECK(capture_time_source IN ('exif','quicktime','manual','mtime','unknown')),
  width INTEGER CHECK(width IS NULL OR width > 0),
  height INTEGER CHECK(height IS NULL OR height > 0),
  orientation INTEGER,
  duration_ms INTEGER CHECK(duration_ms IS NULL OR duration_ms >= 0),
  codec TEXT,
  camera_make TEXT,
  camera_model TEXT,
  gps_lat REAL CHECK(gps_lat IS NULL OR (gps_lat BETWEEN -90 AND 90)),
  gps_lon REAL CHECK(gps_lon IS NULL OR (gps_lon BETWEEN -180 AND 180)),
  file_state TEXT NOT NULL DEFAULT 'available'
    CHECK(file_state IN ('available','offline','missing','unsupported','corrupt')),
  sha256 TEXT CHECK(sha256 IS NULL OR length(sha256) = 64),
  perceptual_hash TEXT,
  perceptual_hash_algorithm TEXT,
  metadata_json TEXT,
  indexed_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(source_id, relative_path)
);

CREATE TABLE IF NOT EXISTS asset_reviews (
  asset_id TEXT PRIMARY KEY REFERENCES media_assets(id) ON DELETE CASCADE,
  pick_state TEXT NOT NULL DEFAULT 'unreviewed'
    CHECK(pick_state IN ('unreviewed','keep','maybe','reject')),
  rating INTEGER NOT NULL DEFAULT 0 CHECK(rating BETWEEN 0 AND 5),
  is_favorite INTEGER NOT NULL DEFAULT 0 CHECK(is_favorite IN (0,1)),
  note TEXT NOT NULL DEFAULT '',
  updated_at INTEGER NOT NULL
);

CREATE TRIGGER IF NOT EXISTS trg_asset_create_review
AFTER INSERT ON media_assets
BEGIN
  INSERT INTO asset_reviews(asset_id, updated_at)
  VALUES (NEW.id, NEW.indexed_at);
END;

CREATE TABLE IF NOT EXISTS tags (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL COLLATE NOCASE UNIQUE,
  color TEXT,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS asset_tags (
  asset_id TEXT NOT NULL REFERENCES media_assets(id) ON DELETE CASCADE,
  tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY(asset_id, tag_id)
);

CREATE TABLE IF NOT EXISTS collections (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('album','trip')),
  cover_asset_id TEXT REFERENCES media_assets(id) ON DELETE SET NULL,
  description TEXT,
  start_at INTEGER,
  end_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS collection_assets (
  collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
  asset_id TEXT NOT NULL REFERENCES media_assets(id) ON DELETE CASCADE,
  sort_order INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(collection_id, asset_id)
);
CREATE TABLE IF NOT EXISTS saved_filters (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  filter_json TEXT NOT NULL,
  sort_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS similar_groups (
  id TEXT PRIMARY KEY,
  group_type TEXT NOT NULL CHECK(group_type IN ('exact','similar')),
  algorithm TEXT NOT NULL,
  algorithm_version TEXT NOT NULL,
  representative_asset_id TEXT REFERENCES media_assets(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS similar_group_items (
  group_id TEXT NOT NULL REFERENCES similar_groups(id) ON DELETE CASCADE,
  asset_id TEXT NOT NULL REFERENCES media_assets(id) ON DELETE CASCADE,
  distance INTEGER CHECK(distance IS NULL OR distance >= 0),
  manually_excluded INTEGER NOT NULL DEFAULT 0 CHECK(manually_excluded IN (0,1)),
  PRIMARY KEY(group_id, asset_id)
);

CREATE TABLE IF NOT EXISTS thumbnail_cache (
  asset_id TEXT NOT NULL REFERENCES media_assets(id) ON DELETE CASCADE,
  variant TEXT NOT NULL CHECK(variant IN ('small','medium','screen','poster')),
  cache_rel_path TEXT NOT NULL,
  source_mtime_ms INTEGER,
  source_byte_size INTEGER,
  format_version INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  PRIMARY KEY(asset_id, variant)
);

CREATE TABLE IF NOT EXISTS jobs (
  id TEXT PRIMARY KEY,
  type TEXT NOT NULL CHECK(type IN ('SCAN','METADATA','THUMBNAIL','HASH_EXACT','HASH_SIMILAR','EXPORT','BACKUP','REPAIR')),
  status TEXT NOT NULL CHECK(status IN ('queued','running','paused','completed','failed','cancelled')),
  progress_done INTEGER NOT NULL DEFAULT 0 CHECK(progress_done >= 0),
  progress_total INTEGER NOT NULL DEFAULT 0 CHECK(progress_total >= 0),
  checkpoint_json TEXT,
  error_summary TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS change_batches (
  id TEXT PRIMARY KEY,
  action TEXT NOT NULL,
  payload_json TEXT,
  created_at INTEGER NOT NULL,
  undone_at INTEGER
);
CREATE TABLE IF NOT EXISTS change_batch_items (
  batch_id TEXT NOT NULL REFERENCES change_batches(id) ON DELETE CASCADE,
  asset_id TEXT NOT NULL REFERENCES media_assets(id) ON DELETE CASCADE,
  before_json TEXT NOT NULL,
  after_json TEXT NOT NULL,
  PRIMARY KEY(batch_id, asset_id)
);

CREATE TABLE IF NOT EXISTS linked_media (
  id TEXT PRIMARY KEY,
  primary_asset_id TEXT NOT NULL REFERENCES media_assets(id) ON DELETE CASCADE,
  secondary_asset_id TEXT NOT NULL REFERENCES media_assets(id) ON DELETE CASCADE,
  link_type TEXT NOT NULL CHECK(link_type IN ('live_photo','raw_jpeg')),
  match_method TEXT NOT NULL,
  confidence REAL CHECK(confidence IS NULL OR confidence BETWEEN 0 AND 1),
  confirmed INTEGER NOT NULL DEFAULT 0 CHECK(confirmed IN (0,1)),
  CHECK(primary_asset_id <> secondary_asset_id),
  UNIQUE(primary_asset_id,secondary_asset_id,link_type)
);

CREATE TABLE IF NOT EXISTS app_settings (
  key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_assets_capture ON media_assets(capture_utc_ms, id);
CREATE INDEX IF NOT EXISTS idx_assets_source_time ON media_assets(source_id, capture_utc_ms, id);
CREATE INDEX IF NOT EXISTS idx_assets_file_state ON media_assets(file_state);
CREATE INDEX IF NOT EXISTS idx_assets_size ON media_assets(byte_size);
CREATE INDEX IF NOT EXISTS idx_assets_sha256 ON media_assets(sha256) WHERE sha256 IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_assets_phash ON media_assets(perceptual_hash) WHERE perceptual_hash IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_reviews_pick_rating ON asset_reviews(pick_state, rating, asset_id);
CREATE INDEX IF NOT EXISTS idx_reviews_favorite ON asset_reviews(is_favorite) WHERE is_favorite = 1;
CREATE INDEX IF NOT EXISTS idx_asset_tags_tag ON asset_tags(tag_id, asset_id);
CREATE INDEX IF NOT EXISTS idx_similar_group_kind ON similar_groups(group_type, created_at);
CREATE INDEX IF NOT EXISTS idx_similar_group_asset ON similar_group_items(asset_id);
CREATE INDEX IF NOT EXISTS idx_jobs_state ON jobs(status, updated_at);
CREATE INDEX IF NOT EXISTS idx_collections_kind ON collections(kind, updated_at);

-- Note:
-- 1. Use transaction-scoped migrations with a PRAGMA user_version bump.
-- 2. Source path canonicalization and case-folding differ between Windows/macOS volumes:
--    validate identity in Rust, don't assume SQLite NOCASE handles filesystem equivalence.
-- 3. sha256/vision hashes apply only when decoding/hashing succeeded and store algorithm version.
-- 4. The trigger creates a default asset review in every successfully indexed asset.
-- 5. For large asset deletion operations, ensure change batch and similar group cleanup are intentional.
