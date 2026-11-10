BEGIN IMMEDIATE;
CREATE TABLE review_undo_lineage (
  asset_id TEXT NOT NULL REFERENCES asset_reviews(asset_id) ON DELETE CASCADE,
  undo_revision INTEGER NOT NULL CHECK(undo_revision > 0),
  restored_revision INTEGER NOT NULL CHECK(restored_revision >= 0 AND restored_revision < undo_revision),
  change_batch_id TEXT NOT NULL REFERENCES change_batches(id) ON DELETE CASCADE,
  PRIMARY KEY(asset_id,undo_revision)
);
PRAGMA user_version=6;
COMMIT;
