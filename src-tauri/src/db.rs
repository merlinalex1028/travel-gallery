use crate::{
    error::{AppError, Result},
    now,
};
use rusqlite::{Connection, OptionalExtension};
use std::{path::Path, time::Duration};
pub const CATALOG_VERSION: i64 = 10;

pub fn connect(path: &Path) -> Result<Connection> {
    let c = Connection::open(path)?;
    c.busy_timeout(Duration::from_secs(10))?;
    c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
    Ok(c)
}

pub fn initialize(path: &Path) -> Result<()> {
    initialize_inner(path, false)
}

// The selected backup stays read-only, and restore has already backed up the live catalog.
pub(crate) fn initialize_restore_staging(path: &Path) -> Result<()> {
    initialize_inner(path, true)
}

// Call only after closing all connections to this owned staging database.
pub(crate) fn remove_restore_staging(path: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut owned = path.as_os_str().to_os_string();
        owned.push(suffix);
        let _ = std::fs::remove_file(owned);
    }
}

fn initialize_inner(path: &Path, restore_staging: bool) -> Result<()> {
    let c = connect(path)?;
    // WAL persists in the database. Re-selecting it on every read connection
    // needlessly competes with background writers during startup and polling.
    // Staging has no concurrent readers and must not leave WAL/SHM sidecars.
    c.pragma_update(None, "journal_mode", if restore_staging { "DELETE" } else { "WAL" })?;
    let version: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > CATALOG_VERSION {
        return Err(AppError::new(
            "DB_MIGRATION_FAILED",
            "此资料库来自更新版本，请升级旅图",
        ));
    }
    if !restore_staging
        && version < CATALOG_VERSION
        && path.exists()
        && c.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table'",
            [],
            |r| r.get::<_, i64>(0),
        )? > 0
    {
        backup(
            &c,
            &path.with_extension(format!("pre-migration-{}.sqlite", now())),
        )?;
    }
    if version == 0 {
        c.execute_batch(include_str!("../migrations/001_baseline.sql"))?;
    }
    if version < 2 {
        if let Err(e) = c.execute_batch(include_str!("../migrations/002_interactions.sql")) {
            let _ = c.execute_batch("ROLLBACK");
            let message = if restore_staging {
                "备份迁移失败，原备份未修改"
            } else {
                "资料库迁移失败，已保留升级前备份"
            };
            return Err(
                AppError::new("DB_MIGRATION_FAILED", message)
                    .details(serde_json::json!({"reason":e.to_string()})),
            );
        }
    }
    if version < 3 {
        c.execute_batch("BEGIN IMMEDIATE;
          CREATE TABLE source_options(source_id TEXT PRIMARY KEY REFERENCES source_roots(id) ON DELETE CASCADE, options_json TEXT NOT NULL);
          CREATE TABLE query_contexts(fingerprint TEXT PRIMARY KEY, request_json TEXT NOT NULL, expires_at INTEGER NOT NULL);
          CREATE TABLE app_meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL);
          INSERT INTO app_meta VALUES('epoch',0);
          CREATE TABLE cache_access(asset_id TEXT NOT NULL,variant TEXT NOT NULL,last_access INTEGER NOT NULL,PRIMARY KEY(asset_id,variant));
          CREATE TABLE job_errors(id INTEGER PRIMARY KEY,job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,asset_id TEXT,message TEXT NOT NULL);
          PRAGMA user_version=3; COMMIT;")?;
    }
    if version < 4 {
        c.execute_batch(include_str!("../migrations/004_capture_time.sql"))?;
    }
    if version < 5 {
        c.execute_batch(include_str!("../migrations/005_export_sidecars.sql"))?;
    }
    if version < 6 {
        c.execute_batch(include_str!("../migrations/006_review_undo_lineage.sql"))?;
    }
    if version < 7 {
        c.execute_batch(include_str!("../migrations/007_local_ai.sql"))?;
    }
    if version < 8 {
        c.execute_batch(include_str!("../migrations/008_quality_hints.sql"))?;
    }
    if version < 9 {
        c.execute_batch(include_str!("../migrations/009_heif_preview_orientation.sql"))?;
    }
    if version < 10 {
        c.execute_batch(include_str!("../migrations/010_exact_file_version.sql"))?;
    }
    c.execute("UPDATE jobs SET status='failed',error_summary='上次运行被中断，可安全重试',updated_at=? WHERE status IN ('running','queued','paused')", [now()])?;
    c.execute("DELETE FROM query_contexts WHERE expires_at<?", [now()])?;
    c.execute(
        "DELETE FROM selection_snapshots WHERE expires_at<?",
        [now()],
    )?;
    c.execute(
        "DELETE FROM export_plans WHERE expires_at<? AND consumed_by_job_id IS NULL",
        [now()],
    )?;
    c.execute_batch("CREATE INDEX IF NOT EXISTS idx_assets_capture_sort ON media_assets(COALESCE(capture_local,datetime(mtime_ms/1000,'unixepoch'),''),id); CREATE INDEX IF NOT EXISTS idx_assets_filename ON media_assets(filename COLLATE NOCASE,id);")?;
    c.execute_batch("CREATE INDEX IF NOT EXISTS idx_assets_file_identity ON media_assets(source_id,file_identity,byte_size,mtime_ms) WHERE file_identity IS NOT NULL;")?;
    c.execute_batch("CREATE INDEX IF NOT EXISTS idx_links_secondary_confirmed ON linked_media(secondary_asset_id,confirmed,primary_asset_id);")?;
    c.execute_batch("CREATE INDEX IF NOT EXISTS idx_assets_source_state ON media_assets(source_id,file_state);")?;
    Ok(())
}
pub fn epoch(c: &Connection) -> Result<i64> {
    Ok(
        c.query_row("SELECT value FROM app_meta WHERE key='epoch'", [], |r| {
            r.get(0)
        })?,
    )
}
pub fn bump(c: &Connection) -> Result<()> {
    c.execute("UPDATE app_meta SET value=value+1 WHERE key='epoch'", [])?;
    Ok(())
}
pub fn backup(c: &Connection, dest: &Path) -> Result<()> {
    backup_controlled(c, dest, || Ok(()))
}
pub fn backup_controlled(
    c: &Connection,
    dest: &Path,
    mut control: impl FnMut() -> Result<()>,
) -> Result<()> {
    if dest.exists() {
        return Err(AppError::new("EXPORT_CONFLICT", "备份文件已存在"));
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)?;
    let result = (|| {
        let mut target = Connection::open(dest)?;
        let b = rusqlite::backup::Backup::new(c, &mut target)?;
        let mut blocked_since = None;
        loop {
            control()?;
            match b.step(100)? {
                rusqlite::backup::StepResult::Done => break,
                rusqlite::backup::StepResult::More => {
                    blocked_since = None;
                }
                _ => {
                    let since = blocked_since.get_or_insert_with(std::time::Instant::now);
                    if since.elapsed() > Duration::from_secs(10) {
                        return Err(AppError::new("DB_CONFLICT", "资料库忙，请稍后重试备份"));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
        drop(b);
        // The backup is published as a single file. It must open without
        // creating WAL or SHM files next to the user's selected backup.
        let journal_mode: String = target.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
        if journal_mode != "delete" {
            return Err(AppError::new("DB_CONFLICT", "无法生成可独立读取的备份"));
        }
        drop(target);
        std::fs::OpenOptions::new()
            .write(true)
            .open(dest)?
            .sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}
pub fn setting(c: &Connection, key: &str) -> Result<Option<String>> {
    Ok(c.query_row(
        "SELECT value_json FROM app_settings WHERE key=?",
        [key],
        |r| r.get(0),
    )
    .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v9_catalog(path: &Path) {
        initialize(path).unwrap();
        let c = connect(path).unwrap();
        c.execute("INSERT INTO source_roots(id,display_name,absolute_path,created_at,updated_at) VALUES('s','fixture','/fixture',0,0)", []).unwrap();
        c.execute("INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,mtime_ms,indexed_at,updated_at) VALUES('a','s','a.jpg','a.jpg','jpg','image',123,0,0,0)", []).unwrap();
        c.execute("UPDATE asset_reviews SET pick_state='keep',rating=4,note='preserve',revision=7 WHERE asset_id='a'", []).unwrap();
        c.execute_batch(
            "ALTER TABLE media_assets DROP COLUMN file_version;
             ALTER TABLE media_assets DROP COLUMN sha256_file_version;
             PRAGMA user_version=9;",
        )
        .unwrap();
    }

    fn assert_review(c: &Connection) {
        let review = c.query_row(
            "SELECT pick_state,rating,note,revision FROM asset_reviews WHERE asset_id='a'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?)),
        ).unwrap();
        assert_eq!(review, ("keep".into(), 4, "preserve".into(), 7));
    }

    #[test]
    fn backup_of_live_wal_catalog_can_be_read_as_one_file() {
        let directory = tempfile::tempdir().unwrap();
        let live_path = directory.path().join("catalog.sqlite");
        let backup_path = directory.path().join("backup.sqlite");
        v9_catalog(&live_path);
        let live = connect(&live_path).unwrap();
        live.execute("UPDATE asset_reviews SET updated_at=42 WHERE asset_id='a'", []).unwrap();
        backup(&live, &backup_path).unwrap();

        let header = std::fs::read(&backup_path).unwrap();
        assert_eq!(&header[18..20], &[1, 1]);
        let read_only = Connection::open_with_flags(&backup_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_review(&read_only);
        let updated_at: i64 = read_only.query_row(
            "SELECT updated_at FROM asset_reviews WHERE asset_id='a'", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(updated_at, 42);
        assert!(!directory.path().join("backup.sqlite-wal").exists());
        assert!(!directory.path().join("backup.sqlite-shm").exists());
        assert_eq!(live.pragma_query_value::<String, _>(None, "journal_mode", |r| r.get(0)).unwrap(), "wal");
    }

    #[test]
    fn normal_v9_migration_retains_a_readable_pre_migration_backup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.sqlite");
        v9_catalog(&path);
        initialize(&path).unwrap();
        let backups: Vec<_> = std::fs::read_dir(directory.path()).unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|entry| entry.file_name().unwrap().to_string_lossy().contains("pre-migration"))
            .collect();
        assert_eq!(backups.len(), 1);
        let before = connect(&backups[0]).unwrap();
        assert_eq!(before.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0)).unwrap(), 9);
        assert!(before.prepare("SELECT file_version FROM media_assets").is_err());
        assert_review(&before);
        let after = connect(&path).unwrap();
        assert_eq!(after.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0)).unwrap(), CATALOG_VERSION);
        assert_eq!(after.pragma_query_value::<String, _>(None, "journal_mode", |r| r.get(0)).unwrap(), "wal");
        after.prepare("SELECT file_version,sha256_file_version FROM media_assets").unwrap();
        assert_review(&after);
    }

    #[test]
    fn restore_staging_migrates_without_duplicate_backups_or_source_changes() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("selected.sqlite");
        let staged = directory.path().join("restore-staging-test.sqlite");
        v9_catalog(&source);
        let source_before = std::fs::read(&source).unwrap();
        {
            let selected = Connection::open_with_flags(&source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            backup(&selected, &staged).unwrap();
        }
        initialize_restore_staging(&staged).unwrap();
        {
            let c = connect(&staged).unwrap();
            assert_eq!(c.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0)).unwrap(), CATALOG_VERSION);
            assert_eq!(c.pragma_query_value::<String, _>(None, "journal_mode", |r| r.get(0)).unwrap(), "delete");
            c.prepare("SELECT file_version,sha256_file_version FROM media_assets").unwrap();
            assert_review(&c);
        }
        assert_eq!(std::fs::read(&source).unwrap(), source_before);
        remove_restore_staging(&staged);
        assert!(!std::fs::read_dir(directory.path()).unwrap().any(|entry| {
            entry.unwrap().file_name().to_string_lossy().starts_with("restore-staging-")
        }));
    }

    #[test]
    fn failed_staging_migration_keeps_the_source_and_no_duplicate_backup() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("selected.sqlite");
        let staged = directory.path().join("restore-staging-test.sqlite");
        v9_catalog(&source);
        // A readable v9 backup can still contain a conflicting newer column.
        connect(&source).unwrap().execute_batch(
            "ALTER TABLE media_assets ADD COLUMN sha256_file_version TEXT;"
        ).unwrap();
        let source_before = std::fs::read(&source).unwrap();
        {
            let selected = Connection::open_with_flags(&source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            backup(&selected, &staged).unwrap();
        }
        let error = initialize_restore_staging(&staged).unwrap_err();
        assert!(error.details["reason"].as_str().unwrap().contains("duplicate column"));
        {
            let c = connect(&staged).unwrap();
            assert_eq!(c.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0)).unwrap(), 9);
            assert!(c.prepare("SELECT file_version FROM media_assets").is_err());
            assert_review(&c);
        }
        assert_eq!(std::fs::read(&source).unwrap(), source_before);
        remove_restore_staging(&staged);
        assert!(!std::fs::read_dir(directory.path()).unwrap().any(|entry| {
            entry.unwrap().file_name().to_string_lossy().starts_with("restore-staging-")
        }));
    }

    #[test]
    fn v9_rebuilds_only_heif_derivatives_and_keeps_reviews_and_hint_history() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.sqlite");
        initialize(&path).unwrap();
        let c = connect(&path).unwrap();
        c.execute("INSERT INTO source_roots(id,display_name,absolute_path,created_at,updated_at) VALUES('s','fixture','/fixture',0,0)", []).unwrap();
        for (id, ext) in [("heif", "heic"), ("jpeg", "jpg")] {
            c.execute("INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,mtime_ms,perceptual_hash,perceptual_hash_algorithm,indexed_at,updated_at) VALUES(?,'s',?,?,?,'image',123,0,'0123456789abcdef','dhash64-v1',0,0)", rusqlite::params![id,format!("{id}.{ext}"),format!("{id}.{ext}"),ext]).unwrap();
            c.execute("UPDATE asset_reviews SET pick_state='keep',rating=4,is_favorite=1,note='keep my note',revision=7 WHERE asset_id=?", [id]).unwrap();
            c.execute("INSERT INTO thumbnail_cache VALUES(?,'small',?,0,123,1,0)", rusqlite::params![id,format!("{}.jpg", (if id=="heif"{"a"}else{"b"}).repeat(64))]).unwrap();
            c.execute("INSERT INTO ai_embeddings VALUES(?,'old',123,0,1,?,768,zeroblob(3072),0)", rusqlite::params![id,"a".repeat(64)]).unwrap();
            c.execute("INSERT INTO quality_observations VALUES(?,'detail','old',123,0,1,?,'{}',0)", rusqlite::params![id,"a".repeat(64)]).unwrap();
            c.execute("INSERT INTO quality_hint_reviews VALUES(?,'detail',?,0)", rusqlite::params![id,"a".repeat(64)]).unwrap();
        }
        for kind in ["exact", "similar"] {
            c.execute("INSERT INTO similar_groups VALUES(?,?,?,'v1','heif',0)", rusqlite::params![kind,kind,kind]).unwrap();
            c.execute("INSERT INTO similar_group_items VALUES(?,'heif',0,0)", [kind]).unwrap();
            c.execute("INSERT INTO similar_group_items VALUES(?,'jpeg',0,0)", [kind]).unwrap();
        }
        c.execute_batch(
            "ALTER TABLE media_assets DROP COLUMN file_version;
                         ALTER TABLE media_assets DROP COLUMN sha256_file_version;
                         PRAGMA user_version=8;",
        )
        .unwrap();
        drop(c);
        initialize(&path).unwrap();
        let c = connect(&path).unwrap();
        assert_eq!(
            c.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
                .unwrap(),
            CATALOG_VERSION
        );
        for table in ["ai_embeddings", "quality_observations"] {
            assert_eq!(c.query_row(&format!("SELECT count(*) FROM {table} WHERE asset_id='heif'"),[],|r|r.get::<_,i64>(0)).unwrap(),0);
            assert_eq!(c.query_row(&format!("SELECT count(*) FROM {table} WHERE asset_id='jpeg'"),[],|r|r.get::<_,i64>(0)).unwrap(),1);
        }
        assert_eq!(c.query_row("SELECT count(*) FROM thumbnail_cache WHERE format_version=1",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        assert_eq!(c.query_row("SELECT count(*) FROM asset_reviews WHERE pick_state='keep' AND rating=4 AND is_favorite=1 AND note='keep my note' AND revision=7",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        assert_eq!(c.query_row("SELECT count(*) FROM quality_hint_reviews",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        assert_eq!(c.query_row("SELECT count(*) FROM media_assets WHERE id='heif' AND perceptual_hash IS NULL",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(c.query_row("SELECT count(*) FROM media_assets WHERE id='jpeg' AND perceptual_hash='0123456789abcdef'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(c.query_row("SELECT group_type FROM similar_groups",[],|r|r.get::<_,String>(0)).unwrap(),"exact");
        assert!(std::fs::read_dir(directory.path()).unwrap().any(|entry|entry.unwrap().file_name().to_string_lossy().contains("pre-migration")));
    }
}
