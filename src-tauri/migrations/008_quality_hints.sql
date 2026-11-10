BEGIN IMMEDIATE;
-- Rebuildable observations and human dismissal decisions are separate from
-- asset_reviews. A new source/model input never inherits an old dismissal.
CREATE TABLE quality_observations (
 asset_id TEXT NOT NULL REFERENCES media_assets(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN ('eyes','detail')),
 algorithm_version TEXT NOT NULL,
 source_size INTEGER NOT NULL CHECK(source_size>=0),
 source_mtime_ms INTEGER,
 orientation INTEGER NOT NULL,
 input_key TEXT NOT NULL CHECK(length(input_key)=64),
 result_json TEXT NOT NULL CHECK(json_valid(result_json)),
 created_at INTEGER NOT NULL,
 PRIMARY KEY(asset_id,kind)
);
CREATE INDEX quality_observations_version ON quality_observations(kind,algorithm_version);
CREATE TABLE quality_hint_reviews (
 asset_id TEXT NOT NULL REFERENCES media_assets(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN ('eyes','detail')),
 input_key TEXT NOT NULL CHECK(length(input_key)=64),
 dismissed_at INTEGER NOT NULL,
 PRIMARY KEY(asset_id,kind,input_key)
);
PRAGMA user_version=8;
COMMIT;
