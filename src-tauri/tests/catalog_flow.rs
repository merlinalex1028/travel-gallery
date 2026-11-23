use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;
use travel_gallery::{catalog::Catalog, db, files, id, media};

struct Fixture {
    _temp: TempDir,
    cat: Catalog,
    source: PathBuf,
    output: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("旅途 📷");
        let output = temp.path().join("精选");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&output).unwrap();
        let cat = Catalog::new(&temp.path().join("app"), &temp.path().join("cache")).unwrap();
        Self {
            _temp: temp,
            cat,
            source,
            output,
        }
    }
    fn picture(&self, name: &str, seed: u32) {
        let path = self.source.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let img = image::RgbImage::from_fn(96, 64, |x, y| {
            image::Rgb([
                ((x + seed) % 256) as u8,
                ((y * 3 + seed) % 256) as u8,
                ((x * y + seed) % 256) as u8,
            ])
        });
        img.save(path).unwrap();
    }
    fn add(&self) -> String {
        self.cat.authorize(&self.source, "source").unwrap();
        let r=self.cat.call("add_source",json!({"selectedPath":self.source,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":["**/excluded/**"],"followSymlinks":false}})).unwrap();
        wait(&self.cat, r["jobId"].as_str().unwrap());
        wait_all(&self.cat);
        r["sourceId"].as_str().unwrap().into()
    }
    fn assets(&self) -> Vec<Value> {
        self.cat.call("query_assets",json!({"filter":{},"sort":{"field":"filename","direction":"asc"},"cursor":null,"pageSize":500})).unwrap()["items"].as_array().unwrap().clone()
    }
    fn review(&self, ids: &[String], patch: Value, key: &str) -> Value {
        self.cat.call("apply_review_batch",json!({"target":{"kind":"explicit","assetIds":ids},"patch":patch,"clientMutationId":key})).unwrap()
    }
}
#[test]
fn snapshot_page_membership_and_exclusions_survive_new_assets_and_changed_filters() {
    let f = Fixture::new();
    f.picture("a.png", 1);
    f.picture("b.png", 2);
    let source = f.add();
    let before = f.assets();
    let sort = json!({"field":"filename","direction":"asc"});
    let frozen = f.cat.call("create_selection_snapshot", json!({"filter":{},"sort":sort})).unwrap();
    f.picture("new.png", 3);
    let scan = f.cat.call("rescan_source", json!({"sourceId":source})).unwrap();
    wait(&f.cat, scan["jobId"].as_str().unwrap());
    wait_all(&f.cat);
    let query = |filter: Value, cursor: Value| f.cat.call("query_assets", json!({
        "filter":filter,"sort":sort,"cursor":cursor,"pageSize":2,"selectionSnapshotId":frozen["id"]
    })).unwrap();
    let first = query(json!({}), Value::Null);
    assert_eq!(first["total"], 3);
    assert_eq!(first["selectionMembership"]["snapshotId"], frozen["id"]);
    let mut expected: Vec<Value> = before.iter().map(|a| a["id"].clone()).collect();
    expected.sort_by(|a,b| a.as_str().cmp(&b.as_str()));
    assert_eq!(first["selectionMembership"]["assetIds"], json!(expected));
    let second = query(json!({}), first["nextCursor"].clone());
    assert_eq!(second["items"][0]["filename"], "new.png");
    assert_eq!(second["selectionMembership"]["assetIds"], json!([]));
    let filtered = query(json!({"keyword":"b.png"}), Value::Null);
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["selectionMembership"]["assetIds"], json!([before[1]["id"]]));
    let changed = f.cat.call("apply_review_batch", json!({
        "target":{"kind":"snapshot","snapshotId":frozen["id"],"excludeAssetIds":[before[0]["id"]]},
        "patch":{"pickState":"keep"},"clientMutationId":"snapshot-exclusion-after-new-asset"
    })).unwrap();
    assert_eq!(changed["affectedCount"], 1);
    let after = f.assets();
    assert_eq!(after[0]["review"], before[0]["review"]);
    assert_eq!(after[1]["review"]["pickState"], "keep");
    assert_eq!(after[2]["review"]["pickState"], "unreviewed");
    let undone = f.cat.call("undo_review_batch", json!({"changeBatchId":changed["changeBatchId"]})).unwrap();
    assert_eq!(undone["restoredCount"], 1);
    assert_eq!(query(json!({}), Value::Null)["selectionMembership"]["assetIds"], json!(expected));
    let c = f.cat.connection().unwrap();
    assert_eq!(c.query_row("SELECT count(*) FROM selection_snapshot_items WHERE snapshot_id=?", [frozen["id"].as_str().unwrap()], |r| r.get::<_, i64>(0)).unwrap(), 2);
    c.execute("UPDATE selection_snapshots SET expires_at=0 WHERE id=?", [frozen["id"].as_str().unwrap()]).unwrap();
    let error = f.cat.call("query_assets", json!({"filter":{},"sort":sort,"pageSize":2,"selectionSnapshotId":frozen["id"]})).unwrap_err();
    assert_eq!(error.code, "SNAPSHOT_EXPIRED");
    c.execute("UPDATE selection_snapshots SET expires_at=? WHERE id=?", rusqlite::params![frozen["expiresAt"].as_i64().unwrap(),frozen["id"].as_str().unwrap()]).unwrap();
    c.execute("DELETE FROM media_assets WHERE id=?", [before[1]["id"].as_str().unwrap()]).unwrap();
    let truncated = f.cat.call("query_assets", json!({"filter":{},"sort":sort,"pageSize":2,"selectionSnapshotId":frozen["id"]})).unwrap_err();
    assert_eq!(truncated.code, "SNAPSHOT_EXPIRED");
}

fn wait(cat: &Catalog, jid: &str) -> Value {
    let start = Instant::now();
    loop {
        let j = cat.call("get_job", json!({"jobId":jid})).unwrap();
        if ["completed", "failed", "cancelled"].contains(&j["status"].as_str().unwrap()) {
            return j;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "job timed out: {j}"
        );
        thread::sleep(Duration::from_millis(15));
    }
}
#[test]
fn native_diagnostics_are_authorized_redacted_and_do_not_overwrite_or_change_the_catalog() {
    let f = Fixture::new();
    f.picture("private-location-秘密.jpg", 71);
    f.add();
    let c = f.cat.connection().unwrap();
    c.execute("INSERT INTO jobs(id,type,status,checkpoint_json,error_summary,created_at,updated_at) VALUES('secret-id','REPAIR','failed','{\"path\":\"/private/秘密\"}','/private/error/秘密',1,1)", []).unwrap();
    let before = f.assets();
    let epoch = db::epoch(&c).unwrap();
    assert!(f.cat.call("export_diagnostics",json!({"destinationPath":f.output})).is_err());
    assert!(fs::read_dir(&f.output).unwrap().next().is_none());
    f.cat.authorize(&f.output, "diagnostics").unwrap();
    let sentinel = f.output.join("travel-gallery-diagnostics.json");
    fs::write(&sentinel,b"keep old report").unwrap();
    let first=f.cat.call("export_diagnostics",json!({"destinationPath":f.output})).unwrap();
    let file=first["fileName"].as_str().unwrap();
    let text=fs::read_to_string(f.output.join(file)).unwrap();
    let report:Value=serde_json::from_str(&text).unwrap();
    assert_eq!(report["sourceCount"],1);
    assert_eq!(report["assetCount"],1);
    assert_eq!(report["catalogVersion"],db::CATALOG_VERSION);
    assert_eq!(report["offlineMode"],true);
    assert_eq!(report.as_object().unwrap().len(),9);
    for job in report["jobs"].as_array().unwrap() {
        assert_eq!(job.as_object().unwrap().len(),5);
    }
    assert!(report["jobs"].as_array().unwrap().iter().any(|j|j["type"]=="REPAIR"&&j["status"]=="failed"));
    for secret in ["private-","秘密","secret-id",f.source.to_str().unwrap(),f.output.to_str().unwrap()] {
        assert!(!text.contains(secret),"leaked {secret}");
    }
    let second=f.cat.call("export_diagnostics",json!({"destinationPath":f.output})).unwrap();
    assert_ne!(first["fileName"],second["fileName"]);
    assert_eq!(fs::read(&sentinel).unwrap(),b"keep old report");
    assert_eq!(fs::read_to_string(f.output.join(file)).unwrap(),text);
    assert_eq!(fs::read_dir(&f.output).unwrap().count(),3);
    f.cat.authorize(&f.source,"diagnostics").unwrap();
    let rejected=f.cat.call("export_diagnostics",json!({"destinationPath":f.source})).unwrap_err();
    assert_eq!(rejected.code,"EXPORT_TARGET_UNSAFE");
    assert_eq!(f.assets(),before);
    assert_eq!(db::epoch(&c).unwrap(),epoch);
}
#[test]
fn unchanged_source_status_reads_do_not_wait_for_a_background_writer() {
    let f = Fixture::new();
    f.picture("readable.jpg", 11);
    f.add();
    let c = f.cat.connection().unwrap();
    for online in [true,false] {
        if !online {
            c.execute("UPDATE source_roots SET absolute_path=?",[f._temp.path().join("offline").to_string_lossy()]).unwrap();
            f.cat.call("list_sources",json!({})).unwrap(); // persist the actual transition once
        }
        let before = db::epoch(&c).unwrap();
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        let reader = f.cat.clone();
        let (send,receive) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || send.send(reader.call("list_sources",json!({}))).unwrap());
        let result = receive.recv_timeout(Duration::from_millis(500));
        c.execute_batch("ROLLBACK").unwrap();
        handle.join().unwrap();
        let sources = result.expect("An unchanged source list must be a WAL read, without acquiring a writer lock").unwrap();
        assert_eq!(sources[0]["isOnline"],online);
        assert_eq!(sources[0]["totalAssets"],1);
        assert_eq!(sources[0]["offlineAssets"],if online {0} else {1});
        assert_eq!(db::epoch(&c).unwrap(),before);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn unreadable_source_requires_reauthorization_and_preserves_its_catalog() {
    use std::os::unix::fs::PermissionsExt;

    struct RestorePermissions<'a>(&'a Path, fs::Permissions);
    impl Drop for RestorePermissions<'_> {
        fn drop(&mut self) {
            let _ = fs::set_permissions(self.0, self.1.clone());
        }
    }
    let f = Fixture::new();
    f.picture("photo.png", 11);
    let sid = f.add();
    let aid = f.assets()[0]["id"].as_str().unwrap().to_owned();
    f.review(
        std::slice::from_ref(&aid),
        json!({"pickState":"keep","rating":4}),
        "retain-while-unreadable",
    );
    let photo = f.source.join("photo.png");
    let before = f.assets()[0]["review"].clone();
    let original_bytes = fs::read(&photo).unwrap();
    let c = f.cat.connection().unwrap();
    let scan_count = || {
        c.query_row("SELECT count(*) FROM jobs WHERE type='SCAN'", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
    };
    let initially_scanned = scan_count();
    {
        let _restore_root =
            RestorePermissions(&f.source, fs::metadata(&f.source).unwrap().permissions());
        let _restore_photo =
            RestorePermissions(&photo, fs::metadata(&photo).unwrap().permissions());
        fs::set_permissions(&photo, fs::Permissions::from_mode(0)).unwrap();
        fs::set_permissions(&f.source, fs::Permissions::from_mode(0o100)).unwrap();
        assert!(
            f.source.is_dir(),
            "metadata must stay visible for this regression"
        );
        assert_eq!(
            fs::read_dir(&f.source).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        f.cat.startup_scan().unwrap();
        assert_eq!(scan_count(), initially_scanned);
        let status = f.cat.call("list_sources", json!({})).unwrap();
        assert_eq!(status[0]["isOnline"], false);
        assert_eq!(status[0]["requiresAuthorization"], true);
        assert_eq!(status[0]["offlineAssets"], 1);
        assert_eq!(f.assets()[0]["fileState"], "offline");
        let cached = f
            .cat
            .call("resolve_preview", json!({"assetId":aid,"variant":"small"}))
            .unwrap();
        assert_eq!(cached["ready"], true);
        let error = f
            .cat
            .call("resolve_original", json!({"assetId":aid}))
            .unwrap_err();
        assert_eq!(error.code, "PERMISSION_DENIED");
        let task = f
            .cat
            .call("rescan_source", json!({"sourceId":sid}))
            .unwrap();
        let failed = wait(&f.cat, task["jobId"].as_str().unwrap());
        assert_eq!(failed["status"], "failed");
        assert!(failed["errorSummary"]
            .as_str()
            .unwrap()
            .contains("重新授权"));
        assert_eq!(f.assets()[0]["review"], before);
    }
    f.cat.authorize(&f.source, "relink").unwrap();
    let preview = f
        .cat
        .call(
            "preview_relink_source",
            json!({"sourceId":sid,"selectedNewPath":f.source}),
        )
        .unwrap();
    assert_eq!(preview["matched"], 1);
    let preview_epoch = db::epoch(&c).unwrap();
    f.cat.call("list_sources", json!({})).unwrap();
    assert!(db::epoch(&c).unwrap() > preview_epoch);
    f.cat
        .call(
            "commit_relink_source",
            json!({"sourceId":sid,"verificationToken":preview["verificationToken"]}),
        )
        .unwrap();
    let status = f.cat.call("list_sources", json!({})).unwrap();
    assert_eq!(status[0]["isOnline"], true);
    assert_eq!(status[0]["requiresAuthorization"], false);
    let asset = &f.assets()[0];
    assert_eq!(asset["id"], aid);
    assert_eq!(asset["review"], before);
    assert_eq!(asset["fileState"], "available");
    assert!(f
        .cat
        .call("resolve_original", json!({"assetId":aid}))
        .is_ok());
    assert_eq!(fs::read(&photo).unwrap(), original_bytes);
}

#[test]
fn media_location_uses_the_catalog_id_and_stays_inside_the_source_even_for_corrupt_files() {
    let f = Fixture::new();
    f.picture("目录 📷/broken.jpg", 17);
    f.add();
    let asset = f.assets()[0]["id"].as_str().unwrap().to_owned();
    let path = f.source.join("目录 📷/broken.jpg");
    fs::write(&path, b"broken jpeg, still a file").unwrap();
    let c = f.cat.connection().unwrap();
    let before = db::epoch(&c).unwrap();
    let human = f.cat.call("get_asset", json!({"assetId":asset})).unwrap()["review"].clone();
    assert_eq!(files::source_directory(&c, &asset).unwrap(), path.parent().unwrap().canonicalize().unwrap());
    assert_eq!(files::source_directory(&c, "no-such-id").unwrap_err().code, "ASSET_NOT_FOUND");
    fs::remove_file(&path).unwrap();
    assert_eq!(files::source_directory(&c, &asset).unwrap_err().code, "ASSET_NOT_FOUND");
    #[cfg(unix)]
    {
        let outside = f.output.join("outside.jpg");
        fs::write(&outside, b"outside authorized source").unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert_eq!(files::source_directory(&c, &asset).unwrap_err().code, "PERMISSION_DENIED");
        fs::remove_file(&path).unwrap();
    }
    fs::write(&path, b"broken jpeg, still a file").unwrap();
    let moved = f._temp.path().join("offline");
    fs::rename(&f.source, &moved).unwrap();
    assert_eq!(files::source_directory(&c, &asset).unwrap_err().code, "SOURCE_OFFLINE");
    fs::rename(&moved, &f.source).unwrap();
    assert_eq!(db::epoch(&c).unwrap(), before);
    assert_eq!(f.cat.call("get_asset", json!({"assetId":asset})).unwrap()["review"], human);
}
#[test]
fn quota_reclaims_obsolete_files_before_useful_previews_and_preserves_sources() {
    let f = Fixture::new();
    f.picture("safe.jpg", 17);
    f.add();
    let asset = f.assets()[0]["id"].as_str().unwrap().to_string();
    f.review(&[asset.clone()], json!({"pickState":"keep","rating":4,"note":"retain me"}), "quota-human");
    let original = hashes(&f.source);
    let human = f.cat.call("get_asset", json!({"assetId":asset})).unwrap()["review"].clone();
    let cache = f._temp.path().join("cache");
    let stale = cache.join(format!("{}.jpg", "a".repeat(64)));
    let partial = cache.join(format!("{}.part.jpg", id()));
    let fresh = cache.join(format!("{}.jpg", "b".repeat(64)));
    let unknown = cache.join("notes.txt");
    let old_time = std::time::SystemTime::now() - Duration::from_secs(3600);
    let old_file = fs::File::create(&stale).unwrap();
    old_file.set_len(160 * 1024 * 1024).unwrap(); // sparse, no large memory allocation
    old_file.set_times(fs::FileTimes::new().set_modified(old_time)).unwrap();
    fs::write(&partial, b"interrupted jpeg").unwrap();
    fs::File::options().write(true).open(&partial).unwrap().set_times(fs::FileTimes::new().set_modified(old_time)).unwrap();
    fs::write(&fresh, b"publication in progress").unwrap();
    fs::write(&unknown, b"not generated by the cache").unwrap();
    let directory = cache.join(format!("{}.jpg", "c".repeat(64)));
    fs::create_dir(&directory).unwrap();
    #[cfg(unix)]
    let link = {
        let link = cache.join(format!("{}.jpg", "d".repeat(64)));
        std::os::unix::fs::symlink(f.source.join("safe.jpg"), &link).unwrap();
        link
    };
    f.cat.call("save_settings", json!({"cacheLimitGb":0.1})).unwrap();
    let c = f.cat.connection().unwrap();
    let ready: String = c.query_row("SELECT cache_rel_path FROM thumbnail_cache WHERE asset_id=?", [&asset], |r|r.get(0)).unwrap();
    f.cat.trim_cache(&c).unwrap();
    assert!(!stale.exists());
    assert!(!partial.exists());
    assert!(fresh.is_file() && unknown.is_file() && directory.is_dir());
    #[cfg(unix)]
    assert!(link.is_symlink());
    assert!(cache.join(ready).is_file());
    assert_eq!(c.query_row("SELECT count(*) FROM thumbnail_cache", [], |r| r.get::<_,i64>(0)).unwrap(),1);
    assert!(f.cat.call("cache_stats", json!({})).unwrap()["bytes"].as_u64().unwrap() < 107_374_182);
    assert_eq!(f.cat.call("get_asset", json!({"assetId":asset})).unwrap()["review"], human);
    assert_eq!(hashes(&f.source), original);
}
#[test]
fn quota_evicts_the_least_recently_used_reference_and_its_access_row() {
    let f = Fixture::new();
    f.picture("older.jpg", 3);
    f.picture("recent.jpg", 4);
    f.add();
    let original = hashes(&f.source);
    let c = f.cat.connection().unwrap();
    let rows = c.prepare("SELECT t.asset_id,t.cache_rel_path FROM thumbnail_cache t JOIN media_assets m ON m.id=t.asset_id ORDER BY m.filename").unwrap()
        .query_map([], |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).unwrap().map(Result::unwrap).collect::<Vec<_>>();
    for (i,(aid,path)) in rows.iter().enumerate() {
        fs::File::options().write(true).open(f._temp.path().join("cache").join(path)).unwrap().set_len(64*1024*1024).unwrap();
        c.execute("INSERT OR REPLACE INTO cache_access VALUES(?,'small',?)",rusqlite::params![aid,i as i64]).unwrap();
    }
    f.cat.call("save_settings",json!({"cacheLimitGb":0.1})).unwrap();
    f.cat.trim_cache(&c).unwrap();
    assert!(!f._temp.path().join("cache").join(&rows[0].1).exists());
    assert!(f._temp.path().join("cache").join(&rows[1].1).is_file());
    assert_eq!(c.query_row("SELECT count(*) FROM thumbnail_cache",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(c.query_row("SELECT count(*) FROM cache_access WHERE asset_id=?",[&rows[0].0],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(hashes(&f.source), original);
}
#[test]
fn legacy_heif_preview_remains_available_when_the_original_is_offline() {
    let f = Fixture::new();
    f.picture("offline.jpg", 6);
    let source = f.add();
    let asset = f.assets()[0]["id"].as_str().unwrap().to_string();
    let c = f.cat.connection().unwrap();
    // A cache-only migration fixture: the original is never read as HEIF.
    c.execute("UPDATE media_assets SET extension='heic' WHERE id=?", [&asset]).unwrap();
    c.execute("UPDATE source_roots SET absolute_path=? WHERE id=?", rusqlite::params![f._temp.path().join("disconnected").to_string_lossy(),source]).unwrap();
    let preview = f.cat.call("resolve_preview", json!({"assetId":asset,"variant":"small"})).unwrap();
    assert_eq!(preview["ready"], true);
    assert!(preview["resourceUrl"].as_str().unwrap().starts_with("media://"));
    assert_eq!(c.query_row("SELECT format_version FROM thumbnail_cache WHERE asset_id=? AND variant='small'", [&asset], |r|r.get::<_,i64>(0)).unwrap(),1);
}
#[test]
fn jpeg_v2_metadata_and_warm_cache_survive_the_heif_orientation_upgrade() {
    let f = Fixture::new();
    f.picture("warm.jpg", 5);
    let source = f.add();
    let asset = f.assets()[0]["id"].as_str().unwrap().to_string();
    let c = f.cat.connection().unwrap();
    c.execute("UPDATE media_assets SET metadata_json=json_set(metadata_json,'$._version',2) WHERE id=?", [&asset]).unwrap();
    let before: (String, i64) = c.query_row("SELECT cache_rel_path,created_at FROM thumbnail_cache WHERE asset_id=? AND variant='small'", [&asset], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    let original = files::sha256(&f.source.join("warm.jpg"), None).unwrap();
    scan(&f.cat, &source);
    wait_all(&f.cat);
    let after: (String, i64) = c.query_row("SELECT cache_rel_path,created_at FROM thumbnail_cache WHERE asset_id=? AND variant='small'", [&asset], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(before, after);
    let detail = f.cat.call("get_asset", json!({"assetId":asset})).unwrap();
    assert_eq!(c.query_row("SELECT json_extract(metadata_json,'$._version') FROM media_assets WHERE id=?", [&asset], |r| r.get::<_, i64>(0)).unwrap(), 2);
    assert_eq!(detail["width"], 96);
    assert_eq!(detail["height"], 64);
    assert_eq!(files::sha256(&f.source.join("warm.jpg"), None).unwrap(), original);
}
#[test]
fn timeline_uses_trusted_wall_dates_unknown_bucket_and_trip_scope() {
    let f = Fixture::new();
    for (i, name) in [
        "known", "wall", "video", "manual", "mtime", "absent", "invalid", "legacy",
    ]
    .iter()
    .enumerate()
    {
        f.picture(&format!("{name}.jpg"), i as u32);
    }
    let source = f.add();
    let before = hashes(&f.source);
    let c = f.cat.connection().unwrap();
    for (name, local, source, tz, utc) in [
        (
            "known",
            Some("2026-06-15 23:30:00"),
            Some("exif"),
            Some(840),
            Some(1781515800000_i64),
        ),
        (
            "wall",
            Some("2026-06-15 00:15:00"),
            Some("exif"),
            None,
            None,
        ),
        (
            "video",
            Some("2026-06-14 23:50:00"),
            Some("quicktime"),
            Some(-480),
            Some(1781509800000),
        ),
        (
            "manual",
            Some("2026-06-16 12:00:00"),
            Some("manual"),
            None,
            None,
        ),
        (
            "mtime",
            Some("2026-06-15 13:00:00"),
            Some("mtime"),
            None,
            Some(1781528400000),
        ),
        ("absent", None, None, None, None),
        (
            "invalid",
            Some("2026-02-30 12:00:00"),
            Some("exif"),
            None,
            None,
        ),
        (
            "legacy",
            Some("2026-06-15 12:00:00"),
            None,
            Some(480),
            Some(1781510400000),
        ),
    ] {
        c.execute("UPDATE media_assets SET capture_local=?,capture_time_source=?,capture_tz_min=?,capture_utc_ms=? WHERE filename=?",rusqlite::params![local,source,tz,utc,format!("{name}.jpg")]).unwrap();
    }
    c.execute(
        "UPDATE media_assets SET media_kind='video' WHERE filename='video.jpg'",
        [],
    )
    .unwrap();
    c.execute("UPDATE asset_reviews SET pick_state='keep' WHERE asset_id IN(SELECT id FROM media_assets WHERE filename IN('known.jpg','mtime.jpg'))",[]).unwrap();
    c.execute(
        "INSERT INTO collections(id,name,kind,created_at,updated_at) VALUES('timeline-trip','Trip','trip',0,0)",
        [],
    )
    .unwrap();
    c.execute("INSERT INTO collection_assets SELECT 'timeline-trip',id,0 FROM media_assets WHERE filename IN('wall.jpg','video.jpg','mtime.jpg','invalid.jpg')",[]).unwrap();
    let all = f.cat.call("timeline_stats", json!({"filter":{}})).unwrap();
    assert_eq!(
        all,
        json!([
            {"date":"2026-06-16","count":1,"reviewed":0,"unknownTimezone":1,"photos":1,"videos":0},
            {"date":"2026-06-15","count":2,"reviewed":1,"unknownTimezone":1,"photos":2,"videos":0},
            {"date":"2026-06-14","count":1,"reviewed":0,"unknownTimezone":0,"photos":0,"videos":1},
            {"date":null,"count":4,"reviewed":1,"unknownTimezone":0,"photos":4,"videos":0}
        ])
    );
    let trip = f
        .cat
        .call(
            "timeline_stats",
            json!({"filter":{"collectionId":"timeline-trip"}}),
        )
        .unwrap();
    assert_eq!(
        trip,
        json!([
            {"date":"2026-06-15","count":1,"reviewed":0,"unknownTimezone":1,"photos":1,"videos":0},
            {"date":"2026-06-14","count":1,"reviewed":0,"unknownTimezone":0,"photos":0,"videos":1},
            {"date":null,"count":2,"reviewed":1,"unknownTimezone":0,"photos":2,"videos":0}
        ])
    );
    let query = |filter| {
        f.cat.call("query_assets", json!({"filter":filter,"sort":{"field":"filename","direction":"asc"},"cursor":null,"pageSize":100})).unwrap()
    };
    assert_eq!(query(json!({"captureDateKnown":true}))["total"], 4);
    assert_eq!(query(json!({"captureDateKnown":false}))["total"], 4);
    let day = query(
        json!({"captureDateKnown":true,"captureLocalFrom":"2026-06-15","captureLocalTo":"2026-06-15"}),
    );
    assert_eq!(
        day["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["filename"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["known.jpg", "wall.jpg"]
    );
    assert_eq!(
        query(json!({"captureDateKnown":false,"collectionId":"timeline-trip"}))["total"],
        2
    );
    assert_eq!(
        f.cat
            .call(
                "timeline_stats",
                json!({"filter":{"captureDateKnown":"false"}})
            )
            .unwrap_err()
            .code,
        "INVALID_FILTER"
    );
    let mtime: String = c
        .query_row(
            "SELECT id FROM media_assets WHERE filename='mtime.jpg'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    f.cat.call("save_capture_time",json!({"assetId":mtime,"captureLocal":"2026-06-17 08:00:00","captureTzMin":480,"expectedRevision":0,"clientMutationId":"timeline-correction"})).unwrap();
    assert_eq!(query(json!({"captureDateKnown":false}))["total"], 3);
    assert_eq!(
        f.cat.call("timeline_stats", json!({"filter":{}})).unwrap()[0]["date"],
        "2026-06-17"
    );
    f.cat.call("save_capture_time",json!({"assetId":mtime,"captureLocal":null,"captureTzMin":null,"expectedRevision":1,"clientMutationId":"timeline-restore"})).unwrap();
    assert_eq!(query(json!({"captureDateKnown":false}))["total"], 4);
    let snapshot = f.cat.call("create_selection_snapshot",json!({"filter":{"captureDateKnown":false},"sort":{"field":"filename","direction":"asc"}})).unwrap();
    assert_eq!(
        f.cat
            .call(
                "timeline_stats",
                json!({"filter":{"selectionSnapshotId":snapshot["id"]}})
            )
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    c.execute(
        "UPDATE selection_snapshots SET expires_at=0 WHERE id=?",
        [snapshot["id"].as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        f.cat
            .call(
                "timeline_stats",
                json!({"filter":{"selectionSnapshotId":snapshot["id"]}})
            )
            .unwrap_err()
            .code,
        "SNAPSHOT_EXPIRED"
    );
    f.cat
        .call(
            "set_source_enabled",
            json!({"sourceId":source,"enabled":false}),
        )
        .unwrap();
    assert_eq!(
        f.cat.call("timeline_stats", json!({"filter":{}})).unwrap(),
        json!([])
    );
    assert_eq!(hashes(&f.source), before);
}

fn wait_all(cat: &Catalog) {
    let start = Instant::now();
    loop {
        let jobs = cat
            .call("list_jobs", json!({"cursor":null,"pageSize":500}))
            .unwrap();
        if jobs["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|j| !["queued", "running", "paused"].contains(&j["status"].as_str().unwrap()))
        {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(30));
        thread::sleep(Duration::from_millis(15));
    }
}
fn hashes(root: &Path) -> Vec<(PathBuf, (u64, i64), String)> {
    let mut rows = vec![];
    for e in walkdir::WalkDir::new(root).follow_links(false) {
        let e = e.unwrap();
        if e.file_type().is_file() {
            rows.push((
                e.path().to_owned(),
                files::stamp(e.path()).unwrap(),
                files::sha256(e.path(), None).unwrap(),
            ));
        }
    }
    rows.sort();
    rows
}
fn scan(cat: &Catalog, sid: &str) -> Value {
    let r = cat.call("rescan_source", json!({"sourceId":sid})).unwrap();
    let job = wait(cat, r["jobId"].as_str().unwrap());
    wait_all(cat);
    job
}
fn all_snapshot(cat: &Catalog) -> String {
    cat.call(
        "create_selection_snapshot",
        json!({"filter":{},"sort":{"field":"filename","direction":"asc"}}),
    )
    .unwrap()["id"]
        .as_str()
        .unwrap()
        .into()
}

#[test]
fn import_incremental_metadata_and_source_safety() {
    let f = Fixture::new();
    f.picture("湖泊.jpg", 1);
    f.picture("sub/IMG_001.jpg", 2);
    f.picture(".hidden.jpg", 3);
    f.picture("excluded/no.jpg", 4);
    fs::write(f.source.join("broken.jpg"), b"not a photo").unwrap();
    fs::write(f.source.join("empty.jpg"), []).unwrap();
    let before = hashes(&f.source);
    let sid = f.add();
    let assets = f.assets();
    assert_eq!(assets.len(), 4);
    let a = assets.iter().find(|a| a["filename"] == "湖泊.jpg").unwrap();
    assert_eq!(a["width"], 96);
    let aid = a["id"].as_str().unwrap().to_string();
    f.review(
        &[aid.clone()],
        json!({"pickState":"keep","rating":5}),
        &id(),
    );
    assert_eq!(scan(&f.cat, &sid)["status"], "completed");
    assert_eq!(f.assets().len(), 4);
    assert_eq!(
        f.cat.call("get_asset", json!({"assetId":aid})).unwrap()["review"]["rating"],
        5
    );
    f.cat.connection().unwrap().execute("UPDATE media_assets SET width=1,metadata_json=json_remove(metadata_json,'$._version') WHERE id=?",[&aid]).unwrap();
    scan(&f.cat, &sid);
    let upgraded = f.cat.call("get_asset", json!({"assetId":aid})).unwrap();
    assert_eq!(upgraded["width"], 96);
    assert_eq!(upgraded["review"]["rating"], 5);
    assert!(upgraded["metadata"].get("_version").is_none());
    assert_eq!(before, hashes(&f.source));
    fs::remove_file(f.source.join("湖泊.jpg")).unwrap();
    scan(&f.cat, &sid);
    let missing = f.cat.call("get_asset", json!({"assetId":aid})).unwrap();
    assert_eq!(missing["fileState"], "missing");
    assert_eq!(missing["review"]["rating"], 5);
    let duplicate=f.cat.call("add_source",json!({"selectedPath":f.source,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}})).unwrap_err();
    assert_eq!(duplicate.code, "SOURCE_EXISTS");
}

#[test]
fn atomic_reviews_idempotency_revisions_undo_and_500_updates() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    f.picture("b.jpg", 2);
    f.add();
    let ids: Vec<String> = f
        .assets()
        .iter()
        .map(|a| a["id"].as_str().unwrap().into())
        .collect();
    let before = hashes(&f.source);
    let first = f.review(&ids, json!({"pickState":"keep","rating":4}), "once");
    assert_eq!(
        first,
        f.review(&ids, json!({"pickState":"keep","rating":4}), "once")
    );
    let conflict=f.cat.call("apply_review_batch",json!({"target":{"kind":"explicit","assetIds":ids},"patch":{"pickState":"reject"},"clientMutationId":id(),"expectedRevisions":{ids[0].clone():0}})).unwrap_err();
    assert_eq!(conflict.code, "DB_CONFLICT");
    assert!(f
        .assets()
        .iter()
        .all(|a| a["review"]["pickState"] == "keep"));
    let second = f.review(&[ids[0].clone()], json!({"rating":5}), &id());
    let u = f
        .cat
        .call(
            "undo_review_batch",
            json!({"changeBatchId":first["changeBatchId"]}),
        )
        .unwrap();
    assert_eq!(u["restoredCount"], 1);
    assert_eq!(u["skippedCount"], 1);
    assert_eq!(
        f.cat.call("get_asset", json!({"assetId":ids[0]})).unwrap()["review"]["rating"],
        5
    );
    f.cat
        .call(
            "undo_review_batch",
            json!({"changeBatchId":second["changeBatchId"]}),
        )
        .unwrap();
    for n in 0..500 {
        f.review(
            &[ids[0].clone()],
            json!({"pickState":if n%2==0{"keep"}else{"reject"},"rating":n%6,"isFavorite":n%2==0}),
            &format!("stress-{n}"),
        );
    }
    let final_asset = f.cat.call("get_asset", json!({"assetId":ids[0]})).unwrap();
    assert_eq!(final_asset["review"]["pickState"], "reject");
    assert_eq!(final_asset["review"]["rating"], 1);
    assert_eq!(hashes(&f.source), before);
    let same=f.cat.call("apply_review_batch",json!({"target":{"kind":"explicit","assetIds":ids},"patch":{"rating":1},"clientMutationId":"once"})).unwrap_err();
    assert_eq!(same.code, "DB_CONFLICT");
}

#[test]
fn neighbor_counters_follow_live_index_growth_filter_membership_and_stable_ties() {
    let f = Fixture::new();
    for name in ["a.jpg", "b.jpg", "c.jpg"] {
        f.picture(name, 0);
    }
    let sid = f.add();
    let original = f.assets();
    let b = original[1].clone();
    let query = |filter: Value, sort: Value| {
        f.cat.call("query_assets", json!({"filter":filter,"sort":sort,"cursor":null,"pageSize":120})).unwrap()
    };
    let neighbors = |page: &Value, anchor: &Value| {
        f.cat.call("get_asset_neighbors", json!({"assetId":anchor["id"],"queryFingerprint":page["queryFingerprint"],"limit":5})).unwrap()
    };
    let sort = json!({"field":"filename","direction":"asc"});
    let first = query(json!({}), sort.clone());
    let n = neighbors(&first, &b);
    assert_eq!(n["total"], 3);
    assert_eq!(n["position"], 2);
    let frozen = f.cat.call("create_selection_snapshot", json!({"filter":{},"sort":sort})).unwrap();

    f.picture("aa.jpg", 0);
    let scan = f.cat.call("rescan_source", json!({"sourceId":sid})).unwrap();
    wait(&f.cat, scan["jobId"].as_str().unwrap());
    wait_all(&f.cat);
    // The entered fingerprint remains valid for neighbor navigation, but its
    // actual membership has grown. Count and rank must describe that same set.
    let n = neighbors(&first, &b);
    assert_eq!(n["total"], 4);
    assert_eq!(n["position"], 3);
    assert_eq!(n["items"].as_array().unwrap().len(), 4);
    assert_eq!(n["items"][2]["id"], b["id"]);

    let fixed = query(json!({"selectionSnapshotId":frozen["id"]}), json!({"field":"filename","direction":"asc"}));
    let n = neighbors(&fixed, &b);
    assert_eq!(n["total"], 3);
    assert_eq!(n["position"], 2);

    let desc = query(json!({}), json!({"field":"filename","direction":"desc"}));
    assert_eq!(neighbors(&desc, &b)["position"], 2);
    let bytes = query(json!({}), json!({"field":"byteSize","direction":"asc"}));
    let items = bytes["items"].as_array().unwrap();
    assert!(items.iter().all(|a| a["byteSize"] == items[0]["byteSize"]));
    let mut expected_ids = items.iter().map(|a| a["id"].as_str().unwrap()).collect::<Vec<_>>();
    expected_ids.sort();
    for item in items {
        let n = neighbors(&bytes, item);
        assert_eq!(n["total"], 4);
        assert_eq!(n["position"], expected_ids.iter().position(|id| *id == item["id"].as_str().unwrap()).unwrap() + 1);
    }

    f.review(&[b["id"].as_str().unwrap().into()], json!({"pickState":"keep"}), &id());
    let remaining = query(json!({"pickStates":["unreviewed"]}), json!({"field":"filename","direction":"asc"}));
    let outside = neighbors(&remaining, &b);
    assert_eq!(outside["total"], 3);
    assert_eq!(outside["position"], Value::Null);
    assert_eq!(outside["anchorMatches"], false);
    assert!(outside["items"].as_array().unwrap().iter().any(|a| a["id"] == b["id"]));
}

#[test]
fn query_keyset_snapshot_and_filter_validation() {
    let f = Fixture::new();
    for n in 0..8 {
        f.picture(&format!("p{n}.jpg"), n);
    }
    f.add();
    let ids: Vec<String> = f
        .assets()
        .iter()
        .map(|a| a["id"].as_str().unwrap().into())
        .collect();
    f.review(&ids[..4], json!({"pickState":"keep","rating":4}), &id());
    let snapshot = all_snapshot(&f.cat);
    let filtered=f.cat.call("query_assets",json!({"filter":{"pickStates":["keep"],"ratingMin":4,"mediaKinds":["image"]},"sort":{"field":"filename","direction":"asc"},"cursor":null,"pageSize":3})).unwrap();
    assert_eq!(filtered["total"], 4);
    let second=f.cat.call("query_assets",json!({"filter":{"pickStates":["keep"],"ratingMin":4,"mediaKinds":["image"]},"sort":{"field":"filename","direction":"asc"},"cursor":filtered["nextCursor"],"pageSize":3})).unwrap();
    assert_eq!(second["items"].as_array().unwrap().len(), 1);
    assert_ne!(filtered["items"][2]["id"], second["items"][0]["id"]);
    let drift=f.cat.call("query_assets",json!({"filter":{},"sort":{"field":"filename","direction":"asc"},"cursor":filtered["nextCursor"],"pageSize":3})).unwrap_err();
    assert_eq!(drift.code, "INVALID_FILTER");
    let res=f.cat.call("apply_review_batch",json!({"target":{"kind":"snapshot","snapshotId":snapshot},"patch":{"isFavorite":true},"clientMutationId":id()})).unwrap();
    assert_eq!(res["affectedCount"], 8);
    for invalid in [
        json!({"ratingMin":6}),
        json!({"pickStates":["garbage"]}),
        json!({"rawSql":"DROP TABLE media_assets"}),
        json!({"minBytes":12,"maxBytes":1}),
    ] {
        assert_eq!(f.cat.call("query_assets",json!({"filter":invalid,"sort":{"field":"filename","direction":"asc"},"cursor":null,"pageSize":3})).unwrap_err().code,"INVALID_FILTER");
    }
    let c = f.cat.connection().unwrap();
    c.execute(
        "UPDATE selection_snapshots SET expires_at=0 WHERE id=?",
        [&snapshot],
    )
    .unwrap();
    drop(c);
    assert_eq!(f.cat.call("apply_review_batch",json!({"target":{"kind":"snapshot","snapshotId":snapshot},"patch":{"rating":2},"clientMutationId":id()})).unwrap_err().code,"SNAPSHOT_EXPIRED");
}

#[test]
fn select_all_rejects_a_membership_change_even_when_the_result_count_is_unchanged() {
    let f = Fixture::new();
    f.picture("first.jpg", 1);
    f.picture("second.jpg", 2);
    f.add();
    let assets = f.assets();
    let first = assets[0]["id"].as_str().unwrap().to_owned();
    let second = assets[1]["id"].as_str().unwrap().to_owned();
    f.review(std::slice::from_ref(&first), json!({"pickState":"keep"}), &id());

    let filter = json!({"pickStates":["keep"]});
    let sort = json!({"field":"filename","direction":"asc"});
    let page = || {
        f.cat.call(
            "query_assets",
            json!({"filter":filter,"sort":sort,"cursor":null,"pageSize":120}),
        ).unwrap()
    };
    let freeze = |epoch: &Value| {
        f.cat.call(
            "create_selection_snapshot",
            json!({"filter":filter,"sort":sort,"expectedCatalogEpoch":epoch}),
        )
    };
    let displayed = page();
    assert_eq!(displayed["items"][0]["id"], first);
    assert!(displayed["nextCursor"].is_null());
    let original = freeze(&displayed["catalogEpoch"]).unwrap();

    f.review(std::slice::from_ref(&first), json!({"pickState":"unreviewed"}), &id());
    f.review(std::slice::from_ref(&second), json!({"pickState":"keep"}), &id());
    let fresh = page();
    assert_eq!(fresh["total"], displayed["total"]);
    assert_eq!(fresh["queryFingerprint"], displayed["queryFingerprint"]);
    assert_ne!(fresh["catalogEpoch"], displayed["catalogEpoch"]);
    assert_eq!(fresh["items"][0]["id"], second);

    let rejected = freeze(&displayed["catalogEpoch"]).unwrap_err();
    assert_eq!(rejected.code, "INVALID_FILTER");
    assert!(rejected.message.contains("刷新"));
    let db = f.cat.connection().unwrap();
    let snapshot_count: i64 = db.query_row("SELECT count(*) FROM selection_snapshots", [], |row| row.get(0)).unwrap();
    assert_eq!(snapshot_count, 1);
    let updated = freeze(&fresh["catalogEpoch"]).unwrap();
    let read_member = |snapshot: &Value| -> String {
        db.query_row(
            "SELECT asset_id FROM selection_snapshot_items WHERE snapshot_id=?",
            [snapshot["id"].as_str().unwrap()],
            |row| row.get(0),
        ).unwrap()
    };
    assert_eq!(read_member(&original), first);
    assert_eq!(read_member(&updated), second);
    assert_eq!(original["count"], 1);
    assert_eq!(updated["count"], 1);
}

#[test]
fn catalog_mutations_roll_back_when_their_epoch_cannot_be_committed() {
    let f = Fixture::new();
    f.picture("first.jpg", 1);
    f.picture("second.jpg", 2);
    let source = f.add();
    let assets = f.assets();
    let first = assets[0]["id"].as_str().unwrap();
    let second = assets[1]["id"].as_str().unwrap();
    let c = f.cat.connection().unwrap();
    // Exercise link validation without requiring a RAW decoder.
    c.execute("UPDATE media_assets SET extension='dng' WHERE id=?", [first]).unwrap();
    let tag = f.cat.call("upsert_tag", json!({"name":"original"})).unwrap()["tagId"].clone();
    let trip = f.cat.call("save_trip", json!({"name":"trip"})).unwrap()["tripId"].clone();
    f.cat.call("add_to_trip", json!({"tripId":trip,"target":{"kind":"explicit","assetIds":[first]}})).unwrap();
    let epoch = db::epoch(&c).unwrap();
    c.execute_batch("CREATE TRIGGER reject_epoch BEFORE UPDATE OF value ON app_meta
        WHEN OLD.key='epoch' BEGIN SELECT RAISE(ABORT,'epoch fault'); END;").unwrap();
    for (command, request) in [
        ("set_source_enabled", json!({"sourceId":source,"enabled":false})),
        ("upsert_tag", json!({"id":tag,"name":"renamed"})),
        ("delete_trip", json!({"tripId":trip})),
        ("link_media", json!({"assetId":first,"secondaryAssetId":second,"linkType":"raw_jpeg"})),
    ] {
        let error = f.cat.call(command, request).unwrap_err();
        assert!(error.details["reason"].as_str().unwrap().contains("epoch fault"), "{command}: {error:?}");
        assert_eq!(db::epoch(&c).unwrap(), epoch);
    }
    assert!(c.query_row("SELECT is_enabled FROM source_roots WHERE id=?", [&source], |r| r.get::<_, bool>(0)).unwrap());
    assert_eq!(c.query_row("SELECT name FROM tags WHERE id=?", [tag.as_str().unwrap()], |r| r.get::<_, String>(0)).unwrap(), "original");
    assert_eq!(c.query_row("SELECT count(*) FROM collection_assets WHERE collection_id=?", [trip.as_str().unwrap()], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(c.query_row("SELECT count(*) FROM linked_media", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    c.execute_batch("DROP TRIGGER reject_epoch").unwrap();
    f.cat.call("link_media", json!({"assetId":first,"secondaryAssetId":second,"linkType":"raw_jpeg"})).unwrap();
    let links = f.cat.call("get_media_links", json!({"assetId":first})).unwrap();
    let epoch = db::epoch(&c).unwrap();
    c.execute_batch("CREATE TRIGGER reject_epoch BEFORE UPDATE OF value ON app_meta
        WHEN OLD.key='epoch' BEGIN SELECT RAISE(ABORT,'epoch fault'); END;").unwrap();
    assert!(f.cat.call("unlink_media", json!({"assetId":first,"linkId":links[0]["id"]})).unwrap_err().details["reason"].as_str().unwrap().contains("epoch fault"));
    assert_eq!(f.cat.call("get_media_links", json!({"assetId":first})).unwrap(), links);
    assert_eq!(db::epoch(&c).unwrap(), epoch);
    c.execute_batch("DROP TRIGGER reject_epoch").unwrap();
    let displayed = f.cat.call("query_assets", json!({"filter":{"collectionId":trip},
        "sort":{"field":"filename","direction":"asc"},"pageSize":120})).unwrap();
    f.cat.call("delete_trip", json!({"tripId":trip})).unwrap();
    assert_eq!(f.cat.call("create_selection_snapshot", json!({"filter":{"collectionId":trip},
        "sort":{"field":"filename","direction":"asc"},"expectedCatalogEpoch":displayed["catalogEpoch"]})).unwrap_err().code, "INVALID_FILTER");
}

#[test]
fn source_polling_rolls_back_both_availability_and_epoch_on_failure() {
    let f = Fixture::new();
    f.picture("first.jpg", 1);
    let source = f.add();
    let c = f.cat.connection().unwrap();
    let epoch = db::epoch(&c).unwrap();
    fs::rename(&f.source, f._temp.path().join("away")).unwrap();
    c.execute_batch("CREATE TRIGGER reject_epoch BEFORE UPDATE OF value ON app_meta
        WHEN OLD.key='epoch' BEGIN SELECT RAISE(ABORT,'epoch fault'); END;").unwrap();
    assert!(f.cat.call("list_sources", json!({})).unwrap_err().details["reason"].as_str().unwrap().contains("epoch fault"));
    assert!(c.query_row("SELECT is_online FROM source_roots WHERE id=?", [&source], |r| r.get::<_, bool>(0)).unwrap());
    assert_eq!(f.assets()[0]["fileState"], "available");
    assert_eq!(db::epoch(&c).unwrap(), epoch);
    c.execute_batch("DROP TRIGGER reject_epoch").unwrap();
    assert_eq!(f.cat.call("list_sources", json!({})).unwrap()[0]["isOnline"], false);
    assert_eq!(f.assets()[0]["fileState"], "offline");
    assert_eq!(db::epoch(&c).unwrap(), epoch + 1);
}

#[test]
fn incremental_scan_rolls_back_rename_and_recovery_if_epoch_write_fails() {
    for rename in [true, false] {
        let f = Fixture::new();
        f.picture("first.jpg", 1);
        let source = f.add();
        let asset = f.assets()[0].clone();
        let c = f.cat.connection().unwrap();
        if rename {
            let hash = files::sha256(&f.source.join("first.jpg"), None).unwrap();
            c.execute("UPDATE media_assets SET sha256=?,sha256_file_version=file_version WHERE id=?",
                rusqlite::params![hash,asset["id"].as_str().unwrap()]).unwrap();
            fs::rename(f.source.join("first.jpg"), f.source.join("renamed.jpg")).unwrap();
        } else {
            c.execute("UPDATE media_assets SET file_state='offline'", []).unwrap();
        }
        let epoch = db::epoch(&c).unwrap();
        let condition = if rename { "filename='renamed.jpg'" } else { "file_state='available'" };
        c.execute_batch(&format!("CREATE TRIGGER reject_epoch BEFORE UPDATE OF value ON app_meta
            WHEN OLD.key='epoch' AND EXISTS(SELECT 1 FROM media_assets WHERE {condition})
            BEGIN SELECT RAISE(ABORT,'epoch fault'); END;")).unwrap();
        let job = scan(&f.cat, &source);
        assert_eq!(job["status"], "failed", "{job}");
        assert_eq!(job["errorSummary"], "资料库操作失败");
        let after = f.assets();
        assert_eq!(after[0]["id"], asset["id"]);
        assert_eq!(after[0]["filename"], "first.jpg");
        assert_eq!(after[0]["fileState"], if rename { "available" } else { "offline" });
        assert_eq!(db::epoch(&c).unwrap(), epoch);
        c.execute_batch("DROP TRIGGER reject_epoch").unwrap();
        assert_eq!(scan(&f.cat, &source)["status"], "completed");
        let recovered = f.assets();
        assert_eq!(recovered[0]["id"], asset["id"]);
        assert_eq!(recovered[0]["filename"], if rename { "renamed.jpg" } else { "first.jpg" });
        assert_eq!(recovered[0]["fileState"], "available");
        assert!(db::epoch(&c).unwrap() > epoch);
    }
}

#[test]
fn preview_dimension_backfill_invalidates_queries_and_rolls_back_on_epoch_failure() {
    let f = Fixture::new();
    f.picture("first.jpg", 1);
    f.add();
    let asset = f.assets()[0].clone();
    let aid = asset["id"].as_str().unwrap();
    let mut c = f.cat.connection().unwrap();
    // Synthetic metadata enters the HEIF backfill branch while the source stays a decodable JPEG.
    c.execute("UPDATE media_assets SET extension='heic',width=NULL,height=NULL WHERE id=?", [aid]).unwrap();
    c.execute("DELETE FROM thumbnail_cache WHERE asset_id=?", [aid]).unwrap();
    let epoch = db::epoch(&c).unwrap();
    c.execute_batch("CREATE TRIGGER reject_epoch BEFORE UPDATE OF value ON app_meta
        WHEN OLD.key='epoch' BEGIN SELECT RAISE(ABORT,'epoch fault'); END;").unwrap();
    assert!(media::thumbnail(&f.cat, &mut c, aid, "small").unwrap_err().details["reason"].as_str().unwrap().contains("epoch fault"));
    assert!(f.assets()[0]["width"].is_null());
    assert_eq!(db::epoch(&c).unwrap(), epoch);
    c.execute_batch("DROP TRIGGER reject_epoch").unwrap();
    media::thumbnail(&f.cat, &mut c, aid, "small").unwrap();
    assert_eq!(f.assets()[0]["width"], 96);
    assert_eq!(f.assets()[0]["height"], 64);
    assert_eq!(db::epoch(&c).unwrap(), epoch + 1);
    media::thumbnail(&f.cat, &mut c, aid, "screen").unwrap();
    assert_eq!(db::epoch(&c).unwrap(), epoch + 1, "A cache-only write must not invalidate queries");
}

#[test]
fn restoring_older_or_newer_epochs_invalidates_every_pre_restore_query() {
    for (newer_backup, legacy_backup) in [(false, false), (true, false), (false, true), (true, true)] {
        let f = Fixture::new();
        f.picture("first.jpg", 1);
        f.picture("second.jpg", 2);
        f.add();
        let assets = f.assets();
        let aid = assets[0]["id"].as_str().unwrap().to_owned();
        f.review(std::slice::from_ref(&aid), json!({"rating":5}), &id());
        let c = f.cat.connection().unwrap();
        let backup = f.output.join("backup.sqlite");
        db::backup(&c, &backup).unwrap();
        f.review(std::slice::from_ref(&aid), json!({"rating":1}), &id());
        let before = f.cat.call("query_assets", json!({"filter":{},
            "sort":{"field":"rating","direction":"asc"},"pageSize":1})).unwrap();
        let previous_epoch = before["catalogEpoch"].as_i64().unwrap();
        let backup_epoch = if newer_backup { previous_epoch + 500 } else { previous_epoch - 1 };
        let copied = db::connect(&backup).unwrap();
        copied.execute("UPDATE app_meta SET value=? WHERE key='epoch'", [backup_epoch]).unwrap();
        if legacy_backup {
            copied.execute_batch(
                "ALTER TABLE media_assets DROP COLUMN file_version;
                 ALTER TABLE media_assets DROP COLUMN sha256_file_version;
                 PRAGMA user_version=9;",
            ).unwrap();
        }
        drop(copied);
        let backup_bytes = fs::read(&backup).unwrap();
        f.cat.authorize(&backup, "restore").unwrap();
        let prepared = f.cat.call("prepare_restore", json!({"selectedBackupPath":backup})).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match f.cat.call("restore_catalog", json!({"selectedBackupPath":backup,"confirmToken":prepared["confirmToken"]})) {
                Ok(_) => break,
                Err(error) if error.code == "DB_CONFLICT" && Instant::now() < deadline => thread::sleep(Duration::from_millis(15)),
                Err(error) => panic!("restore failed: {error:?}"),
            }
        }
        assert_eq!(db::epoch(&c).unwrap(), previous_epoch.max(backup_epoch) + 1);
        assert_eq!(f.cat.call("get_asset", json!({"assetId":aid})).unwrap()["review"]["rating"], 5);
        assert_eq!(f.cat.call("query_assets", json!({"filter":{},
            "sort":{"field":"rating","direction":"asc"},"pageSize":1,"cursor":before["nextCursor"]})).unwrap_err().code, "INVALID_FILTER");
        assert_eq!(f.cat.call("create_selection_snapshot", json!({"filter":{},
            "sort":{"field":"rating","direction":"asc"},"expectedCatalogEpoch":previous_epoch})).unwrap_err().code, "INVALID_FILTER");
        let fresh = f.cat.call("query_assets", json!({"filter":{},
            "sort":{"field":"rating","direction":"asc"},"pageSize":120})).unwrap();
        let snapshot = f.cat.call("create_selection_snapshot", json!({"filter":{},
            "sort":{"field":"rating","direction":"asc"},"expectedCatalogEpoch":fresh["catalogEpoch"]})).unwrap();
        assert_eq!(snapshot["count"], 2);
        assert_eq!(fs::read(&backup).unwrap(), backup_bytes);
        let data = f._temp.path().join("app");
        let files: Vec<_> = fs::read_dir(data).unwrap().map(|e| e.unwrap().path()).collect();
        assert!(!files.iter().any(|p| p.file_name().unwrap().to_string_lossy().starts_with("restore-staging-")));
        let safety: Vec<_> = files.iter().filter(|p| p.extension().is_some_and(|ext| ext == "sqlite") && p.file_name().unwrap().to_string_lossy().starts_with("before-restore-")).collect();
        assert_eq!(safety.len(), 1);
        let saved = db::connect(safety[0]).unwrap();
        assert_eq!(saved.query_row("SELECT rating FROM asset_reviews WHERE asset_id=?", [&aid], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(db::epoch(&saved).unwrap(), previous_epoch);
    }
}

#[test]
fn restore_requires_reconfirmation_after_same_stamp_or_wal_changes() {
    for use_wal in [false, true] {
        let f = Fixture::new();
        f.picture("photo.jpg", 1);
        f.add();
        let asset = f.assets()[0]["id"].as_str().unwrap().to_owned();
        let c = f.cat.connection().unwrap();
        let backup = f.output.join("selected.sqlite");
        db::backup(&c, &backup).unwrap();
        let edited = db::connect(&backup).unwrap();
        edited.pragma_update(None, "journal_mode", if use_wal { "WAL" } else { "DELETE" }).unwrap();
        if use_wal {
            edited.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        }
        let original = fs::metadata(&backup).unwrap();
        f.cat.authorize(&backup, "restore").unwrap();
        let prepared = f.cat.call("prepare_restore", json!({"selectedBackupPath":backup})).unwrap();
        edited.execute("UPDATE asset_reviews SET rating=5 WHERE asset_id=?", [&asset]).unwrap();
        if !use_wal {
            fs::File::options().write(true).open(&backup).unwrap()
                .set_times(fs::FileTimes::new().set_modified(original.modified().unwrap())).unwrap();
        }
        let changed = fs::metadata(&backup).unwrap();
        assert_eq!((changed.len(), changed.modified().unwrap()), (original.len(), original.modified().unwrap()));

        let error = f.cat.call("restore_catalog", json!({
            "selectedBackupPath":backup,"confirmToken":prepared["confirmToken"]
        })).unwrap_err();
        assert_eq!(error.code, "DB_CONFLICT");
        assert!(error.message.contains("备份文件在确认前已变化"), "{error:?}");
        assert_eq!(f.cat.call("get_asset", json!({"assetId":asset})).unwrap()["review"]["rating"], 0);
        let data = f._temp.path().join("app");
        assert!(!fs::read_dir(data).unwrap().any(|entry| {
            let name = entry.unwrap().file_name();
            let name = name.to_string_lossy();
            name.starts_with("before-restore-") || name.starts_with("restore-staging-")
        }));

        let prepared = f.cat.call("prepare_restore", json!({"selectedBackupPath":backup})).unwrap();
        f.cat.call("restore_catalog", json!({"selectedBackupPath":backup,
            "confirmToken":prepared["confirmToken"]})).unwrap();
        assert_eq!(f.cat.call("get_asset", json!({"assetId":asset})).unwrap()["review"]["rating"], 5);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn restore_reads_static_legacy_wal_backup_without_directory_write_access() {
    const TEST_FOLDER: &str = "TRAVEL_GALLERY_OWNED_STATIC_WAL_TEST_FOLDER";
    if let Some(folder) = std::env::var_os(TEST_FOLDER) {
        let folder = PathBuf::from(folder);
        let selected = folder.join("readonly/selected.sqlite");
        let before = fs::read(&selected).unwrap();
        let readonly = rusqlite::Connection::open_with_flags(
            &selected,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).unwrap();
        assert!(readonly.query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0)).is_err());
        drop(readonly);
        let catalog = Catalog::new(&folder.join("app"), &folder.join("cache")).unwrap();
        catalog.authorize(&selected, "restore").unwrap();
        let prepared = catalog.call("prepare_restore", json!({"selectedBackupPath":selected})).unwrap();
        catalog.call("restore_catalog", json!({"selectedBackupPath":selected,
            "confirmToken":prepared["confirmToken"]})).unwrap();
        assert_eq!(db::setting(&catalog.connection().unwrap(), "test-portable").unwrap().as_deref(), Some("true"));
        assert_eq!(fs::read(&selected).unwrap(), before);
        assert_eq!(fs::read_dir(selected.parent().unwrap()).unwrap().count(), 1);
        assert!(!fs::read_dir(folder.join("app")).unwrap().any(|entry| {
            entry.unwrap().file_name().to_string_lossy().starts_with("restore-input-")
        }));
        return;
    }
    let owned = tempfile::tempdir().unwrap();
    let root = owned.path().canonicalize().unwrap();
    let restricted = root.join("readonly");
    fs::create_dir(&restricted).unwrap();
    let selected = restricted.join("selected.sqlite");
    db::initialize(&selected).unwrap();
    let backup = db::connect(&selected).unwrap();
    backup.execute("INSERT INTO app_settings(key,value_json) VALUES('test-portable','true')", []).unwrap();
    drop(backup);
    assert_eq!(&fs::read(&selected).unwrap()[18..20], &[2, 2]);
    assert_eq!(fs::read_dir(&restricted).unwrap().count(), 1);
    let profile = format!("(version 1) (allow default) (deny file-write* (subpath {}))", json!(restricted));
    let output = std::process::Command::new("/usr/bin/sandbox-exec")
        .args(["-p", &profile])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "restore_reads_static_legacy_wal_backup_without_directory_write_access", "--nocapture"])
        .env(TEST_FOLDER, &root)
        .output().unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

#[test]
fn overflowing_restore_epoch_is_rejected_before_replacing_the_catalog() {
    let f = Fixture::new();
    f.picture("first.jpg", 1);
    f.add();
    let before = f.assets();
    let c = f.cat.connection().unwrap();
    let epoch = db::epoch(&c).unwrap();
    let backup = f.output.join("overflow.sqlite");
    db::backup(&c, &backup).unwrap();
    let copied = db::connect(&backup).unwrap();
    copied.execute("UPDATE app_meta SET value=? WHERE key='epoch'", [i64::MAX]).unwrap();
    copied.execute_batch(
        "ALTER TABLE media_assets DROP COLUMN file_version;
         ALTER TABLE media_assets DROP COLUMN sha256_file_version;
         PRAGMA user_version=9;",
    ).unwrap();
    drop(copied);
    let backup_bytes = fs::read(&backup).unwrap();
    f.cat.authorize(&backup, "restore").unwrap();
    let prepared = f.cat.call("prepare_restore", json!({"selectedBackupPath":backup})).unwrap();
    let error = f.cat.call("restore_catalog", json!({"selectedBackupPath":backup,
        "confirmToken":prepared["confirmToken"]})).unwrap_err();
    assert_eq!(error.code, "DB_CONFLICT");
    assert!(error.message.contains("版本超出范围"), "{error:?}");
    assert_eq!(f.assets(), before);
    assert_eq!(db::epoch(&c).unwrap(), epoch);
    assert_eq!(fs::read(&backup).unwrap(), backup_bytes);
    let data = f._temp.path().join("app");
    assert!(!fs::read_dir(data).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with("restore-staging-")));
}

#[test]
fn tags_merge_batch_and_unicode_search() {
    let f = Fixture::new();
    f.picture("雪山.jpg", 1);
    f.add();
    let aid = f.assets()[0]["id"].as_str().unwrap().to_string();
    let mut tags = vec![];
    for name in ["风光", "Landscape"] {
        let tag = f
            .cat
            .call("upsert_tag", json!({"name":name,"color":"#66846c"}))
            .unwrap();
        tags.push(tag["tagId"].as_str().unwrap().to_string());
    }
    assert_eq!(
        f.cat
            .call("upsert_tag", json!({"name":" landscape ","color":null}))
            .unwrap_err()
            .code,
        "DB_CONFLICT"
    );
    f.cat.call("apply_tags_batch",json!({"target":{"kind":"explicit","assetIds":[aid]},"addTagIds":tags,"removeTagIds":[],"clientMutationId":id()})).unwrap();
    f.cat
        .call(
            "merge_tags",
            json!({"sourceTagId":tags[0],"targetTagId":tags[1]}),
        )
        .unwrap();
    let list = f.cat.call("list_tags", json!({})).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["count"], 1);
    let results=f.cat.call("query_assets",json!({"filter":{"keyword":"雪"},"sort":{"field":"filename","direction":"asc"},"cursor":null,"pageSize":10})).unwrap();
    assert_eq!(results["total"], 1);
}

#[test]
fn exact_duplicates_use_bytes_and_similar_is_separate() {
    let f = Fixture::new();
    f.picture("a/same.jpg", 1);
    f.picture("b/same.jpg", 2);
    fs::copy(f.source.join("a/same.jpg"), f.source.join("copy.jpg")).unwrap();
    fs::write(f.source.join("bad.jpg"), b"bad").unwrap();
    fs::copy(f.source.join("bad.jpg"), f.source.join("bad-copy.jpg")).unwrap();
    let before = hashes(&f.source);
    f.add();
    let snapshot = all_snapshot(&f.cat);
    let task = f
        .cat
        .call(
            "start_duplicate_scan",
            json!({"kind":"exact","scope":{"kind":"snapshot","snapshotId":snapshot}}),
        )
        .unwrap();
    assert_eq!(
        wait(&f.cat, task["jobId"].as_str().unwrap())["status"],
        "completed"
    );
    let groups = f
        .cat
        .call(
            "query_duplicate_groups",
            json!({"kind":"exact","cursor":null,"pageSize":20}),
        )
        .unwrap();
    assert_eq!(groups["total"], 2);
    assert!(groups["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|g| g["memberCount"] == 2));
    let task=f.cat.call("start_duplicate_scan",json!({"kind":"similar","scope":{"kind":"snapshot","snapshotId":snapshot},"similarity":{"maxHammingDistance":20,"timeWindowSeconds":0,"algorithmVersion":"1"}})).unwrap();
    assert_eq!(
        wait(&f.cat, task["jobId"].as_str().unwrap())["status"],
        "completed"
    );
    let groups = f
        .cat
        .call(
            "query_duplicate_groups",
            json!({"kind":"similar","cursor":null,"pageSize":20}),
        )
        .unwrap();
    assert!(groups["total"].as_i64().unwrap() > 0);
    let group = &groups["items"][0];
    let member = &group["members"][1]["asset"]["id"];
    f.cat
        .call(
            "exclude_group_member",
            json!({"groupId":group["id"],"assetId":member,"excluded":true}),
        )
        .unwrap();
    assert_eq!(hashes(&f.source), before);
}

#[test]
fn similar_candidates_match_exhaustive_grouping_with_time_and_aspect_gates() {
    let f = Fixture::new();
    for index in 0..36 {
        let seed = index % 6;
        let (width, height) = if index % 2 == 0 { (96, 64) } else { (64, 96) };
        image::RgbImage::from_fn(width, height, |x, y| {
            if index % 12 < 4 {
                return image::Rgb([(x * 200 / width) as u8; 3]);
            }
            image::Rgb([
                ((x * 11 + y * 7 + seed * 31) % 256) as u8,
                ((x * y + seed * 47) % 256) as u8,
                ((x * 3 + y * 17 + seed * 13) % 256) as u8,
            ])
        })
        .save(f.source.join(format!("{index:02}.png")))
        .unwrap();
    }
    let before = hashes(&f.source);
    f.add();
    let mut c = f.cat.connection().unwrap();
    let assets = f.assets();
    for (index, asset) in assets.iter().enumerate() {
        c.execute(
            "UPDATE media_assets SET capture_utc_ms=? WHERE id=?",
            rusqlite::params![1_700_000_000_000i64 + (index as i64 / 12) * 61_000
                + (index as i64 % 3) * 1_000, asset["id"].as_str().unwrap()],
        ).unwrap();
    }
    f.review(&[assets[0]["id"].as_str().unwrap().into()], json!({"pickState":"keep","rating":4}), "similar-oracle-review");
    let reviews = |c: &rusqlite::Connection| -> String {
        c.query_row("SELECT json_group_array(json_array(asset_id,pick_state,rating,is_favorite,note,revision)) FROM (SELECT * FROM asset_reviews ORDER BY asset_id)", [], |r| r.get(0)).unwrap()
    };
    let reviews_before = reviews(&c);
    let snapshot = all_snapshot(&f.cat);
    for (radius, window) in [(0, 60i64), (8, 60), (8, 0), (20, 60)] {
        let task = f.cat.call("start_duplicate_scan", json!({"kind":"similar",
            "scope":{"kind":"snapshot","snapshotId":snapshot},
            "similarity":{"maxHammingDistance":radius,"timeWindowSeconds":window,"algorithmVersion":"1"}})).unwrap();
        let job = wait(&f.cat, task["jobId"].as_str().unwrap());
        assert_eq!(job["status"], "completed", "{job}");
        assert_eq!(job["failureCount"], 0);
        let rows = c.prepare("SELECT id,perceptual_hash,capture_utc_ms FROM media_assets ORDER BY capture_utc_ms,id").unwrap()
            .query_map([], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,i64>(2)?))).unwrap()
            .collect::<std::result::Result<Vec<_>,_>>().unwrap();
        let entries: Vec<_> = rows.into_iter().map(|(id, hash, time)| {
            let preview = media::thumbnail(&f.cat, &mut c, &id, "small").unwrap();
            let image = media::decode(&preview, 1).unwrap();
            (id, u64::from_str_radix(&hash, 16).unwrap(), time, image.width() as f64 / image.height() as f64)
        }).collect();
        assert!(entries.iter().enumerate().any(|(i, a)| entries[i+1..].iter().any(|b|
            (a.1 ^ b.1).count_ones() <= radius && (a.3 / b.3 - 1.).abs() >= 0.15)),
            "fixture must exercise aspect exclusions");
        if window != 0 {
            assert!(entries.iter().enumerate().any(|(i, a)| entries[i+1..].iter().any(|b|
                (a.1 ^ b.1).count_ones() <= radius && b.2 - a.2 > window * 1000)),
                "fixture must exercise time exclusions");
        }
        // Independent exhaustive oracle: every established center is examined.
        // This shares the documented time/aspect rules, but no production index.
        let mut centers: Vec<usize> = vec![];
        let mut expected: std::collections::BTreeMap<usize, Vec<(String, u32)>> = Default::default();
        for (index, entry) in entries.iter().enumerate() {
            let nearest = centers.iter().filter_map(|center| {
                let candidate = &entries[*center];
                let distance = (candidate.1 ^ entry.1).count_ones();
                (distance <= radius && (window == 0 || entry.2 - candidate.2 <= window * 1000)
                    && (candidate.3 / entry.3 - 1.).abs() < 0.15).then_some((distance, *center))
            }).min();
            if let Some((distance, center)) = nearest {
                expected.entry(center).or_insert_with(|| vec![(entries[center].0.clone(), 0)])
                    .push((entry.0.clone(), distance));
            } else {
                centers.push(index);
            }
        }
        let mut expected: Vec<_> = expected.into_values().collect();
        for members in &mut expected { members.sort(); }
        expected.sort();
        let mut actual: std::collections::BTreeMap<String, Vec<(String, u32)>> = Default::default();
        let mut statement = c.prepare("SELECT group_id,asset_id,distance FROM similar_group_items JOIN similar_groups ON id=group_id WHERE group_type='similar'").unwrap();
        for row in statement.query_map([], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,u32>(2)?))).unwrap() {
            let (group, asset, distance) = row.unwrap();
            actual.entry(group).or_default().push((asset, distance));
        }
        let mut actual: Vec<_> = actual.into_values().collect();
        for members in &mut actual { members.sort(); }
        actual.sort();
        assert!(!expected.is_empty(), "fixture must exercise nonempty groups");
        assert_eq!(actual, expected, "radius={radius}, window={window}");
        assert_eq!(reviews(&c), reviews_before);
    }
    assert_eq!(hashes(&f.source), before);
}

fn check_group_decisions_survive_unchanged_rebuild(kind: &str) {
    let f = Fixture::new();
    f.picture("a.jpg", 19);
    for name in ["b.jpg", "c.jpg", "d.jpg"] {
        fs::copy(f.source.join("a.jpg"), f.source.join(name)).unwrap();
    }
    let before = hashes(&f.source);
    f.add();
    let assets = f.assets();
    let ids: Vec<_> = assets.iter().map(|a| a["id"].as_str().unwrap()).collect();
    let run = |scope: &[&str]| {
        let job = f.cat.call("start_duplicate_scan", json!({"kind":kind,
            "scope":{"kind":"explicit","assetIds":scope},
            "similarity":{"maxHammingDistance":0,"timeWindowSeconds":0}})).unwrap();
        assert_eq!(wait(&f.cat, job["jobId"].as_str().unwrap())["status"], "completed");
        let result = f.cat.call("query_duplicate_groups", json!({"kind":kind})).unwrap();
        assert_eq!(result["total"], 1);
        result["items"][0].clone()
    };
    let group = run(&ids[..3]);
    for (action, aid) in [("set_group_representative", ids[0]), ("exclude_group_member", ids[1])] {
        f.cat.call(action, json!({"groupId":group["id"],"assetId":aid,"excluded":true})).unwrap();
    }
    let rebuilt = run(&ids[..3]);
    assert_eq!(rebuilt["representativeAssetId"], ids[0], "{kind}: representative lost");
    assert_eq!(rebuilt["memberCount"], 2, "{kind}: manual exclusion lost");
    assert_eq!(rebuilt["totalMembers"], 3);
    assert!(rebuilt["members"].as_array().unwrap().iter().any(|member|
        member["asset"]["id"] == ids[1] && member["excluded"] == true));

    // Restoring a candidate must also survive the next rebuild.
    f.cat.call("exclude_group_member", json!({"groupId":rebuilt["id"],
        "assetId":ids[1],"excluded":false})).unwrap();
    let restored = run(&ids[..3]);
    assert_eq!(restored["memberCount"], 3);
    assert_eq!(restored["representativeAssetId"], ids[0]);

    // A different membership is a new result, not an automatic transfer of a decision.
    f.cat.call("exclude_group_member", json!({"groupId":restored["id"],
        "assetId":ids[1],"excluded":true})).unwrap();
    let expanded = run(&ids);
    assert_eq!(expanded["memberCount"], 4);
    assert!(expanded["representativeAssetId"].is_null());

    // Decisions from an obsolete algorithm version must not cross into version 1.
    f.cat.call("set_group_representative", json!({"groupId":expanded["id"],"assetId":ids[0]})).unwrap();
    f.cat.call("exclude_group_member", json!({"groupId":expanded["id"],"assetId":ids[1],"excluded":true})).unwrap();
    let c = f.cat.connection().unwrap();
    c.execute("UPDATE similar_groups SET algorithm_version='obsolete' WHERE id=?",
        [expanded["id"].as_str().unwrap()]).unwrap();
    let upgraded = run(&ids);
    assert_eq!(upgraded["memberCount"], 4);
    assert!(upgraded["representativeAssetId"].is_null());
    assert_eq!(c.query_row("SELECT count(*) FROM asset_reviews WHERE pick_state<>'unreviewed' OR rating<>0 OR is_favorite<>0 OR note<>'' OR revision<>0", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
    assert_eq!(hashes(&f.source), before);
}

#[test]
fn exact_rebuild_preserves_manual_decisions_for_unchanged_members() {
    check_group_decisions_survive_unchanged_rebuild("exact");
}

#[test]
fn group_actions_reject_stale_groups_and_foreign_members_without_mutation() {
    let f = Fixture::new();
    f.picture("a.jpg", 3);
    fs::copy(f.source.join("a.jpg"), f.source.join("b.jpg")).unwrap();
    f.picture("outside.jpg", 17);
    f.add();
    let assets = f.assets();
    let member = assets[0]["id"].as_str().unwrap();
    let outside = assets[2]["id"].as_str().unwrap();
    let scan = || {
        let task = f.cat.call("start_duplicate_scan", json!({"kind":"exact",
            "scope":{"kind":"explicit","assetIds":[member,assets[1]["id"]]}})).unwrap();
        assert_eq!(wait(&f.cat, task["jobId"].as_str().unwrap())["status"], "completed");
        f.cat.call("query_duplicate_groups", json!({"kind":"exact"})).unwrap()["items"][0]["id"].as_str().unwrap().to_owned()
    };
    let stale = scan();
    let current = scan();
    assert_ne!(stale, current);
    let c = f.cat.connection().unwrap();
    let before_epoch = db::epoch(&c).unwrap();
    for (gid, aid) in [(&stale, member), (&current, outside)] {
        let error = f.cat.call("exclude_group_member", json!({"groupId":gid,"assetId":aid,"excluded":true})).unwrap_err();
        assert_eq!(error.code, "INVALID_SELECTION");
        assert_eq!(db::epoch(&c).unwrap(), before_epoch);
    }
    assert_eq!(c.query_row("SELECT sum(manually_excluded) FROM similar_group_items", [], |r|r.get::<_,i64>(0)).unwrap(), 0);
}

fn exact_groups(cat: &Catalog) -> Value {
    cat.call("query_duplicate_groups", json!({"kind":"exact"}))
        .unwrap()
}

fn build_exact_group(cat: &Catalog, ids: &[String]) -> Value {
    let task = cat
        .call(
            "start_duplicate_scan",
            json!({
                "kind":"exact","scope":{"kind":"explicit","assetIds":ids}
            }),
        )
        .unwrap();
    assert_eq!(
        wait(cat, task["jobId"].as_str().unwrap())["status"],
        "completed"
    );
    let groups = exact_groups(cat);
    assert_eq!(groups["total"], 1);
    assert_eq!(groups["items"][0]["stale"], false);
    groups["items"][0].clone()
}

fn exact_recompute_jobs(cat: &Catalog) -> Vec<Value> {
    cat.call("list_jobs", json!({"pageSize":100})).unwrap()["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|job| job["operation"] == "recomputeExact")
        .cloned()
        .collect()
}

#[test]
fn stale_exact_group_recomputes_unloaded_cross_source_members_with_new_sizes() {
    let f = Fixture::new();
    let other = f._temp.path().join("second source");
    fs::create_dir(&other).unwrap();
    f.picture("00.png", 1);
    for i in 1..15 {
        let source = if i % 2 == 0 { &f.source } else { &other };
        fs::copy(f.source.join("00.png"), source.join(format!("{i:02}.png"))).unwrap();
    }
    let replacement = image::RgbImage::from_pixel(12, 9, image::Rgb([7, 11, 13]));
    replacement.save(f.source.join("outside.png")).unwrap();
    f.add();
    f.cat.authorize(&other, "source").unwrap();
    let added = f
        .cat
        .call(
            "add_source",
            json!({"selectedPath":other,"options":{
                "recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false
            }}),
        )
        .unwrap();
    let other_id = added["sourceId"].as_str().unwrap();
    assert_eq!(
        wait(&f.cat, added["jobId"].as_str().unwrap())["status"],
        "completed"
    );
    wait_all(&f.cat);
    let assets = f.assets();
    let mut ids: Vec<_> = assets
        .iter()
        .filter(|a| a["filename"] != "outside.png")
        .map(|a| a["id"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    let group = build_exact_group(&f.cat, &ids);
    let gid = group["id"].as_str().unwrap();
    assert_eq!(group["members"].as_array().unwrap().len(), 12);
    f.cat
        .call(
            "set_group_representative",
            json!({"groupId":gid,"assetId":ids[0]}),
        )
        .unwrap();
    f.cat
        .call(
            "exclude_group_member",
            json!({"groupId":gid,"assetId":ids[1],"excluded":true}),
        )
        .unwrap();
    f.review(
        &[ids[0].clone()],
        json!({"pickState":"keep","rating":4}),
        "exact-recompute-review",
    );
    let reviewed = f.assets();
    let c = f.cat.connection().unwrap();
    let previous_size = assets[0]["byteSize"].as_u64().unwrap();
    let mut changed = vec![];
    for aid in &ids[13..] {
        let asset = assets
            .iter()
            .find(|a| a["id"].as_str() == Some(aid.as_str()))
            .unwrap();
        let root = if asset["sourceId"] == other_id {
            &other
        } else {
            &f.source
        };
        let path = root.join(asset["relativePath"].as_str().unwrap());
        replacement.save(&path).unwrap();
        assert_ne!(files::stamp(&path).unwrap().0, previous_size);
        changed.push((aid, path));
    }
    let before = (hashes(&f.source), hashes(&other));
    c.execute(
        "INSERT INTO jobs(id,type,status,created_at,updated_at) VALUES('busy','HASH_EXACT','paused',0,0)",
        [],
    ).unwrap();
    let stale = exact_groups(&f.cat);
    assert_eq!(stale["items"][0]["stale"], true);
    assert_eq!(stale["items"][0]["id"], gid);
    assert!(exact_recompute_jobs(&f.cat).is_empty());
    assert_eq!(
        f.cat
            .call(
                "exclude_group_member",
                json!({"groupId":gid,"assetId":ids[0],"excluded":true})
            )
            .unwrap_err()
            .code,
        "DB_CONFLICT"
    );
    c.execute("DELETE FROM jobs WHERE id='busy'", []).unwrap();
    let start = std::sync::Barrier::new(4);
    thread::scope(|scope| {
        let start = &start;
        let cat = &f.cat;
        for _ in 0..3 {
            scope.spawn(move || {
                start.wait();
                exact_groups(cat);
            });
        }
        start.wait();
        exact_groups(cat);
    });
    let jobs = exact_recompute_jobs(&f.cat);
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        wait(&f.cat, jobs[0]["id"].as_str().unwrap())["status"],
        "completed"
    );
    let rebuilt = exact_groups(&f.cat);
    assert_eq!(rebuilt["total"], 2);
    let mut sizes = vec![];
    for group in rebuilt["items"].as_array().unwrap() {
        assert_eq!(group["stale"], false);
        assert!(group["representativeAssetId"].is_null());
        assert_eq!(group["totalMembers"], group["memberCount"]);
        sizes.push(group["totalMembers"].as_i64().unwrap());
    }
    sizes.sort();
    assert_eq!(sizes, [2, 13]);
    for (aid, path) in changed {
        let asset = f
            .assets()
            .into_iter()
            .find(|a| a["id"].as_str() == Some(aid.as_str()))
            .unwrap();
        let (size, mtime) = files::stamp(&path).unwrap();
        assert_eq!(asset["byteSize"], size);
        assert_eq!(asset["mtimeMs"], mtime);
        assert_eq!(asset["width"], 12);
        assert_eq!(asset["height"], 9);
        let stored: (String, u64, i64) = c
            .query_row(
                "SELECT sha256,sha256_byte_size,sha256_mtime_ms FROM media_assets WHERE id=?",
                [aid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(stored, (files::sha256(&path, None).unwrap(), size, mtime));
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM thumbnail_cache WHERE asset_id=?",
                [aid],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    for (old, new) in reviewed.iter().zip(f.assets()) {
        assert_eq!(old["id"], new["id"]);
        assert_eq!(old["review"], new["review"]);
    }
    assert_eq!((hashes(&f.source), hashes(&other)), before);
    assert_eq!(exact_recompute_jobs(&f.cat).len(), 1);
}

#[test]
fn source_rescan_retains_stale_exact_group_until_recompute_preserves_same_members() {
    let f = Fixture::new();
    f.picture("a.png", 1);
    fs::copy(f.source.join("a.png"), f.source.join("b.png")).unwrap();
    let source = f.add();
    let ids: Vec<_> = f
        .assets()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_owned())
        .collect();
    let group = build_exact_group(&f.cat, &ids);
    f.cat
        .call(
            "set_group_representative",
            json!({"groupId":group["id"],"assetId":ids[0]}),
        )
        .unwrap();
    f.cat
        .call(
            "exclude_group_member",
            json!({"groupId":group["id"],"assetId":ids[1],"excluded":true}),
        )
        .unwrap();
    image::RgbImage::from_pixel(3, 5, image::Rgb([9, 5, 3]))
        .save(f.source.join("a.png"))
        .unwrap();
    fs::copy(f.source.join("a.png"), f.source.join("b.png")).unwrap();
    scan(&f.cat, &source);
    wait_all(&f.cat);
    let frozen = f
        .cat
        .call(
            "create_selection_snapshot",
            json!({
                "filter":{"duplicateKind":"exact"},"sort":{"field":"filename","direction":"asc"}
            }),
        )
        .unwrap();
    assert_eq!(frozen["count"], 0);
    assert_eq!(exact_groups(&f.cat)["items"][0]["stale"], true);
    let jobs = exact_recompute_jobs(&f.cat);
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        wait(&f.cat, jobs[0]["id"].as_str().unwrap())["status"],
        "completed"
    );
    let rebuilt = &exact_groups(&f.cat)["items"][0];
    assert_eq!(rebuilt["stale"], false);
    assert_eq!(rebuilt["representativeAssetId"], ids[0]);
    assert_eq!(rebuilt["totalMembers"], 2);
    assert_eq!(rebuilt["memberCount"], 1);
    let count: i64 = f
        .cat
        .connection()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM selection_snapshot_items WHERE snapshot_id=?",
            [frozen["id"].as_str().unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

fn migrate_v9_exact_fixture(f: &Fixture) {
    let c = f.cat.connection().unwrap();
    c.execute_batch(
        "ALTER TABLE media_assets DROP COLUMN file_version;
         ALTER TABLE media_assets DROP COLUMN sha256_file_version;
         PRAGMA user_version=9;",
    )
    .unwrap();
    drop(c);
    db::initialize(&f._temp.path().join("app/catalog.sqlite")).unwrap();
}

#[cfg(unix)]
#[test]
fn exact_group_rebuilds_after_same_size_same_mtime_rewrite_with_or_without_rescan() {
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;

    for (legacy, rescan) in [(false, false), (false, true), (true, true)] {
        let f = Fixture::new();
        f.picture("a.bmp", 1);
        fs::copy(f.source.join("a.bmp"), f.source.join("b.bmp")).unwrap();
        fs::copy(f.source.join("a.bmp"), f.source.join("c.bmp")).unwrap();
        f.picture("outside.bmp", 7);
        let source = f.add();
        let ids: Vec<_> = f
            .assets()
            .iter()
            .map(|a| a["id"].as_str().unwrap().to_owned())
            .collect();
        let group = build_exact_group(&f.cat, &ids);
        assert_eq!(group["totalMembers"], 3);
        f.review(
            &ids[..1],
            json!({"rating":4,"pickState":"keep"}),
            "same-stamp-review",
        );
        let c = f.cat.connection().unwrap();
        let cache_name = || {
            c.query_row(
                "SELECT cache_rel_path FROM thumbnail_cache WHERE asset_id=? AND variant='small'",
                [&ids[0]],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        };
        let old_preview = cache_name();
        if legacy {
            migrate_v9_exact_fixture(&f);
        }
        let original = f.source.join("a.bmp");
        let metadata = fs::metadata(&original).unwrap();
        let original_stamp = files::stamp(&original).unwrap();
        let replacement = fs::read(f.source.join("outside.bmp")).unwrap();
        assert_eq!(original_stamp.0, replacement.len() as u64);
        let mut handle = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&original)
            .unwrap();
        handle.write_all(&replacement).unwrap();
        handle
            .set_times(fs::FileTimes::new().set_modified(metadata.modified().unwrap()))
            .unwrap();
        drop(handle);
        let changed = fs::metadata(&original).unwrap();
        assert_eq!(changed.ino(), metadata.ino());
        assert_eq!(changed.modified().unwrap(), metadata.modified().unwrap());
        assert_eq!(files::stamp(&original).unwrap(), original_stamp);

        if rescan {
            assert_eq!(scan(&f.cat, &source)["status"], "completed");
        }
        let stale = exact_groups(&f.cat);
        assert_eq!(stale["items"][0]["id"], group["id"]);
        assert_eq!(stale["items"][0]["stale"], true);
        let jobs = exact_recompute_jobs(&f.cat);
        assert_eq!(jobs.len(), 1);
        assert_eq!(
            wait(&f.cat, jobs[0]["id"].as_str().unwrap())["status"],
            "completed"
        );
        let groups = exact_groups(&f.cat);
        assert_eq!(groups["total"], 1);
        assert_eq!(groups["items"][0]["stale"], false);
        let mut members: Vec<_> = groups["items"][0]["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|member| member["asset"]["id"].as_str().unwrap().to_owned())
            .collect();
        members.sort();
        let mut expected = ids[1..3].to_vec();
        expected.sort();
        assert_eq!(members, expected);
        let review = f.cat.call("get_asset", json!({"assetId":ids[0]})).unwrap();
        assert_eq!(review["review"]["rating"], 4);
        assert_eq!(review["review"]["pickState"], "keep");
        let preview = f
            .cat
            .call(
                "resolve_preview",
                json!({"assetId":ids[0],"variant":"small"}),
            )
            .unwrap();
        assert_eq!(preview["ready"], true);
        assert_ne!(cache_name(), old_preview);

        let task = f
            .cat
            .call(
                "start_duplicate_scan",
                json!({"kind":"exact",
            "scope":{"kind":"explicit","assetIds":ids}}),
            )
            .unwrap();
        assert_eq!(
            wait(&f.cat, task["jobId"].as_str().unwrap())["status"],
            "completed"
        );
        assert_eq!(exact_groups(&f.cat)["total"], 2);
        let (hash, indexed, hashed): (String, String, String) = c
            .query_row(
                "SELECT sha256,file_version,sha256_file_version FROM media_assets WHERE id=?",
                [&ids[0]],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            hash,
            files::sha256(&f.source.join("outside.bmp"), None).unwrap()
        );
        assert_eq!(indexed, hashed);
    }
}

#[test]
fn v9_exact_group_revalidates_legacy_hashes_and_keeps_manual_decisions() {
    let f = Fixture::new();
    f.picture("a.png", 1);
    fs::copy(f.source.join("a.png"), f.source.join("b.png")).unwrap();
    f.add();
    let ids: Vec<_> = f
        .assets()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_owned())
        .collect();
    let group = build_exact_group(&f.cat, &ids);
    f.cat
        .call(
            "set_group_representative",
            json!({"groupId":group["id"],"assetId":ids[0]}),
        )
        .unwrap();
    f.cat
        .call(
            "exclude_group_member",
            json!({"groupId":group["id"],"assetId":ids[1],"excluded":true}),
        )
        .unwrap();
    f.review(&ids[..1], json!({"rating":4}), "legacy-exact-review");
    let c = f.cat.connection().unwrap();
    let cached_paths = || {
        let mut statement = c
            .prepare("SELECT cache_rel_path FROM thumbnail_cache ORDER BY asset_id,variant")
            .unwrap();
        statement.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let original_cache = cached_paths();
    assert_eq!(original_cache.len(), 2);
    migrate_v9_exact_fixture(&f);
    let unstamped: i64 = c.query_row(
        "SELECT count(*) FROM media_assets WHERE sha256 IS NOT NULL AND sha256_file_version IS NULL",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(unstamped, 2);
    assert_eq!(exact_groups(&f.cat)["items"][0]["stale"], true);
    let jobs = exact_recompute_jobs(&f.cat);
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        wait(&f.cat, jobs[0]["id"].as_str().unwrap())["status"],
        "completed"
    );
    let groups = exact_groups(&f.cat);
    assert_eq!(groups["total"], 1);
    let rebuilt = &groups["items"][0];
    assert_eq!(rebuilt["stale"], false);
    assert_eq!(rebuilt["representativeAssetId"], ids[0]);
    assert_eq!(rebuilt["totalMembers"], 2);
    assert_eq!(rebuilt["memberCount"], 1);
    assert_eq!(
        f.cat.call("get_asset", json!({"assetId":ids[0]})).unwrap()["review"]["rating"],
        4
    );
    let stamped: i64 = c.query_row(
        "SELECT count(*) FROM media_assets WHERE sha256 IS NOT NULL AND sha256_file_version=file_version",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(stamped, 2);
    assert_eq!(cached_paths(), original_cache);
}

#[test]
fn unavailable_exact_group_waits_and_failed_recompute_requires_explicit_retry() {
    let f = Fixture::new();
    f.picture("a.png", 1);
    fs::copy(f.source.join("a.png"), f.source.join("b.png")).unwrap();
    f.add();
    let ids: Vec<_> = f
        .assets()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_owned())
        .collect();
    let group = build_exact_group(&f.cat, &ids);
    let away = f._temp.path().join("offline");
    fs::rename(&f.source, &away).unwrap();
    for _ in 0..2 {
        assert_eq!(exact_groups(&f.cat)["items"][0]["stale"], true);
    }
    assert!(exact_recompute_jobs(&f.cat).is_empty());

    // Seed an interrupted job, then run the actual retry while the root is absent.
    let request = json!({"operation":"recomputeExact","groupIds":[group["id"]]});
    let c = f.cat.connection().unwrap();
    c.execute(
        "INSERT INTO jobs(id,type,status,checkpoint_json,created_at,updated_at) VALUES
         ('interrupted','HASH_EXACT','failed',?,0,0)",
        [request.to_string()],
    )
    .unwrap();
    let retry = || {
        f.cat
            .call(
                "retry_job",
                json!({
                    "jobId":exact_recompute_jobs(&f.cat)[0]["id"]
                }),
            )
            .unwrap()
    };
    let failed = retry();
    assert_eq!(
        wait(&f.cat, failed["newJobId"].as_str().unwrap())["status"],
        "failed"
    );
    assert_eq!(
        f.cat
            .call("get_job_errors", json!({"jobId":failed["newJobId"]}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    fs::rename(away, &f.source).unwrap();
    image::RgbImage::from_pixel(3, 5, image::Rgb([9, 5, 3]))
        .save(f.source.join("a.png"))
        .unwrap();
    let before = hashes(&f.source);
    for _ in 0..2 {
        let stale = exact_groups(&f.cat);
        assert_eq!(stale["items"][0]["id"], group["id"]);
        assert_eq!(stale["items"][0]["stale"], true);
    }
    assert_eq!(exact_recompute_jobs(&f.cat).len(), 2);
    let recovered = retry();
    assert_eq!(
        wait(&f.cat, recovered["newJobId"].as_str().unwrap())["status"],
        "completed"
    );
    assert_eq!(exact_groups(&f.cat)["total"], 0);
    assert_eq!(hashes(&f.source), before);
}

#[test]
fn similar_rebuild_preserves_manual_decisions_for_unchanged_members() {
    check_group_decisions_survive_unchanged_rebuild("similar");
}

#[test]
fn export_preflight_recheck_collision_verification_and_read_only_sources() {
    let f = Fixture::new();
    f.picture("a/same.jpg", 1);
    f.picture("b/same.jpg", 2);
    f.add();
    let before = hashes(&f.source);
    let snap = all_snapshot(&f.cat);
    f.cat.authorize(&f.output, "export").unwrap();
    fs::write(f.output.join("same.jpg"), b"existing").unwrap();
    let request = json!({"target":{"kind":"snapshot","snapshotId":snap},"destinationPath":f.output,"layout":"flat","conflictPolicy":"rename"});
    let p = f.cat.call("preview_export", request.clone()).unwrap();
    assert_eq!(p["fileCount"], 2);
    assert_eq!(p["conflictCount"], 2);
    // A destination created after preview must never be replaced.
    fs::write(f.output.join("same (2).jpg"), b"racing file").unwrap();
    let start = json!({"planToken":p["planToken"],"clientMutationId":"export-once"});
    let j = f.cat.call("start_export", start.clone()).unwrap();
    assert_eq!(j, f.cat.call("start_export", start).unwrap());
    let job = wait(&f.cat, j["jobId"].as_str().unwrap());
    assert_eq!(job["status"], "completed");
    let report = f
        .cat
        .call("get_export_report", json!({"jobId":j["jobId"]}))
        .unwrap();
    assert_eq!(report["copiedCount"], 1);
    assert_eq!(report["verifiedCount"], 1);
    assert_eq!(report["failedCount"], 1);
    assert_eq!(
        fs::read(f.output.join("same (2).jpg")).unwrap(),
        b"racing file"
    );
    assert_eq!(fs::read(f.output.join("same.jpg")).unwrap(), b"existing");
    assert_eq!(hashes(&f.source), before);
    let inside = f.source.join("export");
    fs::create_dir(&inside).unwrap();
    f.cat.authorize(&inside, "export").unwrap();
    let mut unsafe_request = request;
    unsafe_request["destinationPath"] = json!(inside);
    assert_eq!(
        f.cat
            .call("preview_export", unsafe_request)
            .unwrap_err()
            .code,
        "EXPORT_TARGET_UNSAFE"
    );
    assert!(fs::read_dir(&f.output).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("travelgallery-part")));
}

#[cfg(unix)]
#[test]
fn export_freezes_source_versions_and_requires_a_fresh_preview_after_same_stamp_changes() {
    let f = Fixture::new();
    f.picture("a.bmp", 1);
    f.picture("b.bmp", 7);
    let source_id = f.add();
    let assets = f.assets();
    let a = assets[0]["id"].as_str().unwrap();
    let b = assets[1]["id"].as_str().unwrap();
    let c = f.cat.connection().unwrap();
    // A migrated asset can establish its first export version from the file at preview.
    c.execute("UPDATE media_assets SET file_version=NULL WHERE id=?", [b])
        .unwrap();
    f.cat.authorize(&f.output, "export").unwrap();
    let preview = |ids: &[&str]| {
        f.cat
            .call(
                "preview_export",
                json!({
                    "target":{"kind":"explicit","assetIds":ids},"destinationPath":f.output,
                    "layout":"flat","conflictPolicy":"rename","includeXmp":true
                }),
            )
            .unwrap()
    };
    let execute = |plan: &Value, mutation: &str| {
        let job = f
            .cat
            .call(
                "start_export",
                json!({
                    "planToken":plan["planToken"],"clientMutationId":mutation
                }),
            )
            .unwrap();
        assert_eq!(
            wait(&f.cat, job["jobId"].as_str().unwrap())["status"],
            "completed"
        );
        f.cat
            .call("get_export_report", json!({"jobId":job["jobId"]}))
            .unwrap()
    };
    let frozen = preview(&[a, b]);
    let changed = f.source.join("a.bmp");
    let before = fs::metadata(&changed).unwrap();
    let original_stamp = files::stamp(&changed).unwrap();
    let replacement = fs::read(f.source.join("b.bmp")).unwrap();
    assert_eq!(replacement.len() as u64, before.len());
    assert_ne!(replacement, fs::read(&changed).unwrap());
    fs::write(&changed, &replacement).unwrap();
    fs::File::options()
        .write(true)
        .open(&changed)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(before.modified().unwrap()))
        .unwrap();
    assert_eq!(
        fs::metadata(&changed).unwrap().modified().unwrap(),
        before.modified().unwrap()
    );
    assert_eq!(files::stamp(&changed).unwrap(), original_stamp);
    let report = execute(&frozen, "same-stamp-export");
    assert_eq!(report["verifiedCount"], 1);
    assert_eq!(report["failedCount"], 1);
    assert_eq!(report["xmpCopiedCount"], 1);
    assert_eq!(report["xmpSkippedCount"], 1);
    let changed_item = report["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["assetId"] == a)
        .unwrap();
    assert_eq!(changed_item["errorCode"], "DB_CONFLICT");
    assert!(!f.output.join("a.bmp").exists());
    assert!(!f.output.join("a.xmp").exists());
    assert_eq!(fs::read(f.output.join("b.bmp")).unwrap(), replacement);
    assert_eq!(fs::read(&changed).unwrap(), replacement);

    assert_eq!(scan(&f.cat, &source_id)["status"], "completed");
    let legacy = preview(&[a]);
    c.execute("UPDATE export_plans SET plan_json=json_remove(plan_json,'$.items[0].sourceVersion') WHERE token=?",
        [legacy["planToken"].as_str().unwrap()]).unwrap();
    let report = execute(&legacy, "legacy-export-plan");
    assert_eq!(report["items"][0]["errorCode"], "DB_CONFLICT");
    assert!(!f.output.join("a.bmp").exists());

    let report = execute(&preview(&[a]), "fresh-version-export");
    assert_eq!(report["verifiedCount"], 1);
    assert_eq!(report["xmpCopiedCount"], 1);
    assert_eq!(fs::read(f.output.join("a.bmp")).unwrap(), replacement);
    assert!(fs::read_dir(&f.output).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("travelgallery-part")));
}

#[test]
fn export_root_replaced_with_a_file_is_rejected_before_creating_a_job() {
    let f = Fixture::new();
    f.picture("original.png", 7);
    f.add();
    let source_before = hashes(&f.source);
    f.cat.authorize(&f.output, "export").unwrap();
    let preview = f.cat.call("preview_export", json!({
        "target":{"kind":"snapshot","snapshotId":all_snapshot(&f.cat)},
        "destinationPath":f.output,"layout":"flat","conflictPolicy":"rename"
    })).unwrap();
    fs::remove_dir(&f.output).unwrap();
    fs::write(&f.output, b"a concurrent file must survive").unwrap();
    let error = f.cat.call("start_export", json!({
        "planToken":preview["planToken"],"clientMutationId":"changed-target-root"
    })).unwrap_err();
    assert_eq!(error.code, "EXPORT_TARGET_UNSAFE");
    assert!(error.message.contains("重新选择"));
    let c = f.cat.connection().unwrap();
    assert_eq!(c.query_row("SELECT count(*) FROM jobs WHERE type='EXPORT'", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
    assert!(c.query_row("SELECT consumed_by_job_id FROM export_plans WHERE token=?", [preview["planToken"].as_str().unwrap()], |r| r.get::<_,Option<String>>(0)).unwrap().is_none());
    assert_eq!(fs::read(&f.output).unwrap(), b"a concurrent file must survive");
    assert_eq!(hashes(&f.source), source_before);
}

#[test]
fn backup_restore_relink_and_offline_preserve_reviews() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    let sid = f.add();
    let aid = f.assets()[0]["id"].as_str().unwrap().to_string();
    f.review(&[aid.clone()], json!({"rating":5}), &id());
    f.cat.authorize(&f.output, "backup").unwrap();
    let job = f
        .cat
        .call("backup_catalog", json!({"destinationPath":f.output}))
        .unwrap();
    assert_eq!(
        wait(&f.cat, job["jobId"].as_str().unwrap())["status"],
        "completed"
    );
    f.review(&[aid.clone()], json!({"rating":1}), &id());
    let backup = fs::read_dir(&f.output)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "sqlite"))
        .unwrap();
    f.cat.authorize(&backup, "restore").unwrap();
    let prepared = f
        .cat
        .call("prepare_restore", json!({"selectedBackupPath":backup}))
        .unwrap();
    f.cat
        .call(
            "restore_catalog",
            json!({"selectedBackupPath":backup,"confirmToken":prepared["confirmToken"]}),
        )
        .unwrap();
    assert_eq!(
        f.cat.call("get_asset", json!({"assetId":aid})).unwrap()["review"]["rating"],
        5
    );
    let moved = f.source.with_file_name("new-root");
    fs::rename(&f.source, &moved).unwrap();
    f.cat.call("list_sources", json!({})).unwrap();
    let a = f.cat.call("get_asset", json!({"assetId":aid})).unwrap();
    assert_eq!(a["fileState"], "offline");
    assert_eq!(a["review"]["rating"], 5);
    f.cat.authorize(&moved, "relink").unwrap();
    let p = f
        .cat
        .call(
            "preview_relink_source",
            json!({"sourceId":sid,"selectedNewPath":moved}),
        )
        .unwrap();
    assert_eq!(p["matched"], 1);
    f.cat
        .call(
            "commit_relink_source",
            json!({"sourceId":sid,"verificationToken":p["verificationToken"]}),
        )
        .unwrap();
    assert_eq!(
        f.cat.call("get_asset", json!({"assetId":aid})).unwrap()["review"]["rating"],
        5
    );
}

#[test]
fn relink_rechecks_same_stamp_candidate_versions_without_cached_hashes() {
    let f = Fixture::new();
    f.picture("photo.jpg", 1);
    let source_id = f.add();
    let asset_id = f.assets()[0]["id"].as_str().unwrap().to_owned();
    f.review(std::slice::from_ref(&asset_id), json!({"rating":5}), &id());
    let c = f.cat.connection().unwrap();
    assert!(c.query_row("SELECT sha256 FROM media_assets WHERE id=?", [&asset_id], |r| r.get::<_, Option<String>>(0)).unwrap().is_none());
    let source = f.source.join("photo.jpg");
    let original = fs::read(&source).unwrap();
    let metadata = fs::metadata(&source).unwrap();
    let moved = f.output.join("new-root");
    fs::create_dir(&moved).unwrap();
    let target = moved.join("photo.jpg");
    fs::copy(&source, &target).unwrap();
    let preserve_time = || fs::File::options().write(true).open(&target).unwrap()
        .set_times(fs::FileTimes::new().set_modified(metadata.modified().unwrap())).unwrap();
    preserve_time();
    f.cat.authorize(&moved, "relink").unwrap();
    let preview = || f.cat.call("preview_relink_source", json!({"sourceId":source_id,"selectedNewPath":moved})).unwrap();
    let prepared = preview();
    assert_eq!(prepared["matched"], 1);
    let mut replaced = original.clone();
    *replaced.last_mut().unwrap() ^= 1;
    fs::write(&target, replaced).unwrap();
    preserve_time();
    assert_eq!(files::stamp(&source).unwrap(), files::stamp(&target).unwrap());
    let error = f.cat.call("commit_relink_source", json!({
        "sourceId":source_id,"verificationToken":prepared["verificationToken"]
    })).unwrap_err();
    assert_eq!(error.code, "DB_CONFLICT");
    assert!(error.message.contains("新目录在预演后发生变化"), "{error:?}");
    assert_eq!(c.query_row("SELECT absolute_path FROM source_roots WHERE id=?", [&source_id], |r| r.get::<_, String>(0)).unwrap(), f.source.to_string_lossy());
    assert_eq!(f.cat.call("get_asset", json!({"assetId":asset_id})).unwrap()["review"]["rating"], 5);
    assert_eq!(fs::read(&source).unwrap(), original);
    fs::write(&target, &original).unwrap();
    preserve_time();
    let prepared = preview();
    f.cat.call("commit_relink_source", json!({"sourceId":source_id,
        "verificationToken":prepared["verificationToken"]})).unwrap();
    assert_eq!(f.cat.call("get_asset", json!({"assetId":asset_id})).unwrap()["review"]["rating"], 5);
}

#[test]
fn schema_upgrade_is_repeatable_and_does_not_erase_marks() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("old.sqlite");
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(include_str!("../migrations/001_baseline.sql"))
            .unwrap();
        c.execute("INSERT INTO source_roots(id,display_name,absolute_path,created_at,updated_at) VALUES('s','s','/test',0,0)",[]).unwrap();
        c.execute("INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,indexed_at,updated_at) VALUES('a','s','a.jpg','a.jpg','jpg','image',1,0,0)",[]).unwrap();
        c.execute("UPDATE asset_reviews SET rating=5 WHERE asset_id='a'", [])
            .unwrap();
    }
    db::initialize(&path).unwrap();
    db::initialize(&path).unwrap();
    let c = db::connect(&path).unwrap();
    assert_eq!(
        c.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        db::CATALOG_VERSION
    );
    assert_eq!(
        c.query_row("SELECT rating FROM asset_reviews", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        5
    );
    assert!(c.execute("UPDATE asset_reviews SET rating=6", []).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_loop_and_export_escape_are_blocked() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    symlink(&f.source, f.source.join("loop")).unwrap();
    f.add();
    assert_eq!(f.assets().len(), 1);
    let outside = f._temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, f.output.join("escape")).unwrap();
    f.cat.authorize(&f.output, "export").unwrap();
    let c = f.cat.connection().unwrap();
    assert!(files::safe_relative(Path::new("../escape/a.jpg")).is_err());
    let dir = cap_std::fs::Dir::open_ambient_dir(&f.output, cap_std::ambient_authority()).unwrap();
    assert!(dir.create("escape/wrong.txt").is_err());
    assert!(!outside.join("wrong.txt").exists());
    drop(c);
}

#[test]
fn invalid_picker_grants_and_source_overlap_are_rejected() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    assert_eq!(f.cat.call("add_source",json!({"selectedPath":f.source,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}})).unwrap_err().code,"PERMISSION_DENIED");
    f.add();
    let child = f.source.join("child");
    fs::create_dir(&child).unwrap();
    f.cat.authorize(&child, "source").unwrap();
    assert_eq!(f.cat.call("add_source",json!({"selectedPath":child,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}})).unwrap_err().code,"SOURCE_OVERLAP");
}

#[test]
fn confirmed_source_overlap_rechecks_the_scope_and_keeps_picker_and_destination_guards() {
    let f = Fixture::new();
    f.picture("outside.jpg", 1);
    f.picture("child_%/a.jpg", 2);
    f.picture("child_AX/b.jpg", 3);
    let parent = f.add();
    let before = f.assets();
    let child = f.source.join("child_%");
    f.cat.authorize(&child, "source").unwrap();
    let request = json!({"selectedPath":child,
        "options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}});
    let warning = f.cat.call("add_source", request.clone()).unwrap_err();
    assert_eq!(warning.code, "SOURCE_OVERLAP");
    assert_eq!(warning.details["sourceCount"], 1);
    assert_eq!(warning.details["indexedCount"], 1);
    assert_eq!(f.cat.call("list_sources", json!({})).unwrap().as_array().unwrap().len(), 1);
    assert_eq!(f.assets(), before);

    let mut confirmed = request;
    confirmed["overlapConfirmation"] = warning.details["overlapConfirmation"].clone();
    f.picture("child_%/new.jpg", 4);
    let scan = f.cat.call("rescan_source", json!({"sourceId":parent})).unwrap();
    wait(&f.cat, scan["jobId"].as_str().unwrap());
    wait_all(&f.cat);
    let newer = f.cat.call("add_source", confirmed.clone()).unwrap_err();
    assert_eq!(newer.code, "SOURCE_OVERLAP");
    assert_eq!(newer.details["indexedCount"], 2);
    assert_ne!(newer.details["overlapConfirmation"], confirmed["overlapConfirmation"]);
    confirmed["overlapConfirmation"] = newer.details["overlapConfirmation"].clone();

    f.cat.authorize(&child, "export").unwrap();
    assert_eq!(f.cat.call("add_source", confirmed.clone()).unwrap_err().code, "PERMISSION_DENIED");
    f.cat.authorize(&child, "source").unwrap();
    let mut changed = confirmed.clone();
    changed["options"]["recursive"] = json!(false);
    assert_eq!(f.cat.call("add_source", changed).unwrap_err().code, "SOURCE_OVERLAP");
    let bytes = fs::read(child.join("a.jpg")).unwrap();
    let results = thread::scope(|scope| {
        let first = scope.spawn(|| f.cat.call("add_source", confirmed.clone()));
        let second = scope.spawn(|| f.cat.call("add_source", confirmed.clone()));
        [first.join().unwrap(), second.join().unwrap()]
    });
    let mut accepted = results.iter().filter_map(|result| result.as_ref().ok());
    let result = accepted.next().unwrap();
    assert!(accepted.next().is_none());
    assert_eq!(
        results.iter().find_map(|result| result.as_ref().err()).unwrap().code,
        "SOURCE_EXISTS"
    );
    wait(&f.cat, result["jobId"].as_str().unwrap());
    wait_all(&f.cat);
    let nested = result["sourceId"].as_str().unwrap();
    assert_ne!(nested, parent);
    let after = f.assets();
    assert_eq!(after.len(), 6);
    for original in before {
        let still_present = after.iter().find(|asset| asset["id"] == original["id"]).unwrap();
        assert_eq!(still_present["review"], original["review"]);
    }
    assert_eq!(fs::read(child.join("a.jpg")).unwrap(), bytes);
    assert_eq!(f.cat.call("add_source", confirmed).unwrap_err().code, "SOURCE_EXISTS");

    f.cat.authorize(&child, "export").unwrap();
    assert_eq!(files::safe_destination(&f.cat, &f.cat.connection().unwrap(), &child, "export")
        .unwrap_err().code, "EXPORT_TARGET_UNSAFE");
    f.cat.authorize(&child, "relink").unwrap();
    let relink = f.cat.call("preview_relink_source", json!({"sourceId":nested,"selectedNewPath":child})).unwrap();
    assert_eq!(relink["matched"], 2);
    assert_eq!(f.cat.call("commit_relink_source",json!({
        "sourceId":nested,"verificationToken":relink["verificationToken"]
    })).unwrap()["updated"], true);
}

#[test]
fn orientation_six_and_unknown_timezone_are_not_guessed() {
    let f = Fixture::new();
    f.picture("oriented.jpg", 1);
    let path = f.source.join("oriented.jpg");
    let original = fs::read(&path).unwrap();
    let mut tiff = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
    tiff.extend(1u16.to_le_bytes());
    tiff.extend(0x0112u16.to_le_bytes());
    tiff.extend(3u16.to_le_bytes());
    tiff.extend(1u32.to_le_bytes());
    tiff.extend(6u16.to_le_bytes());
    tiff.extend(0u16.to_le_bytes());
    tiff.extend(0u32.to_le_bytes());
    let mut exif = b"Exif\0\0".to_vec();
    exif.extend(tiff);
    let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
    jpeg.extend(((exif.len() + 2) as u16).to_be_bytes());
    jpeg.extend(exif);
    jpeg.extend(&original[2..]);
    fs::write(&path, jpeg).unwrap();
    let m = media::metadata(&path, "jpg", "image", 0);
    assert_eq!(m.values["orientation"], 6);
    assert_eq!(m.values["width"], 64);
    assert_eq!(m.values["height"], 96);
    let img = media::decode(&path, 6).unwrap();
    assert_eq!((img.width(), img.height()), (64, 96));
}

#[test]
fn interrupted_job_history_is_recoverable() {
    let f = Fixture::new();
    let c = f.cat.connection().unwrap();
    c.execute("INSERT INTO jobs(id,type,status,created_at,updated_at) VALUES('interrupted','SCAN','running',0,0)",[]).unwrap();
    drop(c);
    db::initialize(
        &f.cat
            .connection()
            .unwrap()
            .path()
            .map(PathBuf::from)
            .unwrap(),
    )
    .unwrap();
    let job = f
        .cat
        .call("get_job", json!({"jobId":"interrupted"}))
        .unwrap();
    assert_eq!(job["status"], "failed");
    assert!(job["errorSummary"].as_str().unwrap().contains("重试"));
}

#[test]
fn cooperative_cancel_does_not_mark_unseen_files_missing() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    f.picture("b.jpg", 2);
    let sid = f.add();
    let c = f.cat.connection().unwrap();
    c.execute("INSERT INTO jobs(id,type,status,checkpoint_json,created_at,updated_at) VALUES('queued-test','SCAN','queued','{}',0,0)",[]).unwrap();
    drop(c);
    f.cat
        .call("cancel_job", json!({"jobId":"queued-test"}))
        .unwrap();
    assert_eq!(
        f.cat
            .call("get_job", json!({"jobId":"queued-test"}))
            .unwrap()["status"],
        "queued"
    );
    assert_eq!(
        travel_gallery::tasks::safe_point(&f.cat, "queued-test")
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert_eq!(
        f.cat
            .call("get_job", json!({"jobId":"queued-test"}))
            .unwrap()["status"],
        "cancelled"
    );
    let j = f
        .cat
        .call("rescan_source", json!({"sourceId":sid}))
        .unwrap();
    f.cat
        .call("cancel_job", json!({"jobId":j["jobId"]}))
        .unwrap();
    wait(&f.cat, j["jobId"].as_str().unwrap());
    wait_all(&f.cat);
    assert!(f.assets().iter().all(|a| a["fileState"] == "available"));
}

#[test]
fn malformed_backup_is_rejected_without_replacing_the_catalog() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    f.add();
    let aid = f.assets()[0]["id"].as_str().unwrap().to_string();
    f.review(&[aid.clone()], json!({"rating":5}), &id());
    let backup = f.output.join("invalid.sqlite");
    let c = rusqlite::Connection::open(&backup).unwrap();
    c.execute_batch("PRAGMA user_version=3;CREATE TABLE media_assets(id TEXT);")
        .unwrap();
    drop(c);
    f.cat.authorize(&backup, "restore").unwrap();
    assert!(f
        .cat
        .call("prepare_restore", json!({"selectedBackupPath":backup}))
        .is_err());
    assert_eq!(
        f.cat.call("get_asset", json!({"assetId":aid})).unwrap()["review"]["rating"],
        5
    );
}

#[test]
fn deleted_source_after_export_preview_is_a_per_file_failure() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    f.add();
    f.cat.authorize(&f.output, "export").unwrap();
    let p=f.cat.call("preview_export",json!({"target":{"kind":"snapshot","snapshotId":all_snapshot(&f.cat)},"destinationPath":f.output,"layout":"flat","conflictPolicy":"rename"})).unwrap();
    fs::rename(f.source.join("a.jpg"), f.source.join("moved.jpg")).unwrap();
    let j = f
        .cat
        .call(
            "start_export",
            json!({"planToken":p["planToken"],"clientMutationId":id()}),
        )
        .unwrap();
    wait(&f.cat, j["jobId"].as_str().unwrap());
    let report = f
        .cat
        .call("get_export_report", json!({"jobId":j["jobId"]}))
        .unwrap();
    assert_eq!(report["copiedCount"], 0);
    assert_eq!(report["failedCount"], 1);
    assert!(!f.output.join("a.jpg").exists());
    assert!(f.source.join("moved.jpg").exists());
}

#[test]
fn saved_filter_rejects_temporary_result_ids_without_overwriting_rules() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    f.add();
    let sort = json!({"field":"filename","direction":"asc"});
    let snapshot = f.cat.call("create_selection_snapshot", json!({"filter":{},"sort":sort})).unwrap();
    assert_eq!(snapshot["count"], 1);
    let rule = json!({"id":"durable","name":"照片规则","filter":{"mediaKinds":["image"]},"sort":sort});
    f.cat.call("save_filter", rule).unwrap();
    let original = f.cat.call("list_saved_filters", json!({})).unwrap();
    for (field, value) in [("selectionSnapshotId", snapshot["id"].clone()), ("aiClusterId", json!("group"))] {
        let error = f.cat.call("save_filter", json!({
            "id":"durable","name":"替换规则","filter":{(field):value},"sort":sort
        })).unwrap_err();
        assert_eq!(error.code, "INVALID_FILTER");
        assert_eq!(error.details["field"], field);
        assert_eq!(f.cat.call("list_saved_filters", json!({})).unwrap(), original);
    }
}

#[test]
fn trips_saved_filters_and_tag_deletion_keep_history_consistent() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    f.add();
    let aid = f.assets()[0]["id"].as_str().unwrap().to_string();
    let trip = f
        .cat
        .call(
            "save_trip",
            json!({"name":"旅途","description":"test","startAt":null,"endAt":null}),
        )
        .unwrap();
    f.cat
        .call(
            "add_to_trip",
            json!({"tripId":trip["tripId"],"target":{"kind":"explicit","assetIds":[aid]}}),
        )
        .unwrap();
    let list = f.cat.call("list_trips", json!({})).unwrap();
    assert_eq!(list[0]["count"], 1);
    assert_eq!(list[0]["coverAssetId"], aid);
    f.cat.call("save_filter",json!({"name":"精选","filter":{"pickStates":["keep"]},"sort":{"field":"filename","direction":"asc"}})).unwrap();
    assert_eq!(
        f.cat.call("list_saved_filters", json!({})).unwrap()[0]["filter"]["pickStates"][0],
        "keep"
    );
    let tag = f
        .cat
        .call("upsert_tag", json!({"name":"a","color":null}))
        .unwrap();
    f.cat.call("apply_tags_batch",json!({"target":{"kind":"explicit","assetIds":[aid]},"addTagIds":[tag["tagId"]],"removeTagIds":[],"clientMutationId":id()})).unwrap();
    let batch = f.review(&[aid.clone()], json!({"rating":5}), &id());
    f.cat
        .call("delete_tag", json!({"tagId":tag["tagId"]}))
        .unwrap();
    let u = f
        .cat
        .call(
            "undo_review_batch",
            json!({"changeBatchId":batch["changeBatchId"]}),
        )
        .unwrap();
    assert_eq!(u["skippedCount"], 1);
    assert_eq!(
        f.cat.call("get_asset", json!({"assetId":aid})).unwrap()["review"]["rating"],
        5
    );
    f.cat
        .call(
            "remove_from_trip",
            json!({"tripId":trip["tripId"],"target":{"kind":"explicit","assetIds":[aid]}}),
        )
        .unwrap();
    let list = f.cat.call("list_trips", json!({})).unwrap();
    assert_eq!(list[0]["count"], 0);
    assert!(list[0]["coverAssetId"].is_null());
}

#[test]
fn second_catalog_instance_cannot_recover_another_instances_jobs() {
    let f = Fixture::new();
    let c = f.cat.connection().unwrap();
    c.execute("INSERT INTO jobs(id,type,status,checkpoint_json,created_at,updated_at) VALUES('live','SCAN','queued','{}',0,0)",[]).unwrap();
    drop(c);
    let result = Catalog::new(&f._temp.path().join("app"), &f._temp.path().join("cache"));
    assert_eq!(result.err().unwrap().code, "DB_CONFLICT");
    assert_eq!(
        f.cat.call("get_job", json!({"jobId":"live"})).unwrap()["status"],
        "queued"
    );
}

#[test]
fn verified_internal_move_preserves_the_asset_identity_and_rating() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    fs::copy(f.source.join("a.jpg"), f.source.join("copy.jpg")).unwrap();
    let sid = f.add();
    let aid = f
        .assets()
        .iter()
        .find(|a| a["filename"] == "a.jpg")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    f.review(&[aid.clone()], json!({"rating":5}), &id());
    let job = f
        .cat
        .call(
            "start_duplicate_scan",
            json!({"kind":"exact","scope":{"kind":"snapshot","snapshotId":all_snapshot(&f.cat)}}),
        )
        .unwrap();
    wait(&f.cat, job["jobId"].as_str().unwrap());
    fs::rename(f.source.join("a.jpg"), f.source.join("renamed.jpg")).unwrap();
    scan(&f.cat, &sid);
    let asset = f.cat.call("get_asset", json!({"assetId":aid})).unwrap();
    assert_eq!(asset["filename"], "renamed.jpg");
    assert_eq!(asset["review"]["rating"], 5);
    assert_eq!(f.assets().len(), 2);
}

#[test]
fn local_calendar_filters_snapshot_scope_and_neighbor_window_are_exact() {
    let f = Fixture::new();
    for n in 0..18 {
        f.picture(&format!("{n:02}.jpg"), n);
    }
    f.add();
    let items = f.assets();
    let c = f.cat.connection().unwrap();
    c.execute("UPDATE media_assets SET capture_utc_ms=NULL,capture_local='2023-07-18 23:59:59',mtime_ms=1735689600000,gps_lat=31.234,gps_lon=121.476",[]).unwrap();
    c.execute("UPDATE media_assets SET capture_local='2024-02-29 00:00:00',gps_lat=-22.501,gps_lon=-43.201 WHERE id=?",[items[0]["id"].as_str().unwrap()]).unwrap();
    let sort = json!({"field":"filename","direction":"asc"});
    let query = |filter: Value| {
        f.cat
            .call(
                "query_assets",
                json!({"filter":filter,"sort":sort,"pageSize":1,"cursor":null}),
            )
            .unwrap()
    };
    assert_eq!(
        query(json!({"captureLocalFrom":"2023-07-18","captureLocalTo":"2023-07-18"}))["total"],
        17
    );
    assert_eq!(
        query(json!({"captureLocalFrom":"2024-02-29","captureLocalTo":"2024-02-29"}))["total"],
        1
    );
    assert_eq!(query(json!({"gpsCell":[31.23,121.48]}))["total"], 17);
    assert_eq!(
        query(json!({"gpsCell":[31.230003,121.479997]}))["total"],
        17,
        "tile geometry is quantized; drilldown must compare the same coordinate cell"
    );
    assert_eq!(query(json!({"gpsCell":[-22.50,-43.20]}))["total"], 1);
    assert_eq!(
        query(json!({"gpsCell":[-22.500003,-43.199997]}))["total"],
        1
    );
    assert_eq!(
        query(json!({"captureFromMs":1735689600000i64,"captureToMs":1735776000000i64}))["total"],
        0,
        "unknown-zone EXIF must not match a UTC interval using mtime"
    );
    assert_eq!(
        query(json!({"captureFromMs":-315619200000i64,"captureToMs":-1}))["total"],
        0,
        "negative Unix timestamps are valid bounds"
    );
    assert!(f.cat.call("query_assets",json!({"filter":{"captureLocalFrom":"2024-02-30"},"sort":sort,"pageSize":1,"cursor":null})).is_err());
    assert!(c
        .execute(
            "UPDATE media_assets SET gps_lat=999 WHERE id=?",
            [items[0]["id"].as_str().unwrap()]
        )
        .is_err());
    c.execute(
        "UPDATE media_assets SET gps_lat=NULL WHERE id=?",
        [items[0]["id"].as_str().unwrap()],
    )
    .unwrap();
    let map = f.cat.call("map_assets", json!({"filter":{}})).unwrap();
    assert_eq!(
        map.as_array()
            .unwrap()
            .iter()
            .map(|p| p["count"].as_i64().unwrap())
            .sum::<i64>(),
        17
    );
    let all = f
        .cat
        .call(
            "create_selection_snapshot",
            json!({"filter":{},"sort":sort}),
        )
        .unwrap();
    let child=f.cat.call("create_selection_snapshot",json!({"filter":{},"sort":sort,"target":{"kind":"snapshot","snapshotId":all["id"],"excludeAssetIds":[items[0]["id"]]}})).unwrap();
    assert_eq!(child["count"], 17);
    assert_eq!(
        query(json!({"selectionSnapshotId":child["id"]}))["total"],
        17
    );
    let page = query(json!({}));
    let neighbors = f
        .cat
        .call(
            "get_asset_neighbors",
            json!({"assetId":items[9]["id"],"queryFingerprint":page["queryFingerprint"],"limit":5}),
        )
        .unwrap();
    assert_eq!(neighbors["items"].as_array().unwrap().len(), 11);
    assert_eq!(neighbors["items"][5]["id"], items[9]["id"]);
    assert_eq!(neighbors["anchorMatches"], true);
    c.execute(
        "UPDATE selection_snapshots SET expires_at=0 WHERE id=?",
        [child["id"].as_str().unwrap()],
    )
    .unwrap();
    assert!(f
        .cat
        .call(
            "create_selection_snapshot",
            json!({"filter":{"selectionSnapshotId":child["id"]},"sort":sort})
        )
        .is_err());
}
#[test]
fn live_photo_identifier_pairs_different_names_and_rejects_ambiguity() {
    let f = Fixture::new();
    f.picture("still.jpg", 1);
    f.picture("other.jpg", 2);
    fs::write(f.source.join("different.mov"), b"unsupported movie fixture").unwrap();
    let sid = f.add();
    let items = f.assets();
    let image = items.iter().find(|a| a["filename"] == "still.jpg").unwrap()["id"]
        .as_str()
        .unwrap();
    let movie = items
        .iter()
        .find(|a| a["filename"] == "different.mov")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let c = f.cat.connection().unwrap();
    let metadata = json!({"_version":media::METADATA_VERSION,"contentIdentifier":"4a1c0798-8b79-4b47-8ad4-a392cb54b8c5"}).to_string();
    c.execute(
        "UPDATE media_assets SET metadata_json=? WHERE id IN (?,?)",
        rusqlite::params![metadata, image, movie],
    )
    .unwrap();
    scan(&f.cat, &sid);
    let links = f
        .cat
        .call("get_media_links", json!({"assetId":image}))
        .unwrap();
    assert_eq!(links[0]["matchMethod"], "apple_content_identifier");
    assert_eq!(links[0]["confirmed"], true);
    c.execute(
        "UPDATE media_assets SET metadata_json=? WHERE filename='other.jpg'",
        [metadata],
    )
    .unwrap();
    scan(&f.cat, &sid);
    assert_eq!(
        f.cat
            .call("get_media_links", json!({"assetId":image}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0
    );
    f.cat
        .call(
            "link_media",
            json!({"assetId":image,"secondaryAssetId":movie,"linkType":"live_photo"}),
        )
        .unwrap();
    scan(&f.cat, &sid);
    assert_eq!(
        f.cat
            .call("get_media_links", json!({"assetId":image}))
            .unwrap()[0]["matchMethod"],
        "manual"
    );
}
#[test]
fn database_backup_cancels_between_pages_and_removes_partial_output() {
    let temp = tempfile::tempdir().unwrap();
    let c = rusqlite::Connection::open_in_memory().unwrap();
    c.execute_batch(
        "CREATE TABLE sample(bytes BLOB); INSERT INTO sample VALUES(zeroblob(16777216));",
    )
    .unwrap();
    let dest = temp.path().join("cancelled.sqlite");
    let mut calls = 0;
    let result = db::backup_controlled(&c, &dest, || {
        calls += 1;
        if calls == 3 {
            Err(travel_gallery::error::AppError::new(
                "CANCELLED",
                "test cancellation",
            ))
        } else {
            Ok(())
        }
    });
    assert_eq!(result.unwrap_err().code, "CANCELLED");
    assert_eq!(calls, 3);
    assert!(!dest.exists());
    assert_eq!(
        c.query_row("SELECT length(bytes) FROM sample", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        16777216
    );
    db::backup(&c, &dest).unwrap();
    assert_eq!(
        rusqlite::Connection::open(dest)
            .unwrap()
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}

#[test]
fn logical_pairs_preserve_filter_matches_and_export_both_only_when_requested() {
    let f = Fixture::new();
    f.picture("still.jpg", 1);
    fs::write(f.source.join("companion.mov"), b"video fixture").unwrap();
    let sid = f.add();
    let items = f.assets();
    let still = items.iter().find(|a| a["filename"] == "still.jpg").unwrap()["id"]
        .as_str()
        .unwrap();
    let movie = items
        .iter()
        .find(|a| a["filename"] == "companion.mov")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let c = f.cat.connection().unwrap();
    c.execute(
        "UPDATE media_assets SET metadata_json=?",
        [json!({"_version":media::METADATA_VERSION,"contentIdentifier":"4a1c0798-8b79-4b47-8ad4-a392cb54b8c5"}).to_string()],
    )
    .unwrap();
    scan(&f.cat, &sid);
    let query = |filter: Value| {
        f.cat.call("query_assets",json!({"filter":filter,"sort":{"field":"filename","direction":"asc"},"cursor":null,"pageSize":10})).unwrap()
    };
    assert_eq!(query(json!({}))["total"], 1);
    assert_eq!(query(json!({}))["items"][0]["linkedMediaCount"], 1);
    assert_eq!(query(json!({"combineLinked":false}))["total"], 2);
    assert_eq!(
        query(json!({"mediaKinds":["video"]}))["items"][0]["id"],
        movie
    );
    assert_eq!(query(json!({"keyword":"companion"}))["total"], 1);
    f.cat.authorize(&f.output, "export").unwrap();
    for (linked, count) in [(false, 1), (true, 2)] {
        let preview=f.cat.call("preview_export",json!({"target":{"kind":"explicit","assetIds":[still]},"destinationPath":f.output,"layout":"flat","conflictPolicy":"rename","includeLinked":linked})).unwrap();
        assert_eq!(preview["fileCount"], count);
    }
    let links = f
        .cat
        .call("get_media_links", json!({"assetId":still}))
        .unwrap();
    f.cat
        .call(
            "unlink_media",
            json!({"assetId":still,"linkId":links[0]["id"]}),
        )
        .unwrap();
    scan(&f.cat, &sid);
    assert_eq!(query(json!({}))["total"], 2);
    assert_eq!(
        f.cat
            .call("get_media_links", json!({"assetId":still}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert!(f
        .cat
        .call(
            "link_media",
            json!({"assetId":movie,"secondaryAssetId":still,"linkType":"live_photo"})
        )
        .is_err());
    assert!(f
        .cat
        .call(
            "link_media",
            json!({"assetId":still,"secondaryAssetId":still,"linkType":"live_photo"})
        )
        .is_err());
}

#[test]
fn live_photo_content_identity_can_cross_sources_but_names_cannot() {
    let f = Fixture::new();
    f.picture("IMG_001.jpg", 1);
    let _first = f.add();
    let image = f.assets()[0]["id"].as_str().unwrap().to_string();
    let another = f._temp.path().join("另一来源");
    fs::create_dir(&another).unwrap();
    fs::write(another.join("IMG_001.mov"), b"movie fixture").unwrap();
    f.cat.authorize(&another, "source").unwrap();
    let source=f.cat.call("add_source",json!({"selectedPath":another,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}})).unwrap();
    wait_all(&f.cat);
    assert!(f
        .cat
        .call("get_media_links", json!({"assetId":image}))
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty());
    let c = f.cat.connection().unwrap();
    c.execute(
        "UPDATE media_assets SET metadata_json=?",
        [json!({"_version":media::METADATA_VERSION,"contentIdentifier":"4a1c0798-8b79-4b47-8ad4-a392cb54b8c5"}).to_string()],
    )
    .unwrap();
    scan(&f.cat, source["sourceId"].as_str().unwrap());
    let links = f
        .cat
        .call("get_media_links", json!({"assetId":image}))
        .unwrap();
    assert_eq!(links.as_array().unwrap().len(), 1);
    assert_eq!(links[0]["confirmed"], true);
}

#[test]
fn similar_hash_records_raw_preview_origin_and_rejects_changed_cached_sources() {
    let f = Fixture::new();
    let seed = f.source.join("reference.jpg");
    image::RgbImage::from_fn(512, 384, |x, y| {
        image::Rgb([(x % 255) as u8, (y % 255) as u8, ((x * y) % 255) as u8])
    })
    .save(&seed)
    .unwrap();
    let mut raw = b"synthetic RAW wrapper, embedded JPEG follows".to_vec();
    raw.extend(fs::read(&seed).unwrap());
    fs::write(f.source.join("reference.nef"), &raw).unwrap();
    f.add();
    let items = f.assets();
    let raw_id = items.iter().find(|a| a["extension"] == "nef").unwrap()["id"]
        .as_str()
        .unwrap();
    let scan_raw = || {
        f.cat.call("start_duplicate_scan",json!({"kind":"similar","scope":{"kind":"explicit","assetIds":[raw_id]},"similarity":{"maxHammingDistance":8,"timeWindowSeconds":0,"algorithmVersion":"1"}})).unwrap()
    };
    let job = scan_raw();
    let result = wait(&f.cat, job["jobId"].as_str().unwrap());
    assert_eq!(result["failureCount"], 0);
    assert_eq!(
        f.cat.call("get_asset", json!({"assetId":raw_id})).unwrap()["metadata"]
            ["perceptualHashSource"],
        "raw_embedded_preview"
    );
    assert_eq!(
        f.cat.call("get_asset", json!({"assetId":raw_id})).unwrap()["preview"]
            ["canOriginalPreview"],
        false
    );
    raw.extend(b"external content change without rescan");
    fs::write(f.source.join("reference.nef"), raw).unwrap();
    let job = scan_raw();
    let result = wait(&f.cat, job["jobId"].as_str().unwrap());
    assert_eq!(result["failureCount"], 1);
    let errors = f
        .cat
        .call("get_job_errors", json!({"jobId":job["jobId"]}))
        .unwrap();
    assert!(!errors.as_array().unwrap().is_empty());
}

#[test]
fn filesystem_notifications_rescan_changes_and_reenabled_sources() {
    fn eventually(mut condition: impl FnMut() -> bool) {
        let start = Instant::now();
        while !condition() {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "filesystem change was not indexed"
            );
            thread::sleep(Duration::from_millis(40));
        }
    }
    let f = Fixture::new();
    f.picture("original.jpg", 8);
    let sid = f.add();
    let original = f.assets()[0]["id"].as_str().unwrap().to_owned();
    f.review(&[original.clone()], json!({"rating":4}), "before-watch");
    f.cat.start_watching();
    f.cat.start_watching(); // Starting twice must not create duplicate watchers.
    eventually(|| f.cat.watch_status().watched_roots == 1);
    f.picture("new/subfolder.jpg", 9);
    eventually(|| f.assets().len() == 2);
    image::RgbImage::new(120, 80)
        .save(f.source.join("original.jpg"))
        .unwrap();
    eventually(|| {
        f.cat
            .call("get_asset", json!({"assetId":original}))
            .unwrap()["width"]
            == 120
    });
    assert_eq!(
        f.cat
            .call("get_asset", json!({"assetId":original}))
            .unwrap()["review"]["rating"],
        4
    );
    fs::remove_file(f.source.join("new/subfolder.jpg")).unwrap();
    eventually(|| {
        f.cat
            .connection()
            .unwrap()
            .query_row(
                "SELECT file_state FROM media_assets WHERE filename='subfolder.jpg'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
            == "missing"
    });
    f.cat
        .call(
            "set_source_enabled",
            json!({"sourceId":sid,"enabled":false}),
        )
        .unwrap();
    eventually(|| f.cat.watch_status().watched_roots == 0);
    f.picture("while-disabled.jpg", 11);
    thread::sleep(Duration::from_millis(1200));
    assert_eq!(
        f.cat
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM media_assets WHERE filename='while-disabled.jpg'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    f.cat
        .call("set_source_enabled", json!({"sourceId":sid,"enabled":true}))
        .unwrap();
    eventually(|| {
        f.cat
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM media_assets WHERE filename='while-disabled.jpg'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    });
    let disconnected = f.source.with_file_name("disconnected");
    fs::rename(&f.source, &disconnected).unwrap();
    eventually(|| {
        f.cat
            .connection()
            .unwrap()
            .query_row(
                "SELECT is_online FROM source_roots WHERE id=?",
                [&sid],
                |r| r.get::<_, bool>(0),
            )
            .unwrap()
            == false
    });
    fs::rename(&disconnected, &f.source).unwrap();
    eventually(|| {
        f.cat
            .connection()
            .unwrap()
            .query_row(
                "SELECT is_online FROM source_roots WHERE id=?",
                [&sid],
                |r| r.get::<_, bool>(0),
            )
            .unwrap()
    });
    assert_eq!(
        f.cat
            .call("get_asset", json!({"assetId":original}))
            .unwrap()["review"]["rating"],
        4
    );
    f.cat.stop_watching();
    assert!(!f.cat.watch_status().active);
    wait_all(&f.cat);
}

#[test]
fn idle_filesystem_watcher_does_not_retain_the_catalog_lock() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let cat = Catalog::new(&data, &cache).unwrap();
    cat.start_watching();
    thread::sleep(Duration::from_millis(50));
    drop(cat);
    let start = Instant::now();
    loop {
        if let Ok(reopened) = Catalog::new(&data, &cache) {
            assert!(!reopened.watch_status().active);
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "watcher retained catalog lock"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn cull_progress_uses_frozen_scope_local_day_and_exact_source_folder() {
    let f = Fixture::new();
    for (name, seed) in [
        ("day/a.jpg", 1),
        ("day/b.jpg", 2),
        ("day/nested/c.jpg", 3),
        ("outside.jpg", 4),
    ] {
        f.picture(name, seed);
    }
    f.add();
    let items = f.assets();
    let aid = |name: &str| {
        items.iter().find(|a| a["filename"] == name).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let a = aid("a.jpg");
    let b = aid("b.jpg");
    let c = aid("c.jpg");
    let outside = aid("outside.jpg");
    let conn = f.cat.connection().unwrap();
    for (id, date) in [
        (&a, "2026-06-15 23:50:00"),
        (&b, "2026-06-16 00:10:00"),
        (&c, "2026-06-15 23:59:00"),
        (&outside, "2026-06-15 01:00:00"),
    ] {
        conn.execute("UPDATE media_assets SET capture_local=?,capture_utc_ms=NULL,capture_tz_min=NULL WHERE id=?",rusqlite::params![date,id]).unwrap();
    }
    f.review(
        &[b.clone(), c.clone(), outside],
        json!({"pickState":"keep"}),
        "progress-marks",
    );
    let snapshot=f.cat.call("create_selection_snapshot",json!({"filter":{},"sort":{"field":"filename","direction":"asc"},"target":{"kind":"explicit","assetIds":[a,b,c]}})).unwrap();
    let request = json!({"assetId":a,"filter":{"selectionSnapshotId":snapshot["id"],"pickStates":["unreviewed"]}});
    let result = f.cat.call("get_cull_progress", request.clone()).unwrap();
    assert_eq!(result["scope"], json!({"total":3,"reviewed":2}));
    assert_eq!(
        result["day"],
        json!({"label":"2026-06-15","total":2,"reviewed":1})
    );
    assert_eq!(
        result["folder"],
        json!({"label":"day","total":2,"reviewed":1})
    );
    conn.execute(
        "UPDATE selection_snapshots SET expires_at=0 WHERE id=?",
        [snapshot["id"].as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        f.cat.call("get_cull_progress", request).unwrap_err().code,
        "SNAPSHOT_EXPIRED"
    );
}

#[test]
fn manual_capture_time_is_versioned_source_safe_and_survives_rescanning() {
    let f = Fixture::new();
    f.picture("clock.jpg", 5);
    let sid = f.add();
    let aid = f.assets()[0]["id"].as_str().unwrap().to_owned();
    let path = f.source.join("clock.jpg");
    let hash = files::sha256(&path, None).unwrap();
    let stamp = files::stamp(&path).unwrap();
    let c = f.cat.connection().unwrap();
    c.execute("UPDATE media_assets SET capture_local='2022-03-04 05:06:07',capture_utc_ms=NULL,capture_tz_min=NULL,capture_time_source='exif' WHERE id=?",[&aid]).unwrap();
    let first = json!({"assetId":aid,"captureLocal":"2026-06-15 23:30:00","captureTzMin":480,"expectedRevision":0,"clientMutationId":"time-1"});
    let result = f.cat.call("save_capture_time", first.clone()).unwrap();
    assert_eq!(result["revision"], 1);
    assert_eq!(
        f.cat.call("save_capture_time", first.clone()).unwrap(),
        result
    );
    let detail = f.cat.call("get_asset", json!({"assetId":aid})).unwrap();
    assert_eq!(detail["captureLocal"], "2026-06-15 23:30:00");
    assert_eq!(
        detail["captureUtcMs"],
        chrono::DateTime::parse_from_rfc3339("2026-06-15T15:30:00Z")
            .unwrap()
            .timestamp_millis()
    );
    assert_eq!(detail["captureTimeSource"], "manual");
    assert_eq!(
        detail["captureTimeCorrection"]["original"]["captureLocal"],
        "2022-03-04 05:06:07"
    );
    assert_eq!(
        detail["captureTimeCorrection"]["original"]["captureTimeSource"],
        "exif"
    );
    assert!(detail["captureTimeCorrection"]["original"]["captureTzMin"].is_null());
    let mut invalid = first.clone();
    invalid["clientMutationId"] = json!("time-invalid");
    invalid["expectedRevision"] = json!(1);
    invalid["captureLocal"] = json!("2026-02-30 12:00:00");
    assert_eq!(
        f.cat.call("save_capture_time", invalid).unwrap_err().code,
        "INVALID_FILTER"
    );
    let second = json!({"assetId":aid,"captureLocal":"2026-06-16 00:15:00","captureTzMin":null,"expectedRevision":1,"clientMutationId":"time-2"});
    f.cat.call("save_capture_time", second).unwrap();
    f.review(&[aid.clone()], json!({"rating":5}), "time-review");
    c.execute(
        "UPDATE media_assets SET metadata_json='{}' WHERE id=?",
        [&aid],
    )
    .unwrap();
    let job = f
        .cat
        .call("rescan_source", json!({"sourceId":sid}))
        .unwrap();
    wait(&f.cat, job["jobId"].as_str().unwrap());
    wait_all(&f.cat);
    let detail = f.cat.call("get_asset", json!({"assetId":aid})).unwrap();
    assert_eq!(detail["captureLocal"], "2026-06-16 00:15:00");
    assert!(detail["captureUtcMs"].is_null());
    assert!(detail["captureTzMin"].is_null());
    assert_eq!(detail["captureTimeCorrection"]["revision"], 2);
    assert_eq!(
        detail["captureTimeCorrection"]["original"]["captureTimeSource"],
        "mtime"
    );
    assert_eq!(detail["review"]["rating"], 5);
    let restore = json!({"assetId":aid,"captureLocal":null,"captureTzMin":null,"expectedRevision":2,"clientMutationId":"time-restore"});
    f.cat.call("save_capture_time", restore).unwrap();
    let restored = f.cat.call("get_asset", json!({"assetId":aid})).unwrap();
    assert_eq!(restored["captureTimeCorrection"]["isCorrected"], false);
    assert_eq!(restored["captureTimeCorrection"]["revision"], 3);
    assert_eq!(restored["captureTimeSource"], "mtime");
    assert_eq!(
        restored["captureLocal"],
        detail["captureTimeCorrection"]["original"]["captureLocal"]
    );
    let mut stale = first;
    stale["clientMutationId"] = json!("time-stale");
    assert_eq!(
        f.cat.call("save_capture_time", stale).unwrap_err().code,
        "DB_CONFLICT"
    );
    assert_eq!(files::sha256(&path, None).unwrap(), hash);
    assert_eq!(files::stamp(&path).unwrap(), stamp);
}

#[test]
fn map_trails_keep_days_timezones_gaps_and_trip_membership_separate() {
    let f = Fixture::new();
    for i in 0..9 {
        f.picture(&format!("{i}.jpg"), i);
    }
    f.add();
    let items = f.assets();
    let c = f.cat.connection().unwrap();
    let times = [
        "2026-06-15 09:00:00",
        "2026-06-15 09:01:00",
        "2026-06-16 09:00:00",
        "2026-06-16 09:10:00",
        "2026-06-16 16:00:00",
        "2026-06-16 16:01:00",
        "2026-06-16 16:02:00",
        "2026-06-16 16:03:00",
        "invalid",
    ];
    for (i, a) in items.iter().enumerate() {
        c.execute("UPDATE media_assets SET capture_local=?,capture_time_source=?,capture_tz_min=?,gps_lon=?,gps_lat=30.04 WHERE id=?",rusqlite::params![times[i],if i==7{"mtime"}else{"exif"},if i==6{None}else{Some(480)},101.0+i as f64*0.01,a["id"].as_str()]).unwrap();
    }
    let result = f.cat.call("map_trails", json!({"filter":{}})).unwrap();
    assert_eq!(result["totalPhotos"], 8);
    assert_eq!(result["includedPhotos"], 7);
    assert_eq!(result["unknownTimezone"], 1);
    assert_eq!(result["skippedInvalidTimes"], 1);
    assert_eq!(result["features"].as_array().unwrap().len(), 3);
    assert_eq!(
        result["features"][0]["geometry"]["coordinates"],
        json!([[101.0, 30.04], [101.01, 30.04]])
    );
    let trip = f
        .cat
        .call("save_trip", json!({"name":"Route subset"}))
        .unwrap();
    f.cat.call("add_to_trip",json!({"tripId":trip["tripId"],"target":{"kind":"explicit","assetIds":[items[0]["id"],items[1]["id"],items[2]["id"]]}})).unwrap();
    let scoped = f
        .cat
        .call(
            "map_trails",
            json!({"filter":{"collectionId":trip["tripId"]}}),
        )
        .unwrap();
    assert_eq!(scoped["totalPhotos"], 3);
    assert_eq!(scoped["features"].as_array().unwrap().len(), 1);
    assert_eq!(scoped["features"][0], result["features"][0]);
}

#[test]
fn map_trails_report_the_five_thousand_point_limit_explicitly() {
    let f = Fixture::new();
    f.picture("anchor.jpg", 1);
    let sid = f.add();
    let mut c = f.cat.connection().unwrap();
    let tx = c.transaction().unwrap();
    for i in 0..5002 {
        let time = chrono::NaiveDate::from_ymd_opt(2026, 6, 15)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            + chrono::Duration::seconds(i);
        tx.execute("INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,capture_local,capture_time_source,capture_tz_min,gps_lon,gps_lat,indexed_at,updated_at) VALUES(?,?,?,?,'jpg','image',0,?,'exif',480,?,30.04,0,0)",rusqlite::params![format!("route-{i:05}"),sid,format!("route-{i:05}.jpg"),format!("route-{i:05}.jpg"),time.format("%Y-%m-%d %H:%M:%S").to_string(),101.0+(i%20) as f64*0.01]).unwrap();
    }
    tx.commit().unwrap();
    let result = f.cat.call("map_trails", json!({"filter":{}})).unwrap();
    assert_eq!(result["totalPhotos"], 5002);
    assert_eq!(result["includedPhotos"], 5000);
    assert_eq!(result["truncated"], true);
    assert_eq!(
        result["features"][0]["geometry"]["coordinates"]
            .as_array()
            .unwrap()
            .len(),
        5000
    );
}

#[test]
fn shooting_groups_are_time_only_partitioned_bounded_and_frozen_before_review() {
    let f = Fixture::new();
    f.picture("anchor.jpg", 1);
    let sid = f.add();
    let before = hashes(&f.source);
    let c = f.cat.connection().unwrap();
    let insert = |name: &str, local: &str, camera: Option<&str>, tz: Option<i64>, source: &str| {
        let filename = name.rsplit('/').next().unwrap();
        c.execute("INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,capture_local,capture_time_source,capture_tz_min,camera_model,indexed_at,updated_at) VALUES(?,?,?,?,'jpg','image',0,?,?,?,?,0,0)",rusqlite::params![name,sid,name,filename,local,source,tz,camera]).unwrap();
    };
    for i in 0..20 {
        insert(
            &format!("sequence-{i:02}.jpg"),
            &format!("2026-06-15 09:00:{i:02}"),
            Some("Camera A"),
            None,
            "exif",
        );
    }
    insert(
        "other-camera.jpg",
        "2026-06-15 09:00:01",
        Some("Camera B"),
        None,
        "exif",
    );
    insert(
        "child/other-folder.jpg",
        "2026-06-15 09:00:01",
        Some("Camera A"),
        None,
        "exif",
    );
    insert(
        "other-zone.jpg",
        "2026-06-15 09:00:01",
        Some("Camera A"),
        Some(480),
        "exif",
    );
    insert("no-camera.jpg", "2026-06-15 09:00:01", None, None, "exif");
    insert(
        "mtime.jpg",
        "2026-06-15 09:00:01",
        Some("Camera A"),
        None,
        "mtime",
    );
    insert(
        "invalid.jpg",
        "2026-02-30 09:00:01",
        Some("Camera A"),
        None,
        "exif",
    );
    insert(
        "midnight-1.jpg",
        "2026-06-15 23:59:59",
        Some("Camera A"),
        None,
        "exif",
    );
    insert(
        "midnight-2.jpg",
        "2026-06-16 00:00:00",
        Some("Camera A"),
        None,
        "exif",
    );
    for i in 0..40 {
        insert(
            &format!("long/{i:02}.jpg"),
            &format!("2026-06-17 09:00:{i:02}"),
            Some("Camera A"),
            None,
            "manual",
        );
    }
    let request = json!({"filter":{},"gapSeconds":2,"pageSize":1});
    let first = f.cat.call("query_burst_groups", request.clone()).unwrap();
    assert_eq!(first["total"], 3);
    assert_eq!(first["skippedInvalidTimes"], 1);
    assert_eq!(first["items"][0]["memberCount"], 9);
    let second = f
        .cat
        .call(
            "query_burst_groups",
            json!({"cursor":first["nextCursor"],"filter":{},"gapSeconds":2,"pageSize":1}),
        )
        .unwrap();
    assert_eq!(second["items"][0]["memberCount"], 31);
    let third = f
        .cat
        .call(
            "query_burst_groups",
            json!({"cursor":second["nextCursor"],"filter":{},"gapSeconds":2,"pageSize":1}),
        )
        .unwrap();
    let group = &third["items"][0];
    assert_eq!(group["memberCount"], 20);
    assert_eq!(group["members"].as_array().unwrap().len(), 8);
    assert_eq!(group["captureTzMin"], Value::Null);
    let frozen = f
        .cat
        .call(
            "create_burst_snapshot",
            json!({"filter":{},"gapSeconds":2,"groupId":group["id"]}),
        )
        .unwrap();
    assert_eq!(frozen["count"], 20);
    c.execute(
        "UPDATE media_assets SET capture_local='2026-06-18 12:00:00' WHERE id='sequence-10.jpg'",
        [],
    )
    .unwrap();
    let members=f.cat.call("query_assets",json!({"filter":{"selectionSnapshotId":frozen["id"]},"sort":{"field":"filename","direction":"asc"},"pageSize":50})).unwrap();
    assert_eq!(
        members["total"], 20,
        "existing snapshot membership is immutable after a time edit"
    );
    assert_eq!(
        f.cat
            .call(
                "create_burst_snapshot",
                json!({"filter":{},"gapSeconds":2,"groupId":group["id"]})
            )
            .unwrap_err()
            .code,
        "DB_CONFLICT"
    );
    assert_eq!(
        f.cat
            .call("query_burst_groups", json!({"filter":{},"gapSeconds":0}))
            .unwrap_err()
            .code,
        "INVALID_FILTER"
    );
    assert_eq!(
        f.cat
            .call(
                "query_burst_groups",
                json!({"filter":{"favorite":true},"gapSeconds":2,"cursor":first["nextCursor"]})
            )
            .unwrap_err()
            .code,
        "DB_CONFLICT"
    );
    let empty = f
        .cat
        .call(
            "query_burst_groups",
            json!({"filter":{"favorite":true},"gapSeconds":2}),
        )
        .unwrap();
    assert_eq!(empty["total"], 0);
    let changed:i64=c.query_row("SELECT count(*) FROM asset_reviews WHERE pick_state<>'unreviewed' OR rating<>0 OR is_favorite<>0",[],|r|r.get(0)).unwrap();
    assert_eq!(changed, 0);
    assert_eq!(before, hashes(&f.source));
}

#[test]
fn shooting_group_candidate_limit_never_publishes_an_incomplete_last_group() {
    let f = Fixture::new();
    f.picture("anchor.jpg", 1);
    let sid = f.add();
    let mut c = f.cat.connection().unwrap();
    let tx = c.transaction().unwrap();
    {
        let mut insert=tx.prepare("INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,capture_local,capture_time_source,camera_model,indexed_at,updated_at) VALUES(?,?,?,?,'jpg','image',0,'2026-06-15 09:00:00','exif','camera',0,0)").unwrap();
        for i in 0..100_002 {
            let name = format!("burst-{i:06}.jpg");
            insert
                .execute(rusqlite::params![name, sid, name, name])
                .unwrap();
        }
    }
    tx.commit().unwrap();
    let result = f
        .cat
        .call("query_burst_groups", json!({"filter":{},"gapSeconds":2}))
        .unwrap();
    assert_eq!(result["candidateCount"], 100_002);
    assert_eq!(result["examinedCount"], 100_000);
    assert_eq!(result["truncated"], true);
    assert_eq!(
        result["total"], 0,
        "the only group runs past the cap and must not be published as complete"
    );
}

#[test]
fn xmp_export_is_opt_in_frozen_unicode_and_source_safe() {
    let f = Fixture::new();
    f.picture("山 <&📷.jpg", 1);
    f.add();
    let a = f.assets()[0].clone();
    let aid = a["id"].as_str().unwrap().to_owned();
    let before = hashes(&f.source);
    f.review(&[aid.clone()],json!({"rating":4,"pickState":"reject","isFavorite":true,"note":"旧备注 <tag> & 🏔️\n第二行"}),"xmp-review");
    let tag = f
        .cat
        .call("upsert_tag", json!({"name":"山<&\"'📷","color":null}))
        .unwrap();
    f.cat.call("apply_tags_batch",json!({"target":{"kind":"explicit","assetIds":[aid]},"addTagIds":[tag["tagId"]],"removeTagIds":[],"clientMutationId":"xmp-tags"})).unwrap();
    f.cat.authorize(&f.output, "export").unwrap();
    let base = json!({"target":{"kind":"explicit","assetIds":[aid]},"destinationPath":f.output,"layout":"flat","conflictPolicy":"rename"});
    let default = f.cat.call("preview_export", base.clone()).unwrap();
    assert_eq!(default["xmpCount"], 0);
    let mut request = base;
    request["includeXmp"] = json!(true);
    let preview = f.cat.call("preview_export", request.clone()).unwrap();
    assert_eq!(preview["xmpCount"], 1);
    assert!(preview["totalBytes"].as_i64().unwrap() > a["byteSize"].as_i64().unwrap());
    f.review(
        &[aid.clone()],
        json!({"rating":1,"note":"预演之后的新备注"}),
        "after-xmp-preview",
    );
    let job = f
        .cat
        .call(
            "start_export",
            json!({"planToken":preview["planToken"],"clientMutationId":"export-xmp-frozen"}),
        )
        .unwrap();
    assert_eq!(
        wait(&f.cat, job["jobId"].as_str().unwrap())["status"],
        "completed"
    );
    let report = f
        .cat
        .call("get_export_report", json!({"jobId":job["jobId"]}))
        .unwrap();
    assert_eq!(report["copiedCount"], 1);
    assert_eq!(report["xmpCopiedCount"], 1);
    assert_eq!(report["xmpFailedCount"], 0);
    let sidecar = f
        .output
        .join(report["sidecars"][0]["destination"].as_str().unwrap());
    let xml = fs::read_to_string(&sidecar).unwrap();
    assert!(xml.contains("<xmp:Rating>4</xmp:Rating>"));
    assert!(xml.contains("<tg:PickState>reject</tg:PickState>"));
    assert!(xml.contains("旧备注 &lt;tag&gt; &amp; 🏔️\n第二行"));
    assert!(xml.contains("山&lt;&amp;&quot;&apos;📷"));
    assert!(!xml.contains("预演之后"));
    assert_eq!(
        files::sha256(&sidecar, None).unwrap(),
        report["sidecars"][0]["sha256"]
    );
    let manifest =
        fs::read_to_string(f.output.join(report["manifestPath"].as_str().unwrap())).unwrap();
    assert!(manifest.contains("\"xmp\""));
    assert!(manifest.contains("\"media\""));
    assert_eq!(before, hashes(&f.source));
    request["includeXmp"] = json!("true");
    assert_eq!(
        f.cat.call("preview_export", request).unwrap_err().code,
        "INVALID_FILTER"
    );
}

#[test]
fn xmp_names_are_reserved_with_media_and_conflicts_never_overwrite() {
    let f = Fixture::new();
    f.picture("same.jpg", 1);
    f.picture("same.png", 2);
    f.add();
    let before = hashes(&f.source);
    let snapshot = all_snapshot(&f.cat);
    f.cat.authorize(&f.output, "export").unwrap();
    fs::write(f.output.join("same.xmp"), b"existing metadata").unwrap();
    let mut request = json!({"target":{"kind":"snapshot","snapshotId":snapshot},"destinationPath":f.output,"layout":"flat","conflictPolicy":"skip","includeXmp":true});
    let preview = f.cat.call("preview_export", request.clone()).unwrap();
    assert_eq!(preview["conflictCount"], 2);
    let job = f
        .cat
        .call(
            "start_export",
            json!({"planToken":preview["planToken"],"clientMutationId":"skip-pairs"}),
        )
        .unwrap();
    wait(&f.cat, job["jobId"].as_str().unwrap());
    let report = f
        .cat
        .call("get_export_report", json!({"jobId":job["jobId"]}))
        .unwrap();
    assert_eq!(report["copiedCount"], 0);
    assert_eq!(report["skippedCount"], 2);
    assert_eq!(report["xmpSkippedCount"], 2);
    request["conflictPolicy"] = json!("rename");
    let preview = f.cat.call("preview_export", request).unwrap();
    let job = f
        .cat
        .call(
            "start_export",
            json!({"planToken":preview["planToken"],"clientMutationId":"rename-pairs"}),
        )
        .unwrap();
    wait(&f.cat, job["jobId"].as_str().unwrap());
    let report = f
        .cat
        .call("get_export_report", json!({"jobId":job["jobId"]}))
        .unwrap();
    assert_eq!(report["copiedCount"], 2);
    assert_eq!(report["xmpCopiedCount"], 2);
    let mut stems = std::collections::HashSet::new();
    for item in report["items"].as_array().unwrap() {
        let path = Path::new(item["destination"].as_str().unwrap());
        assert!(stems.insert(path.file_stem().unwrap()));
        assert!(f.output.join(path.with_extension("xmp")).is_file());
    }
    assert_eq!(
        fs::read(f.output.join("same.xmp")).unwrap(),
        b"existing metadata"
    );
    assert_eq!(before, hashes(&f.source));
}

#[test]
fn xmp_race_reports_partial_success_and_source_changes_skip_sidecars() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    f.add();
    let aid = f.assets()[0]["id"].clone();
    f.cat.authorize(&f.output, "export").unwrap();
    let request = json!({"target":{"kind":"explicit","assetIds":[aid]},"destinationPath":f.output,"layout":"flat","conflictPolicy":"rename","includeXmp":true});
    let preview = f.cat.call("preview_export", request.clone()).unwrap();
    fs::write(f.output.join("a.xmp"), b"racing metadata").unwrap();
    let job = f
        .cat
        .call(
            "start_export",
            json!({"planToken":preview["planToken"],"clientMutationId":"xmp-race"}),
        )
        .unwrap();
    let result = wait(&f.cat, job["jobId"].as_str().unwrap());
    assert_eq!(result["failureCount"], 1);
    let report = f
        .cat
        .call("get_export_report", json!({"jobId":job["jobId"]}))
        .unwrap();
    assert_eq!(report["copiedCount"], 1);
    assert_eq!(report["verifiedCount"], 1);
    assert_eq!(report["failedCount"], 0);
    assert_eq!(report["xmpFailedCount"], 1);
    assert_eq!(report["sidecars"][0]["errorCode"], "EXPORT_CONFLICT");
    assert_eq!(
        files::sha256(&f.output.join("a.jpg"), None).unwrap(),
        files::sha256(&f.source.join("a.jpg"), None).unwrap()
    );
    assert_eq!(
        fs::read(f.output.join("a.xmp")).unwrap(),
        b"racing metadata"
    );
    let preview = f.cat.call("preview_export", request).unwrap();
    fs::write(f.source.join("a.jpg"), b"externally changed test fixture").unwrap();
    let job = f
        .cat
        .call(
            "start_export",
            json!({"planToken":preview["planToken"],"clientMutationId":"changed-xmp-source"}),
        )
        .unwrap();
    wait(&f.cat, job["jobId"].as_str().unwrap());
    let report = f
        .cat
        .call("get_export_report", json!({"jobId":job["jobId"]}))
        .unwrap();
    assert_eq!(report["failedCount"], 1);
    assert_eq!(report["xmpSkippedCount"], 1);
    assert_eq!(report["sidecars"][0]["errorCode"], "MEDIA_EXPORT_FAILED");
    assert!(fs::read_dir(&f.output).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("travelgallery-part")));
}

#[test]
fn v4_catalog_upgrade_preserves_export_history_and_checks_sidecar_backup_shape() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let cat = Catalog::new(&data, &cache).unwrap();
    let c = cat.connection().unwrap();
    c.execute("INSERT INTO source_roots(id,display_name,absolute_path,created_at,updated_at) VALUES('s','s','/unused',0,0)",[]).unwrap();
    c.execute("INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,indexed_at,updated_at) VALUES('a','s','a.jpg','a.jpg','jpg','image',0,0,0)",[]).unwrap();
    c.execute("UPDATE asset_reviews SET rating=5 WHERE asset_id='a'", [])
        .unwrap();
    c.execute("INSERT INTO jobs(id,type,status,created_at,updated_at) VALUES('old-export','EXPORT','completed',0,0)",[]).unwrap();
    c.execute("INSERT INTO export_run_items VALUES('old-export','a','a.jpg','copied',0,NULL,'old-hash',0)",[]).unwrap();
    c.execute_batch(
        "DROP TABLE quality_hint_reviews;DROP TABLE quality_observations;DROP TABLE ai_cluster_members;DROP TABLE ai_clusters;DROP TABLE ai_embeddings;DROP TABLE review_undo_lineage;DROP TABLE export_sidecar_items;
         ALTER TABLE media_assets DROP COLUMN file_version;
         ALTER TABLE media_assets DROP COLUMN sha256_file_version;
         PRAGMA user_version=4;",
    )
    .unwrap();
    drop(c);
    drop(cat);
    let cat = Catalog::new(&data, &cache).unwrap();
    let c = cat.connection().unwrap();
    assert_eq!(
        c.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        db::CATALOG_VERSION
    );
    assert_eq!(
        c.query_row(
            "SELECT rating FROM asset_reviews WHERE asset_id='a'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        5
    );
    let report = cat
        .call("get_export_report", json!({"jobId":"old-export"}))
        .unwrap();
    assert_eq!(report["copiedCount"], 1);
    assert_eq!(report["xmpCopiedCount"], 0);
    assert!(report["sidecars"].as_array().unwrap().is_empty());
    let broken = temp.path().join("broken-current.sqlite");
    db::backup(&c, &broken).unwrap();
    let broken_c = rusqlite::Connection::open(&broken).unwrap();
    broken_c
        .execute_batch("DROP TABLE export_sidecar_items")
        .unwrap();
    drop(broken_c);
    cat.authorize(&broken, "restore").unwrap();
    assert!(cat
        .call("prepare_restore", json!({"selectedBackupPath":broken}))
        .is_err());
    assert_eq!(
        c.query_row(
            "SELECT rating FROM asset_reviews WHERE asset_id='a'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        5
    );
}

#[test]
fn consecutive_undo_follows_only_recorded_undo_lineage_and_keeps_the_journal_immutable() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    f.picture("b.jpg", 2);
    f.add();
    let before = hashes(&f.source);
    let assets = f.assets();
    let a = assets[0]["id"].as_str().unwrap().to_owned();
    let b = assets[1]["id"].as_str().unwrap().to_owned();
    let first = f.review(
        &[a.clone(), b.clone()],
        json!({"pickState":"keep","rating":4}),
        "undo-chain-a",
    );
    let second = f.review(&[a.clone()], json!({"note":"a note"}), "undo-chain-b");
    let third = f.review(&[b.clone()], json!({"isFavorite":true}), "undo-chain-c");
    let tag = f
        .cat
        .call("upsert_tag", json!({"name":"undo tag","color":null}))
        .unwrap();
    let fourth=f.cat.call("apply_tags_batch",json!({"target":{"kind":"explicit","assetIds":[a,b]},"addTagIds":[tag["tagId"]],"removeTagIds":[],"clientMutationId":"undo-chain-d"})).unwrap();
    let c = f.cat.connection().unwrap();
    let journal = || {
        c.prepare("SELECT batch_id,asset_id,before_json,after_json FROM change_batch_items ORDER BY batch_id,asset_id").unwrap().query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).unwrap().map(|r|r.unwrap()).collect::<Vec<_>>()
    };
    let original = journal();
    for (batch, count) in [(&fourth, 2), (&third, 1), (&second, 1), (&first, 2)] {
        let result = f
            .cat
            .call(
                "undo_review_batch",
                json!({"changeBatchId":batch["changeBatchId"]}),
            )
            .unwrap();
        assert_eq!(result["restoredCount"], count);
        assert_eq!(result["skippedCount"], 0);
    }
    for aid in [&a, &b] {
        let asset = f.cat.call("get_asset", json!({"assetId":aid})).unwrap();
        assert_eq!(asset["review"]["pickState"], "unreviewed");
        assert_eq!(asset["review"]["rating"], 0);
        assert_eq!(asset["review"]["note"], "");
        assert_eq!(asset["review"]["isFavorite"], false);
        assert!(asset["tagIds"].as_array().unwrap().is_empty());
        assert_eq!(asset["review"]["revision"], 6);
    }
    assert_eq!(journal(), original);
    assert_eq!(
        c.query_row("SELECT count(*) FROM review_undo_lineage", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        6
    );
    assert_eq!(before, hashes(&f.source));
}

#[test]
fn undo_lineage_does_not_accept_equal_values_after_new_writes_or_missing_v5_provenance() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    f.add();
    let aid = f.assets()[0]["id"].as_str().unwrap().to_owned();
    let first = f.review(&[aid.clone()], json!({"rating":4}), "aba-first");
    let second = f.review(&[aid.clone()], json!({"note":"temporary"}), "aba-second");
    f.cat
        .call(
            "undo_review_batch",
            json!({"changeBatchId":second["changeBatchId"]}),
        )
        .unwrap();
    f.review(&[aid.clone()], json!({"rating":5}), "aba-changed");
    f.review(&[aid.clone()], json!({"rating":4}), "aba-equal-again");
    let result = f
        .cat
        .call(
            "undo_review_batch",
            json!({"changeBatchId":first["changeBatchId"]}),
        )
        .unwrap();
    assert_eq!(result["restoredCount"], 0);
    assert_eq!(result["skippedCount"], 1);
    assert_eq!(
        f.cat.call("get_asset", json!({"assetId":aid})).unwrap()["review"]["rating"],
        4
    );
    // Simulate a v5 backup with an undo revision but no verifiable lineage.
    let third = f.review(&[aid.clone()], json!({"isFavorite":true}), "legacy-first");
    let fourth = f.review(&[aid.clone()], json!({"note":"legacy"}), "legacy-second");
    f.cat
        .call(
            "undo_review_batch",
            json!({"changeBatchId":fourth["changeBatchId"]}),
        )
        .unwrap();
    f.cat
        .connection()
        .unwrap()
        .execute("DELETE FROM review_undo_lineage", [])
        .unwrap();
    let result = f
        .cat
        .call(
            "undo_review_batch",
            json!({"changeBatchId":third["changeBatchId"]}),
        )
        .unwrap();
    assert_eq!(result["restoredCount"], 0);
    assert_eq!(result["skippedCount"], 1);
}

#[test]
fn undo_skips_a_removed_tag_deleted_while_unused_without_aborting_other_records() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    f.picture("b.jpg", 2);
    f.add();
    let assets = f.assets();
    let a = assets[0]["id"].as_str().unwrap().to_owned();
    let b = assets[1]["id"].as_str().unwrap().to_owned();
    let tag = f
        .cat
        .call("upsert_tag", json!({"name":"removed tag","color":null}))
        .unwrap();
    f.cat.call("apply_tags_batch",json!({"target":{"kind":"explicit","assetIds":[a]},"addTagIds":[tag["tagId"]],"removeTagIds":[],"clientMutationId":"add-before-removal"})).unwrap();
    let batch = f.review(
        &[a.clone(), b.clone()],
        json!({"rating":4}),
        "before-removal",
    );
    // The first asset's journal before-state includes a tag that is later deleted.
    let removed=f.cat.call("apply_tags_batch",json!({"target":{"kind":"explicit","assetIds":[a]},"addTagIds":[],"removeTagIds":[tag["tagId"]],"clientMutationId":"remove-before-deletion"})).unwrap();
    f.cat
        .call("delete_tag", json!({"tagId":tag["tagId"]}))
        .unwrap();
    let undo = f
        .cat
        .call(
            "undo_review_batch",
            json!({"changeBatchId":removed["changeBatchId"]}),
        )
        .unwrap();
    assert_eq!(undo["restoredCount"], 0);
    assert_eq!(undo["skippedCount"], 1);
    let undo = f
        .cat
        .call(
            "undo_review_batch",
            json!({"changeBatchId":batch["changeBatchId"]}),
        )
        .unwrap();
    assert_eq!(undo["restoredCount"], 1);
    assert_eq!(undo["skippedCount"], 1);
    assert_eq!(
        f.cat.call("get_asset", json!({"assetId":a})).unwrap()["review"]["rating"],
        4
    );
    assert_eq!(
        f.cat.call("get_asset", json!({"assetId":b})).unwrap()["review"]["rating"],
        0
    );
}

#[test]
fn export_presets_are_versioned_reusable_and_never_store_directory_authority() {
    let f = Fixture::new();
    f.cat.call("save_settings", json!({"fontSize":17})).unwrap();
    let options = json!({"layout":"captureMonth","conflictPolicy":"skip","includeLinked":true,"includeXmp":true,"naming":{"pattern":"山野_{date}_{seq}","start":7,"padding":4}});
    let saved = f
        .cat
        .call(
            "save_export_preset",
            json!({"name":"精选 📷","options":options}),
        )
        .unwrap();
    assert_eq!(saved["revision"], 1);
    assert_eq!(
        f.cat.call("list_export_presets", json!({})).unwrap()[0],
        saved
    );
    assert_eq!(
        f.cat.call("get_settings", json!({})).unwrap()["fontSize"],
        17
    );
    let updated = f
        .cat
        .call(
            "save_export_preset",
            json!({"id":saved["id"],"expectedRevision":1,"name":"旅行精选","options":options}),
        )
        .unwrap();
    assert_eq!(updated["revision"], 2);
    for command in ["save_export_preset", "delete_export_preset"] {
        assert_eq!(
            f.cat
                .call(
                    command,
                    json!({"id":saved["id"],"expectedRevision":1,"name":"旧名称","options":options})
                )
                .unwrap_err()
                .code,
            "DB_CONFLICT"
        );
    }
    assert!(f
        .cat
        .call(
            "save_export_preset",
            json!({"name":"旅行精选","options":options})
        )
        .is_err());
    let mut unsafe_options = options.clone();
    unsafe_options["destinationPath"] = json!(f.output);
    assert!(f
        .cat
        .call(
            "save_export_preset",
            json!({"name":"不能保存授权","options":unsafe_options})
        )
        .is_err());
    let c = f.cat.connection().unwrap();
    let backup = f._temp.path().join("preset-backup.sqlite");
    db::backup(&c, &backup).unwrap();
    let copied = db::connect(&backup).unwrap();
    assert_eq!(
        travel_gallery::export_options::list(&copied).unwrap()[0].name,
        "旅行精选"
    );
    db::initialize(&backup).unwrap();
    assert_eq!(
        travel_gallery::export_options::list(&copied).unwrap()[0].revision,
        2
    );
    f.cat
        .call(
            "delete_export_preset",
            json!({"id":saved["id"],"expectedRevision":2}),
        )
        .unwrap();
    assert!(f
        .cat
        .call("list_export_presets", json!({}))
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn settings_reads_supply_defaults_without_rewriting_legacy_preferences() {
    let f = Fixture::new();
    let defaults = f.cat.call("get_settings", json!({})).unwrap();
    assert_eq!(defaults["autoAdvance"], true);
    assert_eq!(defaults["density"], 220);
    assert_eq!(defaults["theme"], "system");
    assert_eq!(defaults.as_object().unwrap().len(), 10);
    let c = f.cat.connection().unwrap();
    assert!(db::setting(&c, "preferences").unwrap().is_none());
    let partial = json!({"theme":"dark","fontSize":17});
    f.cat.call("save_settings", partial.clone()).unwrap();
    let loaded = f.cat.call("get_settings", json!({})).unwrap();
    assert_eq!(loaded["theme"], "dark");
    assert_eq!(loaded["fontSize"], 17);
    assert_eq!(loaded["autoAdvance"], true);
    assert_eq!(loaded["cacheLimitGb"], 5);
    assert_eq!(loaded.as_object().unwrap().len(), 10);
    assert_eq!(serde_json::from_str::<Value>(&db::setting(&c, "preferences").unwrap().unwrap()).unwrap(), partial);
}

#[test]
fn naming_export_freezes_order_extensions_paths_and_xmp_collision_decisions() {
    let f = Fixture::new();
    f.picture("B.jpg", 1);
    f.picture("A.jpg", 2);
    f.add();
    let before = hashes(&f.source);
    let ids: Vec<_> = f
        .assets()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_string())
        .collect();
    let request = json!({"target":{"kind":"explicit","assetIds":ids},"destinationPath":f.output,"layout":"sourceRelative","conflictPolicy":"rename","includeXmp":true,"naming":{"pattern":"精选_{seq}","start":7,"padding":3}});
    assert_eq!(
        f.cat
            .call("preview_export", request.clone())
            .unwrap_err()
            .code,
        "PERMISSION_DENIED"
    );
    f.cat.authorize(&f.output, "export").unwrap();
    let folder = f.output.join("旅途 📷");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("精选_007.xmp"), b"preexisting").unwrap();
    let preview = f.cat.call("preview_export", request).unwrap();
    assert_eq!(preview["conflictCount"], 1);
    assert_eq!(preview["examples"][0]["sourceRelativePath"], "A.jpg");
    assert_eq!(
        preview["examples"][0]["destinationRelativePath"],
        "旅途 📷/精选_007 (2).jpg"
    );
    assert_eq!(
        preview["examples"][1]["destinationRelativePath"],
        "旅途 📷/精选_008.jpg"
    );
    let job = f
        .cat
        .call(
            "start_export",
            json!({"planToken":preview["planToken"],"clientMutationId":id()}),
        )
        .unwrap();
    assert_eq!(
        wait(&f.cat, job["jobId"].as_str().unwrap())["status"],
        "completed"
    );
    let report = f
        .cat
        .call("get_export_report", json!({"jobId":job["jobId"]}))
        .unwrap();
    assert_eq!(report["copiedCount"], 2);
    assert_eq!(report["xmpCopiedCount"], 2);
    assert_eq!(
        fs::read(folder.join("精选_007.xmp")).unwrap(),
        b"preexisting"
    );
    assert_eq!(hashes(&f.source), before);
}

#[test]
fn rename_preview_analyzes_real_conflicts_offline_sources_and_is_not_an_executable_export() {
    let f = Fixture::new();
    f.picture("A.jpg", 1);
    f.picture("B.jpg", 2);
    f.picture("sub/A.jpg", 3);
    f.add();
    let ids: Vec<_> = f.assets().iter().map(|a| a["id"].clone()).collect();
    fs::remove_file(f.source.join("B.jpg")).unwrap();
    fs::write(f.source.join("统一.jpg"), b"unindexed collision").unwrap();
    let before = hashes(&f.source);
    let job=f.cat.call("preview_rename",json!({"target":{"kind":"explicit","assetIds":ids},"rule":{"pattern":"统一","start":1,"padding":4}})).unwrap();
    let jid = job["jobId"].as_str().unwrap();
    assert_eq!(wait(&f.cat, jid)["status"], "completed");
    let report = f
        .cat
        .call(
            "get_rename_preview",
            json!({"jobId":jid,"offset":0,"pageSize":100}),
        )
        .unwrap();
    assert_eq!(
        report["summary"],
        json!({"total":3,"ready":1,"unchanged":0,"conflict":1,"invalid":0,"unavailable":1})
    );
    assert_eq!(report["job"]["operation"], "renamePreview");
    assert!(report["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["sourceRelativePath"] == "A.jpg" && a["status"] == "conflict"));
    assert_eq!(
        f.cat
            .call(
                "start_export",
                json!({"planToken":jid,"clientMutationId":id()})
            )
            .unwrap_err()
            .code,
        "INVALID_FILTER"
    );
    assert_eq!(hashes(&f.source), before);
    let c = f.cat.connection().unwrap();
    c.execute("UPDATE export_plans SET expires_at=0 WHERE token=?", [jid])
        .unwrap();
    assert_eq!(
        f.cat
            .call("get_rename_preview", json!({"jobId":jid}))
            .unwrap_err()
            .code,
        "SNAPSHOT_EXPIRED"
    );
}

#[test]
fn rename_preview_paginates_a_frozen_snapshot_and_can_cancel_without_source_writes() {
    let f = Fixture::new();
    for n in 0..125 {
        f.picture(&format!("P_{n:03}.png"), n);
    }
    f.add();
    let before = hashes(&f.source);
    let snap = f
        .cat
        .call(
            "create_selection_snapshot",
            json!({"filter":{},"sort":{"field":"filename","direction":"asc"}}),
        )
        .unwrap();
    let rule = json!({"pattern":"编号_{seq}","start":7,"padding":3});
    let job = f
        .cat
        .call(
            "preview_rename",
            json!({"target":{"kind":"snapshot","snapshotId":snap["id"]},"rule":rule}),
        )
        .unwrap();
    let jid = job["jobId"].as_str().unwrap();
    assert_eq!(wait(&f.cat, jid)["status"], "completed");
    let first = f
        .cat
        .call(
            "get_rename_preview",
            json!({"jobId":jid,"offset":0,"pageSize":500}),
        )
        .unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(), 100);
    assert_eq!(first["nextOffset"], 100);
    assert_eq!(first["items"][0]["proposedFilename"], "编号_007.png");
    let second = f
        .cat
        .call(
            "get_rename_preview",
            json!({"jobId":jid,"offset":100,"pageSize":100}),
        )
        .unwrap();
    assert_eq!(second["items"].as_array().unwrap().len(), 25);
    assert_eq!(second["nextOffset"], Value::Null);
    assert_eq!(second["items"][0]["proposedFilename"], "编号_107.png");
    let c = f.cat.connection().unwrap();
    let cancelled = id();
    travel_gallery::tasks::insert(&c,&cancelled,"REPAIR",&json!({"operation":"renamePreview","rule":rule,"target":first["target"],"cancelRequested":true})).unwrap();
    let outcome = travel_gallery::rename_preview::run(
        &f.cat,
        &cancelled,
        &json!({"rule":rule,"target":first["target"]}),
    )
    .unwrap_err();
    assert_eq!(outcome.code, "CANCELLED");
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM export_plans WHERE token=?",
            [&cancelled],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(hashes(&f.source), before);
}

#[test]
fn rename_preview_can_be_paused_then_cancelled_during_a_large_analysis() {
    let f = Fixture::new();
    f.picture("original.png", 1);
    let source = f.add();
    let before = hashes(&f.source);
    let mut c = f.cat.connection().unwrap();
    let tx = c.transaction().unwrap();
    {
        let mut insert=tx.prepare("INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,metadata_json,indexed_at,updated_at) VALUES(?,?,?,?,'png','image',1,'{}',0,0)").unwrap();
        for n in 0..10000 {
            let name = format!("indexed-{n:05}.png");
            insert
                .execute(rusqlite::params![
                    format!("naming-{n:05}"),
                    source,
                    name,
                    name
                ])
                .unwrap();
        }
    }
    tx.commit().unwrap();
    let snap = f
        .cat
        .call(
            "create_selection_snapshot",
            json!({"filter":{},"sort":{"field":"filename","direction":"asc"}}),
        )
        .unwrap();
    let job=f.cat.call("preview_rename",json!({"target":{"kind":"snapshot","snapshotId":snap["id"]},"rule":{"pattern":"预演_{seq}","start":1,"padding":5}})).unwrap();
    let jid = job["jobId"].as_str().unwrap();
    let start = Instant::now();
    loop {
        let state = f.cat.call("get_job", json!({"jobId":jid})).unwrap();
        if state["progressDone"].as_i64().unwrap() >= 100 {
            assert_eq!(state["status"], "running");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(5));
    }
    f.cat.call("pause_job", json!({"jobId":jid})).unwrap();
    assert_eq!(
        f.cat.call("get_job", json!({"jobId":jid})).unwrap()["status"],
        "paused"
    );
    f.cat.call("cancel_job", json!({"jobId":jid})).unwrap();
    let final_state = wait(&f.cat, jid);
    assert_eq!(final_state["status"], "cancelled");
    assert!(final_state["progressDone"].as_i64().unwrap() < 10001);
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM export_plans WHERE token=?",
            [jid],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(hashes(&f.source), before);
}

fn story_trip(f: &Fixture) -> String {
    let trip = f
        .cat
        .call(
            "save_trip",
            json!({"name":"旅途故事","description":"山野与回忆","startAt":null,"endAt":null}),
        )
        .unwrap()["tripId"]
        .as_str()
        .unwrap()
        .to_owned();
    let ids: Vec<_> = f.assets().into_iter().map(|a| a["id"].clone()).collect();
    f.cat
        .call(
            "add_to_trip",
            json!({"tripId":trip,"target":{"kind":"explicit","assetIds":ids}}),
        )
        .unwrap();
    trip
}
fn story_plan(f: &Fixture, trip: &str, notes: bool) -> Value {
    f.cat.authorize(&f.output, "export").unwrap();
    f.cat.call("preview_story_export",json!({"tripId":trip,"range":"all","title":"<script>回忆</script>","description":"旅途 & 日常","includeNotes":notes,"destinationPath":f.output})).unwrap()
}
#[test]
fn story_paginates_trusted_dates_keeps_unknown_last_and_scopes_members() {
    let f = Fixture::new();
    for (i, n) in ["known.jpg", "wall.jpg", "mtime.jpg", "outside.jpg"]
        .iter()
        .enumerate()
    {
        f.picture(n, i as u32);
    }
    f.add();
    let trip = story_trip(&f);
    let c = f.cat.connection().unwrap();
    c.execute("UPDATE media_assets SET capture_local='2026-06-15 10:00:00',capture_time_source='exif',capture_tz_min=480 WHERE filename='known.jpg'",[]).unwrap();
    c.execute("UPDATE media_assets SET capture_local='2026-06-14 23:00:00',capture_time_source='manual',capture_tz_min=NULL WHERE filename='wall.jpg'",[]).unwrap();
    c.execute("UPDATE media_assets SET capture_local='2000-01-01 00:00:00',capture_time_source='mtime' WHERE filename='mtime.jpg'",[]).unwrap();
    c.execute("DELETE FROM collection_assets WHERE asset_id IN(SELECT id FROM media_assets WHERE filename='outside.jpg')",[]).unwrap();
    c.execute("UPDATE asset_reviews SET pick_state='keep' WHERE asset_id IN(SELECT id FROM media_assets WHERE filename='wall.jpg')",[]).unwrap();
    let first = f
        .cat
        .call(
            "get_trip_story",
            json!({"tripId":trip,"range":"all","offset":0,"pageSize":2,"epoch":null}),
        )
        .unwrap();
    assert_eq!(first["total"], 3);
    assert_eq!(first["items"][0]["filename"], "wall.jpg");
    assert_eq!(first["items"][0]["unknownTimezone"], true);
    assert_eq!(first["items"][1]["date"], "2026-06-15");
    assert_eq!(first["nextOffset"], 2);
    assert!(!first.to_string().contains("sourcePath"));
    let last = f
        .cat
        .call(
            "get_trip_story",
            json!({"tripId":trip,"range":"all","offset":2,"pageSize":2,"epoch":first["epoch"]}),
        )
        .unwrap();
    assert_eq!(last["items"][0]["filename"], "mtime.jpg");
    assert!(last["items"][0]["date"].is_null());
    assert!(last["nextOffset"].is_null());
    let kept = f
        .cat
        .call(
            "get_trip_story",
            json!({"tripId":trip,"range":"keep","offset":0,"pageSize":100,"epoch":null}),
        )
        .unwrap();
    assert_eq!(kept["total"], 1);
    db::bump(&c).unwrap();
    assert_eq!(
        f.cat
            .call(
                "get_trip_story",
                json!({"tripId":trip,"range":"all","offset":2,"pageSize":2,"epoch":first["epoch"]})
            )
            .unwrap_err()
            .code,
        "DB_CONFLICT"
    );
    for bad in [
        json!({"tripId":trip,"range":"all","offset":0,"pageSize":0}),
        json!({"tripId":trip,"range":"all","offset":0,"pageSize":101}),
        json!({"tripId":trip,"range":"all","offset":0,"pageSize":1,"absolutePath":"/tmp"}),
    ] {
        assert_eq!(
            f.cat.call("get_trip_story", bad).unwrap_err().code,
            "INVALID_FILTER"
        );
    }
    c.execute("UPDATE source_roots SET is_enabled=0", [])
        .unwrap();
    assert_eq!(
        f.cat
            .call(
                "get_trip_story",
                json!({"tripId":trip,"range":"all","offset":0,"pageSize":100})
            )
            .unwrap()["total"],
        0
    );
}
#[test]
fn story_export_freezes_text_membership_and_verifies_private_offline_outputs() {
    let f = Fixture::new();
    f.picture("山 & 湖.jpg", 1);
    f.picture("second.jpg", 2);
    f.add();
    let trip = story_trip(&f);
    let before = hashes(&f.source);
    let assets = f.assets();
    let aid = assets[0]["id"].as_str().unwrap();
    f.review(&[aid.into()], json!({"note":"私人备注"}), "story-note");
    let plan = story_plan(&f, &trip, false);
    f.cat
        .call(
            "remove_from_trip",
            json!({"tripId":trip,"target":{"kind":"explicit","assetIds":[aid]}}),
        )
        .unwrap();
    f.review(&[aid.into()], json!({"note":"后来更改"}), "story-note-edit");
    let req = json!({"planToken":plan["planToken"],"clientMutationId":"story-first"});
    assert_eq!(
        f.cat.call("start_export", req.clone()).unwrap_err().code,
        "INVALID_FILTER"
    );
    let started = f.cat.call("start_story_export", req.clone()).unwrap();
    assert_eq!(f.cat.call("start_story_export", req).unwrap(), started);
    let job = wait(&f.cat, started["jobId"].as_str().unwrap());
    assert_eq!(job["status"], "completed", "{job}");
    assert_eq!(job["operation"], "storyExport");
    assert_eq!(job["progressDone"], 4);
    assert_eq!(job["failureCount"], 0);
    let report = f
        .cat
        .call("get_export_report", json!({"jobId":started["jobId"]}))
        .unwrap();
    assert_eq!(report["copiedCount"], 2);
    assert_eq!(report["verifiedCount"], 2);
    assert!(report["manifestPath"].is_null());
    assert!(report["story"]["result"]["html"]["sha256"].is_string());
    let output = f.output.join(plan["folder"].as_str().unwrap());
    let html = fs::read_to_string(output.join("index.html")).unwrap();
    assert!(html.contains("&lt;script&gt;回忆&lt;/script&gt;"));
    assert!(!html.contains("<script>"));
    assert!(!html.contains("私人备注"));
    assert!(!html.contains("后来更改"));
    assert!(!html.contains(f.source.to_str().unwrap()));
    assert!(html.contains("原文件未包含"));
    let manifest: Value =
        serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["previewCount"], 2);
    assert_eq!(manifest["originalsIncluded"], false);
    for file in manifest["files"].as_array().unwrap() {
        let path = output.join(file["relative"].as_str().unwrap());
        assert_eq!(
            files::sha256(&path, None).unwrap(),
            file["sha256"].as_str().unwrap()
        );
        assert_eq!(
            fs::metadata(path).unwrap().len(),
            file["bytes"].as_u64().unwrap()
        );
    }
    assert_eq!(fs::read_dir(&output).unwrap().count(), 4);
    for name in ["00001.jpg", "00002.jpg"] {
        let bytes = fs::read(output.join(name)).unwrap();
        assert!(!bytes.windows(6).any(|b| b == b"Exif\0\0"));
        image::load_from_memory(&bytes).unwrap();
    }
    assert_eq!(before, hashes(&f.source));
    assert_eq!(
        f.cat
            .call(
                "start_story_export",
                json!({"planToken":plan["planToken"],"clientMutationId":"story-other"})
            )
            .unwrap_err()
            .code,
        "SNAPSHOT_EXPIRED"
    );
}
#[test]
fn story_export_includes_only_opted_in_frozen_notes_and_reports_changed_sources() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    f.picture("b.jpg", 2);
    f.add();
    let trip = story_trip(&f);
    let assets = f.assets();
    let aid = assets[0]["id"].as_str().unwrap();
    f.review(
        &[aid.into()],
        json!({"note":"<img src=x> 最初备注"}),
        "note-included",
    );
    let plan = story_plan(&f, &trip, true);
    f.review(
        &[aid.into()],
        json!({"note":"不该出现的后来备注"}),
        "note-changed",
    );
    fs::write(f.source.join("b.jpg"), b"changed").unwrap();
    let jid = f
        .cat
        .call(
            "start_story_export",
            json!({"planToken":plan["planToken"],"clientMutationId":"story-partial"}),
        )
        .unwrap()["jobId"]
        .as_str()
        .unwrap()
        .to_owned();
    let job = wait(&f.cat, &jid);
    assert_eq!(job["status"], "completed");
    assert_eq!(job["failureCount"], 1);
    let report = f
        .cat
        .call("get_export_report", json!({"jobId":jid}))
        .unwrap();
    assert_eq!(report["copiedCount"], 1);
    assert_eq!(report["failedCount"], 1);
    assert!(report["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["errorCode"] == "DB_CONFLICT"));
    let html = fs::read_to_string(
        f.output
            .join(plan["folder"].as_str().unwrap())
            .join("index.html"),
    )
    .unwrap();
    assert!(html.contains("&lt;img src=x&gt; 最初备注"));
    assert!(!html.contains("不该出现的后来备注"));
    assert!(html.contains("此项预览未导出"));
    // Unknown dates sort by persisted asset ID; UUID order is independent of
    // source filename. Check the failed item's actual planned filename.
    let failed = report["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["state"] == "failed")
        .unwrap();
    let failed_name = Path::new(failed["destination"].as_str().unwrap())
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    assert!(!html.contains(&format!("src=\"{failed_name}\"")));
}
#[cfg(unix)]
#[test]
fn story_export_rejects_same_stamp_rewrite_with_warm_cache_and_rebuilds_after_rescan() {
    let f = Fixture::new();
    let a = f.source.join("a.bmp");
    let b = f.source.join("b.bmp");
    image::RgbImage::from_pixel(16, 16, image::Rgb([240, 0, 0]))
        .save(&a)
        .unwrap();
    image::RgbImage::from_pixel(16, 16, image::Rgb([0, 240, 0]))
        .save(&b)
        .unwrap();
    let source = f.add();
    let trip = story_trip(&f);
    let assets = f.assets();
    let aid = assets[0]["id"].as_str().unwrap();
    let bid = assets[1]["id"].as_str().unwrap();
    let mut c = f.cat.connection().unwrap();
    let old_screen = media::thumbnail(&f.cat, &mut c, aid, "screen").unwrap();
    let fresh_screen = media::thumbnail(&f.cat, &mut c, bid, "screen").unwrap();
    // Model a cache from before filenames were derived from the file version.
    let old_name = old_screen.file_name().unwrap().to_str().unwrap();
    c.execute(
        "UPDATE media_assets SET file_version=NULL WHERE id=?",
        [bid],
    )
    .unwrap();
    c.execute(
        "UPDATE thumbnail_cache SET cache_rel_path=? WHERE asset_id=? AND variant='screen'",
        rusqlite::params![old_name, bid],
    )
    .unwrap();

    let frozen = story_plan(&f, &trip, false);
    let mtime = fs::metadata(&a).unwrap().modified().unwrap();
    let stamp = files::stamp(&a).unwrap();
    let bytes = fs::read(&b).unwrap();
    assert_eq!(bytes.len() as u64, stamp.0);
    assert_ne!(fs::read(&a).unwrap(), bytes);
    fs::write(&a, &bytes).unwrap();
    fs::File::options()
        .write(true)
        .open(&a)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(mtime))
        .unwrap();
    assert_eq!(fs::metadata(&a).unwrap().modified().unwrap(), mtime);
    assert_eq!(files::stamp(&a).unwrap(), stamp);

    let export = |plan: &Value, mutation: &str| {
        let job = f
            .cat
            .call(
                "start_story_export",
                json!({
                    "planToken":plan["planToken"], "clientMutationId":mutation
                }),
            )
            .unwrap();
        assert_eq!(
            wait(&f.cat, job["jobId"].as_str().unwrap())["status"],
            "completed"
        );
        f.cat
            .call("get_export_report", json!({"jobId":job["jobId"]}))
            .unwrap()
    };
    let report = export(&frozen, "story-same-stamp");
    assert_eq!(report["verifiedCount"], 1);
    assert_eq!(report["failedCount"], 1);
    let items = report["items"].as_array().unwrap();
    let failed = items.iter().find(|row| row["assetId"] == aid).unwrap();
    let success = items.iter().find(|row| row["assetId"] == bid).unwrap();
    assert_eq!(failed["errorCode"], "DB_CONFLICT");
    let failed_path = f.output.join(failed["destination"].as_str().unwrap());
    assert!(!failed_path.exists());
    let folder = failed_path.parent().unwrap();
    let html = fs::read_to_string(folder.join("index.html")).unwrap();
    assert!(html.contains("此项预览未导出"));
    assert!(!html.contains(failed_path.file_name().unwrap().to_str().unwrap()));
    assert_eq!(fs::read_dir(folder).unwrap().count(), 3);
    let pixel = image::open(f.output.join(success["destination"].as_str().unwrap()))
        .unwrap()
        .to_rgb8();
    assert!(pixel.get_pixel(0, 0)[1] > 200);
    assert_ne!(old_screen, fresh_screen);

    assert_eq!(scan(&f.cat, &source)["status"], "completed");
    let fresh = story_plan(&f, &trip, false);
    assert_eq!(export(&fresh, "story-rescanned")["verifiedCount"], 2);
    assert_eq!(fs::read(&a).unwrap(), bytes);
}

#[test]
fn story_plans_require_authority_correct_operation_ttl_and_fresh_directory() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    f.add();
    let trip = story_trip(&f);
    let req = json!({"tripId":trip,"range":"all","title":"旅行","description":"","includeNotes":false,"destinationPath":f.output});
    assert_eq!(
        f.cat
            .call("preview_story_export", req.clone())
            .unwrap_err()
            .code,
        "PERMISSION_DENIED"
    );
    f.cat.authorize(&f.source, "export").unwrap();
    let mut overlap = req.clone();
    overlap["destinationPath"] = json!(f.source);
    assert_eq!(
        f.cat
            .call("preview_story_export", overlap)
            .unwrap_err()
            .code,
        "EXPORT_TARGET_UNSAFE"
    );
    let plan = story_plan(&f, &trip, false);
    let folder = f.output.join(plan["folder"].as_str().unwrap());
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("precious"), "must remain").unwrap();
    let jid = f
        .cat
        .call(
            "start_story_export",
            json!({"planToken":plan["planToken"],"clientMutationId":"conflict-story"}),
        )
        .unwrap()["jobId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(wait(&f.cat, &jid)["status"], "failed");
    assert_eq!(
        fs::read_to_string(folder.join("precious")).unwrap(),
        "must remain"
    );
    assert_eq!(fs::read_dir(folder).unwrap().count(), 1);
    let expired = story_plan(&f, &trip, false);
    let c = f.cat.connection().unwrap();
    c.execute(
        "UPDATE export_plans SET expires_at=0 WHERE token=?",
        [expired["planToken"].as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        f.cat
            .call(
                "start_story_export",
                json!({"planToken":expired["planToken"],"clientMutationId":"expired-story"})
            )
            .unwrap_err()
            .code,
        "SNAPSHOT_EXPIRED"
    );
    let ordinary=f.cat.call("preview_export",json!({"target":{"kind":"explicit","assetIds":[f.assets()[0]["id"]]},"destinationPath":f.output,"layout":"flat","conflictPolicy":"rename"})).unwrap();
    assert_eq!(
        f.cat
            .call(
                "start_story_export",
                json!({"planToken":ordinary["planToken"],"clientMutationId":"wrong-kind"})
            )
            .unwrap_err()
            .code,
        "INVALID_FILTER"
    );
}
#[test]
fn story_export_pauses_resumes_and_cancels_without_publishing_html_or_temp_files() {
    let f = Fixture::new();
    for i in 0..40 {
        f.picture(&format!("{i:03}.jpg"), i);
    }
    f.add();
    let trip = story_trip(&f);
    let before = hashes(&f.source);
    let preview = story_plan(&f, &trip, false);
    let c = f.cat.connection().unwrap();
    let plan: String = c
        .query_row(
            "SELECT plan_json FROM export_plans WHERE token=?",
            [preview["planToken"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    let plan: Value = serde_json::from_str(&plan).unwrap();
    let jid = "controlled-story";
    travel_gallery::tasks::insert(&c, jid, "EXPORT", &plan).unwrap();
    f.cat.call("pause_job", json!({"jobId":jid})).unwrap();
    let cat = f.cat.clone();
    let worker = thread::spawn(move || travel_gallery::story::run(&cat, jid, &plan));
    thread::sleep(Duration::from_millis(200));
    let folder = f.output.join(preview["folder"].as_str().unwrap());
    assert!(!folder.exists());
    f.cat.call("resume_job", json!({"jobId":jid})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let job = f.cat.call("get_job", json!({"jobId":jid})).unwrap();
        if job["progressDone"].as_u64().unwrap() > 0 {
            break;
        }
        if worker.is_finished() {
            panic!(
                "story worker ended before its first progress: {:?}",
                worker.join().unwrap()
            );
        }
        assert!(
            Instant::now() < deadline,
            "story did not advance after resume: {job}"
        );
        thread::sleep(Duration::from_millis(2));
    }
    f.cat.call("pause_job", json!({"jobId":jid})).unwrap();
    thread::sleep(Duration::from_millis(150));
    let done = f.cat.call("get_job", json!({"jobId":jid})).unwrap()["progressDone"].clone();
    thread::sleep(Duration::from_millis(150));
    assert_eq!(
        f.cat.call("get_job", json!({"jobId":jid})).unwrap()["progressDone"],
        done
    );
    f.cat.call("cancel_job", json!({"jobId":jid})).unwrap();
    assert_eq!(worker.join().unwrap().unwrap_err().code, "CANCELLED");
    assert!(!folder.join("index.html").exists());
    assert!(fs::read_dir(folder).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("travelgallery-part")));
    assert_eq!(before, hashes(&f.source));
}
#[test]
fn story_export_enforces_item_and_text_bounds_before_creating_a_plan() {
    let f = Fixture::new();
    f.picture("real.jpg", 1);
    let source = f.add();
    let trip = story_trip(&f);
    f.cat.authorize(&f.output, "export").unwrap();
    let mut c = f.cat.connection().unwrap();
    let tx = c.transaction().unwrap();
    {
        let mut insert=tx.prepare("INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,indexed_at,updated_at) VALUES(?,?,?,?,'jpg','image',0,0,0)").unwrap();
        for n in 0..5000 {
            let name = format!("virtual-{n:05}.jpg");
            insert
                .execute(rusqlite::params![
                    format!("story-bound-{n}"),
                    source,
                    name,
                    name
                ])
                .unwrap();
        }
    }
    tx.execute(
        "INSERT OR IGNORE INTO collection_assets SELECT ?,id,0 FROM media_assets",
        [&trip],
    )
    .unwrap();
    tx.commit().unwrap();
    let req = json!({"tripId":trip,"range":"all","title":"旅行","description":"","includeNotes":false,"destinationPath":f.output});
    assert_eq!(
        f.cat
            .call("preview_story_export", req.clone())
            .unwrap_err()
            .code,
        "INVALID_SELECTION"
    );
    c.execute(
        "DELETE FROM collection_assets WHERE asset_id='story-bound-0'",
        [],
    )
    .unwrap();
    let plan = f.cat.call("preview_story_export", req.clone()).unwrap();
    assert_eq!(plan["itemCount"], 5000);
    assert_eq!(plan["offlineCount"], 4999);
    c.execute(
        "DELETE FROM collection_assets WHERE asset_id LIKE 'story-bound-%'",
        [],
    )
    .unwrap();
    c.execute("UPDATE asset_reviews SET note=? WHERE asset_id IN(SELECT asset_id FROM collection_assets WHERE collection_id=?)",rusqlite::params!["a".repeat(8*1024*1024+1),trip]).unwrap();
    let mut notes = req.clone();
    notes["includeNotes"] = json!(true);
    assert_eq!(
        f.cat.call("preview_story_export", notes).unwrap_err().code,
        "INVALID_SELECTION"
    );
    assert_eq!(
        f.cat.call("preview_story_export", req.clone()).unwrap()["itemCount"],
        1
    );
    let mut invalid = req;
    invalid["includeNotes"] = json!("yes");
    assert_eq!(
        f.cat
            .call("preview_story_export", invalid)
            .unwrap_err()
            .code,
        "INVALID_FILTER"
    );
    assert!(fs::read_dir(&f.output).unwrap().next().is_none());
}

#[test]
fn local_ai_defaults_off_requires_picker_and_rejects_forged_model_packages() {
    let f = Fixture::new();
    f.picture("ordinary.jpg", 4);
    f.add();
    let before = hashes(&f.source);
    let status = f.cat.call("get_ai_status", json!({"filter":{}})).unwrap();
    assert_eq!(status["enabled"], false);
    assert_eq!(status["modelAvailable"], false);
    assert_eq!(status["runtime"], "idle");
    assert_eq!(status["total"], 1);
    assert_eq!(status["indexedCount"], 0);
    assert_eq!(
        f.cat
            .call("set_ai_enabled", json!({"enabled":true}))
            .unwrap_err()
            .code,
        "AI_MODEL_MISSING"
    );
    assert_eq!(
        f.cat
            .call("set_ai_enabled", json!({"enabled":"true"}))
            .unwrap_err()
            .code,
        "INVALID_FILTER"
    );
    assert_eq!(
        f.cat
            .call(
                "start_ai_index",
                json!({"filter":{},"clientMutationId":"off-index"})
            )
            .unwrap_err()
            .code,
        "AI_DISABLED"
    );
    assert_eq!(
        f.cat
            .call(
                "semantic_search",
                json!({"filter":{},"text":"教堂","limit":60})
            )
            .unwrap_err()
            .code,
        "AI_DISABLED"
    );
    assert_eq!(
        f.cat
            .call(
                "import_ai_model",
                json!({"selectedPath":f.output,"clientMutationId":"no-grant"})
            )
            .unwrap_err()
            .code,
        "PERMISSION_DENIED"
    );
    f.cat.authorize(&f.output, "model").unwrap();
    let mut profile = serde_json::to_value(travel_gallery::model_resources::profile()).unwrap();
    profile["files"][0]["path"] = json!("../ordinary.jpg");
    fs::write(
        f.output.join("travel-gallery-model.json"),
        profile.to_string(),
    )
    .unwrap();
    assert_eq!(
        f.cat
            .call(
                "import_ai_model",
                json!({"selectedPath":f.output,"clientMutationId":"forged-package"})
            )
            .unwrap_err()
            .code,
        "AI_MODEL_INVALID"
    );
    fs::write(
        f.output.join("travel-gallery-model.json"),
        serde_json::to_vec(travel_gallery::model_resources::profile()).unwrap(),
    )
    .unwrap();
    assert!(f
        .cat
        .call(
            "import_ai_model",
            json!({"selectedPath":f.output,"clientMutationId":"missing-weights"})
        )
        .is_err());
    assert_eq!(before, hashes(&f.source));
    assert_eq!(
        f.cat.call("get_ai_status", json!({"filter":{}})).unwrap()["enabled"],
        false
    );
    let c = f.cat.connection().unwrap();
    assert_eq!(c.query_row("SELECT count(*) FROM jobs WHERE json_extract(checkpoint_json,'$.operation') LIKE 'ai%'",[],|r|r.get::<_,usize>(0)).unwrap(),0);
}
#[test]
fn ai_content_groups_preserve_human_reviews_filter_by_live_input_versions_and_survive_disable() {
    let f = Fixture::new();
    f.picture("a.jpg", 1);
    f.picture("b.jpg", 2);
    let source = f.add();
    let assets = f.assets();
    let before = hashes(&f.source);
    let c = f.cat.connection().unwrap();
    c.execute("INSERT INTO jobs(id,type,status,checkpoint_json,created_at,updated_at) VALUES('content-job','REPAIR','completed',?,0,0)",[json!({"operation":"aiCluster","filter":{"combineLinked":false},"summary":{"scopeCount":2,"clusteredCount":2,"failedCount":0,"desiredGroups":2,"actualGroups":1},"modelVersion":travel_gallery::local_ai::version()}).to_string()]).unwrap();
    c.execute(
        "INSERT INTO ai_clusters VALUES('content-group','content-job','内容组 1',?,?,0)",
        rusqlite::params![
            travel_gallery::local_ai::version(),
            travel_gallery::ai_vectors::CLUSTER_VERSION
        ],
    )
    .unwrap();
    let mut vector = vec![0.; 768];
    vector[0] = 1.;
    let blob = travel_gallery::ai_vectors::pack(&vector).unwrap();
    for a in &assets {
        let key = "a".repeat(64);
        c.execute("INSERT INTO ai_embeddings SELECT id,?,byte_size,mtime_ms,COALESCE(orientation,1),?,768,?,0 FROM media_assets WHERE id=?",rusqlite::params![travel_gallery::local_ai::version(),key,blob,a["id"].as_str()]).unwrap();
        c.execute(
            "INSERT INTO ai_cluster_members VALUES('content-group',?,?,1)",
            rusqlite::params![a["id"].as_str(), key],
        )
        .unwrap();
    }
    let result = f
        .cat
        .call(
            "get_ai_clusters",
            json!({"filter":{"combineLinked":false},"jobId":null}),
        )
        .unwrap();
    assert_eq!(result["jobId"], "content-job");
    assert_eq!(result["groups"][0]["validCount"], 2);
    assert_eq!(result["groups"][0]["staleCount"], 0);
    assert_eq!(result["filter"], json!({"combineLinked":false}));
    let query = || {
        f.cat.call("query_assets",json!({"filter":{"aiClusterId":"content-group","combineLinked":false},"sort":{"field":"filename","direction":"asc"},"cursor":null,"pageSize":100})).unwrap()
    };
    assert_eq!(query()["total"], 2);
    f.cat
        .call("set_ai_enabled", json!({"enabled":false}))
        .unwrap();
    assert_eq!(query()["total"], 2);
    c.execute(
        "UPDATE media_assets SET mtime_ms=mtime_ms+1 WHERE id=?",
        [assets[0]["id"].as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(query()["total"], 1);
    let groups = f
        .cat
        .call(
            "get_ai_clusters",
            json!({"filter":{},"jobId":"content-job"}),
        )
        .unwrap();
    assert_eq!(groups["groups"][0]["staleCount"], 1);
    f.cat
        .call(
            "set_source_enabled",
            json!({"sourceId":source,"enabled":false}),
        )
        .unwrap();
    assert_eq!(query()["total"], 0);
    assert_eq!(before, hashes(&f.source));
    assert!(f.assets().is_empty());
    let reviews:usize=c.query_row("SELECT count(*) FROM asset_reviews WHERE pick_state='unreviewed' AND rating=0 AND is_favorite=0 AND note='' AND revision=0",[],|r|r.get(0)).unwrap();
    assert_eq!(reviews, 2);
    assert_eq!(
        f.cat.call("get_ai_status", json!({"filter":{}})).unwrap()["runtime"],
        "idle"
    );
}
#[test]
fn removing_ai_data_cannot_remove_extra_files_or_follow_redirected_model_storage() {
    let f = Fixture::new();
    let models = f._temp.path().join("app/models");
    let model = models.join(&travel_gallery::model_resources::profile().id);
    fs::create_dir_all(&model).unwrap();
    fs::write(model.join("precious.txt"), "keep").unwrap();
    assert_eq!(
        f.cat
            .call(
                "remove_ai_model",
                json!({"confirmed":false,"clientMutationId":"unconfirmed-model"})
            )
            .unwrap_err()
            .code,
        "INVALID_FILTER"
    );
    let req = json!({"confirmed":true,"clientMutationId":"remove-extra"});
    let result = f.cat.call("remove_ai_model", req.clone()).unwrap();
    assert_eq!(f.cat.call("remove_ai_model", req).unwrap(), result);
    let job = wait(&f.cat, result["jobId"].as_str().unwrap());
    assert_eq!(job["status"], "failed");
    assert_eq!(
        fs::read_to_string(model.join("precious.txt")).unwrap(),
        "keep"
    );
    #[cfg(unix)]
    {
        fs::remove_file(model.join("precious.txt")).unwrap();
        fs::remove_dir(&model).unwrap();
        fs::remove_dir(&models).unwrap();
        std::os::unix::fs::symlink(&f.source, &models).unwrap();
        fs::write(f.source.join("preserve.txt"), "source").unwrap();
        let result = f
            .cat
            .call(
                "remove_ai_model",
                json!({"confirmed":true,"clientMutationId":"redirected-model"}),
            )
            .unwrap();
        assert_eq!(
            wait(&f.cat, result["jobId"].as_str().unwrap())["status"],
            "failed"
        );
        assert_eq!(
            fs::read_to_string(f.source.join("preserve.txt")).unwrap(),
            "source"
        );
    }
}
#[test]
fn v7_backup_rejects_nonfinite_ai_vectors_without_restoring_them() {
    let f = Fixture::new();
    f.picture("image.jpg", 1);
    f.add();
    let c = f.cat.connection().unwrap();
    let blob = [f32::NAN.to_le_bytes(); 768].concat();
    c.execute("INSERT INTO ai_embeddings SELECT id,?,byte_size,mtime_ms,COALESCE(orientation,1),?,768,?,0 FROM media_assets",rusqlite::params![travel_gallery::local_ai::version(),"a".repeat(64),blob]).unwrap();
    let backup = f.output.join("invalid-ai.sqlite");
    db::backup(&c, &backup).unwrap();
    f.cat.authorize(&backup, "restore").unwrap();
    assert_eq!(
        f.cat
            .call("prepare_restore", json!({"selectedBackupPath":backup}))
            .unwrap_err()
            .code,
        "DB_MIGRATION_FAILED"
    );
}

#[test]
fn quality_analysis_is_opt_in_versioned_reusable_and_dismissal_is_independent() {
    let f = Fixture::new();
    f.picture("detail-a.jpg", 5);
    f.picture("detail-b.jpg", 10);
    f.add();
    let originals = hashes(&f.source);
    let assets = f.assets();
    let aid = assets[0]["id"].as_str().unwrap();
    let c = f.cat.connection().unwrap();
    let human_before:String=c.query_row("SELECT json_group_array(json_object('id',asset_id,'revision',revision,'pick',pick_state,'rating',rating,'favorite',is_favorite)) FROM asset_reviews",[],|r|r.get(0)).unwrap();
    let status = f
        .cat
        .call("get_quality_status", json!({"filter":{}}))
        .unwrap();
    assert_eq!(status["eyesEnabled"], false);
    assert_eq!(status["detailEnabled"], false);
    assert_eq!(status["runtime"], "idle");
    assert_eq!(status["total"], 2);
    assert_eq!(
        f.cat
            .call("set_eyes_enabled", json!({"enabled":true}))
            .unwrap_err()
            .code,
        "AI_MODEL_MISSING"
    );
    assert_eq!(
        f.cat
            .call("set_detail_enabled", json!({"enabled":"true"}))
            .unwrap_err()
            .code,
        "INVALID_FILTER"
    );
    let req = json!({"filter":{"keyword":"detail-a"},"useEyes":false,"useDetail":true,"clientMutationId":"detail-run-1"});
    assert_eq!(
        f.cat
            .call("start_quality_analysis", req.clone())
            .unwrap_err()
            .code,
        "AI_DISABLED"
    );
    f.cat
        .call("set_detail_enabled", json!({"enabled":true}))
        .unwrap();
    let job = f.cat.call("start_quality_analysis", req.clone()).unwrap();
    assert_eq!(f.cat.call("start_quality_analysis", req).unwrap(), job);
    let job = wait(&f.cat, job["jobId"].as_str().unwrap());
    assert_eq!(job["status"], "completed", "{job}");
    assert_eq!(job["failureCount"], 0);
    assert_eq!(job["progressTotal"], 1);
    let analysis = f
        .cat
        .call("get_asset_quality", json!({"assetId":aid}))
        .unwrap();
    assert_eq!(analysis["observations"].as_array().unwrap().len(), 1);
    let observation = analysis["observations"][0].clone();
    assert_eq!(observation["kind"], "detail");
    assert!(observation["value"]["laplacianVariance"].as_f64().unwrap() > 0.);
    let repeated=f.cat.call("start_quality_analysis",json!({"filter":{"keyword":"detail-a"},"useEyes":false,"useDetail":true,"clientMutationId":"detail-run-2"})).unwrap();
    assert_eq!(
        wait(&f.cat, repeated["jobId"].as_str().unwrap())["status"],
        "completed"
    );
    assert_eq!(
        f.cat
            .call("get_asset_quality", json!({"assetId":aid}))
            .unwrap(),
        analysis
    );
    let req = json!({"assetId":aid,"kind":"detail","inputKey":observation["inputKey"],"dismissed":true,"clientMutationId":"dismiss-detail-1"});
    let receipt = f.cat.call("set_quality_dismissed", req.clone()).unwrap();
    assert_eq!(f.cat.call("set_quality_dismissed", req).unwrap(), receipt);
    let page = |include: bool| {
        f.cat
            .call(
                "get_quality_results",
                json!({"filter":{},"offset":0,"onlyHints":false,"includeDismissed":include}),
            )
            .unwrap()
    };
    assert_eq!(page(false)["total"], 0);
    assert_eq!(page(true)["total"], 1);
    assert_eq!(
        page(true)["items"][0]["analysis"]["observations"][0]["dismissed"],
        true
    );
    assert_eq!(
        f.cat
            .call(
                "get_quality_results",
                json!({"filter":{},"offset":0,"onlyHints":true,"includeDismissed":true})
            )
            .unwrap()["total"],
        0
    );
    // Dismissal cannot accidentally mutate the human review revision or source.
    let human_after:String=c.query_row("SELECT json_group_array(json_object('id',asset_id,'revision',revision,'pick',pick_state,'rating',rating,'favorite',is_favorite)) FROM asset_reviews",[],|r|r.get(0)).unwrap();
    assert_eq!(human_before, human_after);
    assert_eq!(originals, hashes(&f.source));
    // A physical change makes cached output inaccessible even before rescan.
    f.picture("detail-a.jpg", 99);
    let stale = f
        .cat
        .call("get_asset_quality", json!({"assetId":aid}))
        .unwrap();
    assert_eq!(stale["staleCount"], 1);
    assert_eq!(stale["observations"].as_array().unwrap().len(), 0);
    assert_eq!(f.cat.call("set_quality_dismissed",json!({"assetId":aid,"kind":"detail","inputKey":observation["inputKey"],"dismissed":false,"clientMutationId":"stale-dismiss"})).unwrap_err().code,"DB_CONFLICT");
    // Scanned input version and rebuilt result do not inherit the old dismissal.
    let sid = assets[0]["sourceId"].as_str().unwrap();
    let scan = f
        .cat
        .call("rescan_source", json!({"sourceId":sid}))
        .unwrap();
    wait(&f.cat, scan["jobId"].as_str().unwrap());
    wait_all(&f.cat);
    let again=f.cat.call("start_quality_analysis",json!({"filter":{"keyword":"detail-a"},"useEyes":false,"useDetail":true,"clientMutationId":"detail-run-3"})).unwrap();
    assert_eq!(
        wait(&f.cat, again["jobId"].as_str().unwrap())["status"],
        "completed"
    );
    let fresh = f
        .cat
        .call("get_asset_quality", json!({"assetId":aid}))
        .unwrap();
    assert_eq!(fresh["observations"][0]["dismissed"], false);
    assert_ne!(
        fresh["observations"][0]["inputKey"],
        observation["inputKey"]
    );
    f.cat
        .call("set_detail_enabled", json!({"enabled":false}))
        .unwrap();
    assert_eq!(
        f.cat.call("get_ai_status", json!({"filter":{}})).unwrap()["enabled"],
        false
    );
}

#[test]
fn eyes_package_rejects_wrong_model_and_disabling_cancels_only_its_own_analysis() {
    let f = Fixture::new();
    f.picture("face.jpg", 20);
    f.add();
    assert_eq!(
        f.cat
            .call(
                "import_eyes_model",
                json!({"selectedPath":f.output,"clientMutationId":"eyes-no-grant"})
            )
            .unwrap_err()
            .code,
        "PERMISSION_DENIED"
    );
    f.cat.authorize(&f.output, "model").unwrap();
    fs::write(
        f.output.join("travel-gallery-model.json"),
        serde_json::to_vec(travel_gallery::model_resources::profile()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        f.cat
            .call(
                "import_eyes_model",
                json!({"selectedPath":f.output,"clientMutationId":"eyes-wrong-package"})
            )
            .unwrap_err()
            .code,
        "AI_MODEL_INVALID"
    );
    let c = f.cat.connection().unwrap();
    for (jid, op, eyes, detail) in [
        ("q-eyes", "qualityAnalyze", true, false),
        ("q-detail", "qualityAnalyze", false, true),
        ("q-content", "aiIndex", false, false),
    ] {
        c.execute("INSERT INTO jobs(id,type,status,checkpoint_json,created_at,updated_at) VALUES(?,'REPAIR','paused',?,0,0)",rusqlite::params![jid,json!({"operation":op,"useEyes":eyes,"useDetail":detail}).to_string()]).unwrap();
    }
    f.cat
        .call("set_eyes_enabled", json!({"enabled":false}))
        .unwrap();
    let cancelled = |jid: &str| {
        c.query_row("SELECT COALESCE(json_extract(checkpoint_json,'$.cancelRequested'),0) FROM jobs WHERE id=?",[jid],|r|r.get::<_,bool>(0)).unwrap()
    };
    assert!(cancelled("q-eyes"));
    assert!(!cancelled("q-detail"));
    assert!(!cancelled("q-content"));
    f.cat
        .call("set_detail_enabled", json!({"enabled":false}))
        .unwrap();
    assert!(cancelled("q-detail"));
    assert!(!cancelled("q-content"));
}

#[test]
fn quality_backup_checks_typed_scalar_results_and_migrates_old_catalogs() {
    let f = Fixture::new();
    f.picture("backup-face.jpg", 3);
    f.add();
    let a = f.assets();
    let aid = a[0]["id"].as_str().unwrap();
    let c = f.cat.connection().unwrap();
    // Valid JSON is insufficient: a restored result must satisfy the known DTO.
    c.execute("INSERT INTO quality_observations SELECT id,'eyes','test',byte_size,mtime_ms,COALESCE(orientation,1),?,'{}',0 FROM media_assets WHERE id=?",rusqlite::params!["a".repeat(64),aid]).unwrap();
    let backup = f.output.join("bad-quality.sqlite");
    db::backup(&c, &backup).unwrap();
    f.cat.authorize(&backup, "restore").unwrap();
    assert_eq!(
        f.cat
            .call("prepare_restore", json!({"selectedBackupPath":backup}))
            .unwrap_err()
            .code,
        "DB_MIGRATION_FAILED"
    );
    c.execute("DELETE FROM quality_observations", []).unwrap();
    c.execute_batch(
        "DROP TABLE quality_hint_reviews;DROP TABLE quality_observations;
         ALTER TABLE media_assets DROP COLUMN file_version;
         ALTER TABLE media_assets DROP COLUMN sha256_file_version;
         PRAGMA user_version=7;",
    )
    .unwrap();
    drop(c);
    drop(f.cat);
    db::initialize(&f._temp.path().join("app/catalog.sqlite")).unwrap();
    let c = db::connect(&f._temp.path().join("app/catalog.sqlite")).unwrap();
    travel_gallery::quality::validate_backup(&c).unwrap();
    assert_eq!(
        c.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        db::CATALOG_VERSION
    );
}
