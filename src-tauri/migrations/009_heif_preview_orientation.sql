BEGIN IMMEDIATE;
-- HEIF conversion now honors the conversion JPEG's own orientation. Old
-- pixels/observations and affected similarity groups can be rebuilt. Asset
-- reviews, tags, links, and version-bound hint-dismissal history stay.
-- Legacy pixel-version-1 cache rows remain for offline viewing. Online reads
-- require version 2 and replace these rows after a successful rebuild.
DELETE FROM ai_embeddings WHERE asset_id IN (SELECT id FROM media_assets WHERE lower(extension) IN ('heic','heif'));
DELETE FROM quality_observations WHERE asset_id IN (SELECT id FROM media_assets WHERE lower(extension) IN ('heic','heif'));
DELETE FROM similar_groups WHERE group_type='similar' AND id IN (SELECT i.group_id FROM similar_group_items i JOIN media_assets a ON a.id=i.asset_id WHERE lower(a.extension) IN ('heic','heif') AND a.perceptual_hash IS NOT NULL);
UPDATE media_assets SET perceptual_hash=NULL,perceptual_hash_algorithm=NULL,phash_mtime_ms=NULL,phash_byte_size=NULL WHERE lower(extension) IN ('heic','heif');
UPDATE app_meta SET value=value+1 WHERE key='epoch' AND EXISTS(SELECT 1 FROM media_assets WHERE lower(extension) IN ('heic','heif'));
PRAGMA user_version=9;
COMMIT;
