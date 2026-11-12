BEGIN IMMEDIATE;
ALTER TABLE media_assets ADD COLUMN file_version TEXT;
ALTER TABLE media_assets ADD COLUMN sha256_file_version TEXT;
PRAGMA user_version=10;
COMMIT;
