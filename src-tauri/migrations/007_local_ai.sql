BEGIN IMMEDIATE;
-- Regenerable model output is separate from human ratings, tags and albums.
CREATE TABLE ai_embeddings (
  asset_id TEXT PRIMARY KEY REFERENCES media_assets(id) ON DELETE CASCADE,
  model_version TEXT NOT NULL,
  source_size INTEGER NOT NULL CHECK(source_size >= 0),
  source_mtime_ms INTEGER,
  orientation INTEGER NOT NULL,
  input_key TEXT NOT NULL CHECK(length(input_key)=64),
  dimensions INTEGER NOT NULL CHECK(dimensions=768),
  vector BLOB NOT NULL CHECK(length(vector)=3072),
  created_at INTEGER NOT NULL
);
CREATE INDEX ai_embeddings_model ON ai_embeddings(model_version);
CREATE TABLE ai_clusters (
  id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  model_version TEXT NOT NULL,
  algorithm_version TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX ai_clusters_job ON ai_clusters(job_id,created_at,id);
CREATE TABLE ai_cluster_members (
  cluster_id TEXT NOT NULL REFERENCES ai_clusters(id) ON DELETE CASCADE,
  asset_id TEXT NOT NULL REFERENCES media_assets(id) ON DELETE CASCADE,
  input_key TEXT NOT NULL CHECK(length(input_key)=64),
  similarity REAL NOT NULL CHECK(similarity BETWEEN -1.0001 AND 1.0001),
  PRIMARY KEY(cluster_id,asset_id)
);
CREATE INDEX ai_cluster_asset ON ai_cluster_members(asset_id);
PRAGMA user_version=7;
COMMIT;
