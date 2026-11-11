use crate::{
    catalog::Catalog,
    db,
    error::{required, AppError, Result},
    files, id, media, now, query, similarity_index::CandidateIndex,
};
use globset::{Glob, GlobSetBuilder};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use walkdir::WalkDir;

#[derive(Default)]
pub(crate) struct WorkerState {
    registered: HashSet<String>,
    active: HashSet<String>,
}
struct WorkerGuard {
    cat: Catalog,
    jid: String,
}
impl WorkerGuard {
    fn new(cat: &Catalog, jid: &str) -> Self {
        cat.inner
            .workers
            .0
            .lock()
            .unwrap()
            .registered
            .insert(jid.into());
        Self {
            cat: cat.clone(),
            jid: jid.into(),
        }
    }
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let (lock, cond) = &self.cat.inner.workers;
        let mut workers = lock.lock().unwrap();
        workers.active.remove(&self.jid);
        workers.registered.remove(&self.jid);
        cond.notify_all();
    }
}
fn release_slot(cat: &Catalog, jid: &str) {
    let (lock, cond) = &cat.inner.workers;
    if lock.lock().unwrap().active.remove(jid) {
        cond.notify_all();
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceOptions {
    pub recursive: bool,
    pub skip_hidden: bool,
    pub exclude_globs: Vec<String>,
    pub follow_symlinks: bool,
    #[serde(default)]
    pub max_depth: Option<usize>,
}
impl SourceOptions {
    pub fn parse(v: &Value) -> Result<Self> {
        let o: Self = serde_json::from_value(v.clone())?;
        if o.follow_symlinks || o.exclude_globs.len() > 100 || o.max_depth.is_some_and(|d| d > 100)
        {
            return Err(AppError::new(
                "INVALID_FILTER",
                "扫描参数无效，当前版本禁止跟随符号链接",
            ));
        }
        for g in &o.exclude_globs {
            Glob::new(g).map_err(|_| AppError::new("INVALID_FILTER", "排除规则无效"))?;
        }
        Ok(o)
    }
}
pub fn insert(c: &Connection, jid: &str, kind: &str, r: &Value) -> Result<()> {
    let inserted=c.execute("INSERT INTO jobs(id,type,status,checkpoint_json,created_at,updated_at) SELECT ?,?,'queued',?,?,? WHERE NOT EXISTS(SELECT 1 FROM jobs WHERE type=? AND status IN ('queued','running','paused') AND (? IN ('HASH_EXACT','HASH_SIMILAR') OR (?='SCAN' AND json_extract(checkpoint_json,'$.sourceId')=?)))",params![jid,kind,r.to_string(),now(),now(),kind,kind,kind,r["sourceId"].as_str()])?;
    if inserted == 0 {
        return Err(AppError::new(
            "DB_CONFLICT",
            "相同范围已有此类任务，请等待完成或先取消",
        ));
    }
    Ok(())
}
impl Catalog {
    pub(crate) fn queue(&self, c: &Connection, kind: &str, r: Value) -> Result<Value> {
        let jid = id();
        insert(c, &jid, kind, &r)?;
        self.spawn_job(jid.clone(), kind.into(), r);
        Ok(json!({"jobId":jid}))
    }
    pub(crate) fn spawn_job(&self, jid: String, kind: String, r: Value) {
        let cat = self.clone();
        std::thread::spawn(move || {
            let _worker = WorkerGuard::new(&cat, &jid);
            let _gate = cat.inner.maintenance.read().unwrap();
            let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||->Result<()>{safe_point(&cat,&jid)?;let c=cat.connection()?;c.execute("UPDATE jobs SET status='running',updated_at=? WHERE id=? AND status='queued'",params![now(),jid])?;
            match kind.as_str(){"SCAN"=>scan(&cat,&jid,&r),"THUMBNAIL"|"METADATA"=>thumbnails(&cat,&jid,&r),"HASH_EXACT"=>exact(&cat,&jid,&r),"HASH_SIMILAR"=>similar(&cat,&jid,&r),"EXPORT" if r["operation"]=="storyExport"=>crate::story::run(&cat,&jid,&r),"EXPORT"=>files::export(&cat,&jid,&r),"BACKUP"=>files::backup(&cat,&jid,&r),"REPAIR" if r["operation"]=="aiImportModel" || r["operation"]=="eyesImportModel"=>crate::ai_models::import(&cat,&jid,&r),"REPAIR" if r["operation"]=="eyesRemoveModel"=>crate::ai_models::remove_for(&cat,&jid,crate::ai_models::ModelKind::Eyes),"REPAIR" if r["operation"]=="qualityAnalyze"=>crate::quality::run(&cat,&jid,&r),"REPAIR" if r["operation"]=="aiRemoveModel"=>crate::ai_models::remove(&cat,&jid),"REPAIR" if r["operation"]=="aiIndex" || r["operation"]=="aiCluster"=>crate::local_ai::run(&cat,&jid,&r),"REPAIR" if r["operation"]=="renamePreview"=>crate::rename_preview::run(&cat,&jid,&r),_=>Err(AppError::new("INTERNAL","无法执行此任务"))}
        })).unwrap_or_else(|_|Err(AppError::new("INTERNAL","处理此媒体时发生异常，可重试")));
            if let Ok(c) = cat.connection() {
                match outcome {
                    Ok(()) => {
                        let _=c.execute("UPDATE jobs SET status=CASE WHEN status='cancelled' THEN 'cancelled' ELSE 'completed' END,updated_at=? WHERE id=?",params![now(),jid]);
                    }
                    Err(e) => {
                        let status = if e.code == "CANCELLED" {
                            "cancelled"
                        } else {
                            "failed"
                        };
                        let _ = c.execute(
                            "UPDATE jobs SET status=?,error_summary=?,updated_at=? WHERE id=?",
                            params![status, e.message, now(), jid],
                        );
                    }
                }
                let _ = cat.trim_cache(&c);
            }
        });
    }
}
pub fn safe_point(cat: &Catalog, jid: &str) -> Result<()> {
    loop {
        let c = cat.connection()?;
        let (status,cancel,kind,ai_heavy):(String,bool,String,bool)=c.query_row("SELECT status,COALESCE(json_extract(checkpoint_json,'$.cancelRequested'),0),type,COALESCE(json_extract(checkpoint_json,'$.operation') IN ('aiIndex','aiCluster','qualityAnalyze'),0) FROM jobs WHERE id=?",[jid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        if cancel {
            c.execute(
                "UPDATE jobs SET status='cancelled',updated_at=? WHERE id=?",
                params![now(), jid],
            )?;
            return Err(AppError::new(
                "CANCELLED",
                "任务已在安全点取消，已完成项保留",
            ));
        }
        match status.as_str() {
            "cancelled" => return Err(AppError::new("CANCELLED", "任务已取消，已完成项保留")),
            "paused" => {
                release_slot(cat, jid);
                std::thread::sleep(Duration::from_millis(120));
            }
            _ if (ai_heavy || matches!(kind.as_str(), "HASH_EXACT" | "HASH_SIMILAR"))
                && media::foreground_busy(cat) =>
            {
                release_slot(cat, jid);
                std::thread::sleep(Duration::from_millis(25))
            }
            _ => {
                let heavy = ai_heavy
                    || matches!(
                        kind.as_str(),
                        "THUMBNAIL" | "METADATA" | "HASH_EXACT" | "HASH_SIMILAR"
                    );
                let pause = heavy
                    && db::setting(&c, "preferences")?
                        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                        .is_some_and(|v| v["pauseOnBattery"] == true)
                    && cat.on_battery() == Some(true);
                if pause {
                    c.execute("UPDATE jobs SET checkpoint_json=json_set(checkpoint_json,'$.autoPauseReason','battery'),updated_at=? WHERE id=? AND json_extract(checkpoint_json,'$.autoPauseReason') IS NOT 'battery'",params![now(),jid])?;
                    release_slot(cat, jid);
                    drop(c);
                    std::thread::sleep(Duration::from_millis(120));
                } else {
                    c.execute("UPDATE jobs SET checkpoint_json=json_remove(checkpoint_json,'$.autoPauseReason'),updated_at=? WHERE id=? AND json_extract(checkpoint_json,'$.autoPauseReason') IS NOT NULL",params![now(),jid])?;
                    let limit = db::setting(&c, "preferences")?
                        .and_then(|value| serde_json::from_str::<Value>(&value).ok())
                        .and_then(|value| value["scanConcurrency"].as_u64())
                        .unwrap_or(2)
                        .clamp(1, 4) as usize;
                    let (lock, cond) = &cat.inner.workers;
                    let mut workers = lock.lock().unwrap();
                    if !workers.registered.contains(jid) || workers.active.contains(jid) {
                        return Ok(());
                    }
                    if workers.active.len() < limit {
                        workers.active.insert(jid.into());
                        return Ok(());
                    }
                    drop(c);
                    drop(
                        cond.wait_timeout(workers, Duration::from_millis(120))
                            .unwrap(),
                    );
                }
            }
        }
    }
}
pub fn progress(
    c: &Connection,
    jid: &str,
    done: usize,
    total: Option<usize>,
    failures: usize,
) -> Result<()> {
    c.execute("UPDATE jobs SET progress_done=?,progress_total=?,total_is_known=?,failure_count=?,updated_at=? WHERE id=?",params![done,total.unwrap_or(0),total.is_some(),failures,now(),jid])?;
    Ok(())
}
pub fn record_error(c: &Connection, jid: &str, aid: Option<&str>, message: &str) -> Result<()> {
    c.execute(
        "INSERT INTO job_errors(job_id,asset_id,message) VALUES(?,?,?)",
        params![jid, aid, message],
    )?;
    Ok(())
}
pub fn job(c: &Connection, jid: &str) -> Result<Value> {
    c.query_row("SELECT id,type,status,progress_done,progress_total,total_is_known,failure_count,error_summary,created_at,updated_at,COALESCE(json_extract(checkpoint_json,'$.cancelRequested'),0),json_extract(checkpoint_json,'$.autoPauseReason'),json_extract(checkpoint_json,'$.operation') FROM jobs WHERE id=?",[jid],job_row).optional()?.ok_or_else(||AppError::new("INVALID_SELECTION","任务不存在"))
}
fn job_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"operation":r.get::<_,Option<String>>(12)?,"autoPauseReason":r.get::<_,Option<String>>(11)?,"cancelRequested":r.get::<_,bool>(10)?,"id":r.get::<_,String>(0)?,"type":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,"progressDone":r.get::<_,i64>(3)?,"progressTotal":if r.get::<_,bool>(5)?{Some(r.get::<_,i64>(4)?)}else{None},"failureCount":r.get::<_,i64>(6)?,"errorSummary":r.get::<_,Option<String>>(7)?,"createdAt":r.get::<_,i64>(8)?,"updatedAt":r.get::<_,i64>(9)?}),
    )
}
pub fn list(c: &Connection, r: &Value) -> Result<Value> {
    let size = r["pageSize"].as_i64().unwrap_or(100).clamp(1, 500);
    let offset = r["cursor"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
        .max(0);
    let total: i64 = c.query_row("SELECT count(*) FROM jobs", [], |r| r.get(0))?;
    let mut s=c.prepare("SELECT id,type,status,progress_done,progress_total,total_is_known,failure_count,error_summary,created_at,updated_at,COALESCE(json_extract(checkpoint_json,'$.cancelRequested'),0),json_extract(checkpoint_json,'$.autoPauseReason'),json_extract(checkpoint_json,'$.operation') FROM jobs ORDER BY created_at DESC,id DESC LIMIT ? OFFSET ?")?;
    let rows = s
        .query_map(params![size, offset], job_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(
        json!({"items":rows,"total":total,"nextCursor":if offset+size<total{Some((offset+size).to_string())}else{None},"queryFingerprint":"jobs"}),
    )
}
pub fn control(c: &Connection, name: &str, r: &Value) -> Result<Value> {
    let jid = required(r, "jobId")?;
    let old = job(c, jid)?;
    let status = old["status"].as_str().unwrap();
    if name == "cancel_job" && ["queued", "running", "paused"].contains(&status) {
        c.execute("UPDATE jobs SET checkpoint_json=json_set(COALESCE(checkpoint_json,'{}'),'$.cancelRequested',1),updated_at=? WHERE id=?",params![now(),jid])?;
        return Ok(json!({"accepted":true}));
    }
    let next = match name {
        "cancel_job" if ["queued", "running", "paused"].contains(&status) => "cancelled",
        "pause_job" if ["queued", "running"].contains(&status) => "paused",
        "resume_job" if status == "paused" => "running",
        _ => {
            return Err(AppError::new(
                "JOB_NOT_CANCELLABLE",
                "此任务的当前状态不支持该操作",
            ))
        }
    };
    c.execute(
        "UPDATE jobs SET status=?,updated_at=? WHERE id=?",
        params![next, now(), jid],
    )?;
    Ok(json!({"accepted":true}))
}
fn scan(cat: &Catalog, jid: &str, r: &Value) -> Result<()> {
    let sid = required(r, "sourceId")?;
    let source_lock = cat
        .inner
        .source_locks
        .lock()
        .unwrap()
        .entry(sid.to_owned())
        .or_default()
        .clone();
    let _source_guard = source_lock.lock().unwrap();
    safe_point(cat, jid)?;
    let mut c = cat.connection()?;
    let root: String = c.query_row(
        "SELECT absolute_path FROM source_roots WHERE id=?",
        [sid],
        |r| r.get(0),
    )?;
    let root = Path::new(&root);
    if let Err(error) = files::check_source_access(root) {
        if c.execute("UPDATE source_roots SET is_online=0 WHERE id=? AND is_online<>0", [sid])? > 0 {
            db::bump(&c)?;
        }
        return Err(error);
    }
    let opts: String = c
        .query_row(
            "SELECT options_json FROM source_options WHERE source_id=?",
            [sid],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(
            json!({"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false})
                .to_string(),
        );
    let opts = SourceOptions::parse(&serde_json::from_str(&opts)?)?;
    let mut gb = GlobSetBuilder::new();
    for g in &opts.exclude_globs {
        gb.add(Glob::new(g).map_err(|_| AppError::new("INVALID_FILTER", "排除规则无效"))?);
    }
    let globs = gb
        .build()
        .map_err(|_| AppError::new("INVALID_FILTER", "排除规则无效"))?;
    c.execute(
        "UPDATE source_roots SET scan_generation=scan_generation+1,is_online=1 WHERE id=?",
        [sid],
    )?;
    let generation: i64 = c.query_row(
        "SELECT scan_generation FROM source_roots WHERE id=?",
        [sid],
        |r| r.get(0),
    )?;
    let max = if opts.recursive {
        opts.max_depth.unwrap_or(100)
    } else {
        1
    };
    let walker = WalkDir::new(root)
        .follow_links(false)
        .max_depth(max)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || (!e.file_type().is_symlink()
                    && (!opts.skip_hidden || !e.file_name().to_string_lossy().starts_with('.'))
                    && !globs.is_match(e.path().strip_prefix(root).unwrap_or(e.path())))
        });
    let (mut done, mut failures) = (0, 0);
    let mut last = Instant::now();
    let mut complete = true;
    for entry in walker {
        safe_point(cat, jid)?;
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                complete = false;
                failures += 1;
                record_error(
                    &c,
                    jid,
                    None,
                    "部分目录无法读取；本轮不会判定未发现文件为缺失",
                )?;
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let Some(kind) = media::kind(&ext) else {
            continue;
        };
        let rel = path
            .strip_prefix(root)
            .map_err(|_| AppError::new("SOURCE_PATH_INVALID", "目录边界无效"))?
            .to_string_lossy()
            .into_owned();
        let rel = if cfg!(windows) {
            rel.replace('\\', "/")
        } else {
            rel
        };
        let file_stamp = match files::stamp(path) {
            Ok(s) => s,
            Err(_) => {
                complete = false;
                failures += 1;
                continue;
            }
        };
        let identity = files::identity(path);
        let mut old:Option<(String,i64,Option<i64>,i64)>=c.query_row("SELECT id,byte_size,mtime_ms,CAST(COALESCE(json_extract(CASE WHEN json_valid(metadata_json) THEN metadata_json ELSE '{}' END,'$._version'),0) AS INTEGER) FROM media_assets WHERE source_id=? AND relative_path=?",params![sid,rel],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        if old.is_none() {
            if let Some(identity) = &identity {
                let candidate:Option<(String,String,Option<String>,i64)>=c.query_row("SELECT id,relative_path,sha256,CAST(COALESCE(json_extract(CASE WHEN json_valid(metadata_json) THEN metadata_json ELSE '{}' END,'$._version'),0) AS INTEGER) FROM media_assets WHERE source_id=? AND file_identity=? AND byte_size=? AND mtime_ms=? LIMIT 1",params![sid,identity,file_stamp.0,file_stamp.1],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
                if let Some((candidate, previous, Some(hash), version)) = candidate {
                    if !root.join(previous).exists()
                        && files::sha256(path, Some((cat, jid)))? == hash
                    {
                        c.execute(
                            "UPDATE media_assets SET relative_path=?,filename=? WHERE id=?",
                            params![rel, path.file_name().unwrap().to_string_lossy(), candidate],
                        )?;
                        old = Some((candidate, file_stamp.0 as i64, Some(file_stamp.1), version));
                        db::bump(&c)?;
                    }
                }
            }
        }
        if let Some((aid, size, mtime, version)) = &old {
            if *size == file_stamp.0 as i64
                && *mtime == Some(file_stamp.1)
                && media::metadata_version_current(&ext, *version)
            {
                c.execute("UPDATE media_assets SET last_seen_scan_generation=?,file_state=CASE WHEN file_state IN ('missing','offline') THEN 'available' ELSE file_state END WHERE id=?",params![generation,aid])?;
                c.execute(
                    "UPDATE media_assets SET file_identity=? WHERE id=?",
                    params![identity, aid],
                )?;
                done += 1;
                if last.elapsed() > Duration::from_millis(150) {
                    progress(&c, jid, done, None, failures)?;
                    last = Instant::now();
                }
                continue;
            }
        }
        let aid = old.as_ref().map(|o| o.0.clone()).unwrap_or_else(id);
        let meta = media::metadata(path, &ext, kind, file_stamp.1);
        if files::stamp(path).ok() != Some(file_stamp) {
            failures += 1;
            complete = false;
            continue;
        }
        let tx = c.transaction()?;
        if old.is_some() {
            invalidate_indexed_file(&tx, &aid)?;
        }
        IndexedFile {
            aid: &aid,
            sid,
            rel: &rel,
            path,
            ext: &ext,
            kind,
            stamp: file_stamp,
            identity: identity.as_deref(),
            generation,
        }
        .save(&tx, &meta)?;
        db::bump(&tx)?;
        tx.commit()?;
        if done < 18 {
            let _ = media::thumbnail(cat, &mut c, &aid, "small");
        }
        done += 1;
        if meta.state == "corrupt" {
            failures += 1;
            record_error(
                &c,
                jid,
                Some(&aid),
                "媒体损坏；已保留索引，可参与字节级查重",
            )?;
        }
        if last.elapsed() > Duration::from_millis(150) {
            progress(&c, jid, done, None, failures)?;
            last = Instant::now();
        }
    }
    safe_point(cat, jid)?;
    files::check_source_access(root)?;
    let tx = c.transaction()?;
    if complete {
        tx.execute("UPDATE media_assets SET file_state='missing' WHERE source_id=? AND last_seen_scan_generation<>?",params![sid,generation])?;
        tx.execute("DELETE FROM similar_groups WHERE id IN(SELECT gi.group_id FROM similar_group_items gi JOIN media_assets a ON a.id=gi.asset_id WHERE a.source_id=? AND a.file_state='missing')",[sid])?;
    }
    tx.execute(
        "UPDATE source_roots SET last_scan_at=?,last_seen_at=?,is_online=1 WHERE id=?",
        params![now(), now(), sid],
    )?;
    db::bump(&tx)?;
    tx.commit()?;
    progress(&c, jid, done, Some(done), failures)?;
    auto_pairs(&c, sid)?;
    cat.queue(&c, "THUMBNAIL", json!({"sourceId":sid}))?;
    Ok(())
}
// Keep exact groups discoverable after a source scan clears the hash. The
// group query detects the missing hash and schedules a fresh byte comparison.
fn invalidate_indexed_file(c: &Connection, aid: &str) -> Result<()> {
    c.execute("DELETE FROM linked_media WHERE match_method NOT IN ('manual','manual_exclusion') AND (primary_asset_id=? OR secondary_asset_id=?)", params![aid,aid])?;
    c.execute("DELETE FROM thumbnail_cache WHERE asset_id=?", [aid])?;
    c.execute("DELETE FROM similar_groups WHERE group_type='similar' AND id IN(SELECT group_id FROM similar_group_items WHERE asset_id=?)",[aid])?;
    Ok(())
}

struct IndexedFile<'a> {
    aid: &'a str,
    sid: &'a str,
    rel: &'a str,
    path: &'a Path,
    ext: &'a str,
    kind: &'a str,
    stamp: (u64, i64),
    identity: Option<&'a str>,
    generation: i64,
}
impl IndexedFile<'_> {
    fn save(&self, c: &Connection, meta: &media::Metadata) -> Result<()> {
        let v = &meta.values;
        c.execute(
            "INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,
              byte_size,mtime_ms,capture_local,capture_utc_ms,capture_tz_min,capture_time_source,
              width,height,orientation,duration_ms,codec,camera_make,camera_model,gps_lat,gps_lon,
              file_state,metadata_json,indexed_at,updated_at,last_seen_scan_generation,file_identity)
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
             ON CONFLICT(source_id,relative_path) DO UPDATE SET
              byte_size=excluded.byte_size,mtime_ms=excluded.mtime_ms,capture_local=excluded.capture_local,
              capture_utc_ms=excluded.capture_utc_ms,capture_tz_min=excluded.capture_tz_min,
              capture_time_source=excluded.capture_time_source,width=excluded.width,height=excluded.height,
              orientation=excluded.orientation,duration_ms=excluded.duration_ms,codec=excluded.codec,
              camera_make=excluded.camera_make,camera_model=excluded.camera_model,gps_lat=excluded.gps_lat,
              gps_lon=excluded.gps_lon,file_state=excluded.file_state,metadata_json=excluded.metadata_json,
              updated_at=excluded.updated_at,last_seen_scan_generation=excluded.last_seen_scan_generation,
              file_identity=excluded.file_identity,sha256=NULL,sha256_mtime_ms=NULL,sha256_byte_size=NULL,
              perceptual_hash=NULL",
            params![self.aid,self.sid,self.rel,self.path.file_name().unwrap().to_string_lossy(),
                self.ext,self.kind,self.stamp.0,self.stamp.1,v["captureLocal"].as_str(),
                v["captureUtcMs"].as_i64(),v["captureTzMin"].as_i64(),v["captureTimeSource"].as_str(),
                v["width"].as_i64(),v["height"].as_i64(),v["orientation"].as_i64(),v["durationMs"].as_i64(),
                v["codec"].as_str(),v["cameraMake"].as_str(),v["cameraModel"].as_str(),v["gpsLat"].as_f64(),
                v["gpsLon"].as_f64(),meta.state,v["metadata"].to_string(),now(),now(),self.generation,self.identity],
        )?;
        crate::capture_time::refresh_original(c, self.aid, v)?;
        Ok(())
    }
}

fn auto_pairs(c: &Connection, sid: &str) -> Result<()> {
    let tx = c.unchecked_transaction()?;
    refresh_pairs(&tx, sid)?;
    db::bump(&tx)?;
    tx.commit()?;
    Ok(())
}

fn refresh_pairs(c: &Connection, sid: &str) -> Result<()> {
    c.execute(
        "DELETE FROM linked_media WHERE match_method='apple_content_identifier'",
        [],
    )?;
    let mut stems: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
    {
        let mut st=c.prepare("SELECT id,relative_path,extension,media_kind FROM media_assets WHERE source_id=? AND file_state NOT IN ('missing','offline')")?;
        let rows = st.query_map([sid], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (aid, path, ext, kind) = row?;
            let key = Path::new(&path)
                .with_extension("")
                .to_string_lossy()
                .to_lowercase();
            stems.entry(key).or_default().push((aid, ext, kind));
        }
    }
    let mut identifiers: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();
    {
        // Content identity can bridge separate authorized source roots. Keep
        // offline indexed members so disconnecting a disk does not unpair it.
        let mut st=c.prepare("SELECT id,extension,media_kind,json_extract(metadata_json,'$.contentIdentifier') FROM media_assets WHERE json_valid(metadata_json) AND json_type(metadata_json,'$.contentIdentifier')='text'")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (aid, ext, kind, raw) = row?;
            let Ok(identifier) = uuid::Uuid::parse_str(&raw) else {
                continue;
            };
            let pair = identifiers.entry(identifier.to_string()).or_default();
            if kind == "video" && ext == "mov" {
                pair.1.push(aid);
            } else if ["heic", "heif", "jpg", "jpeg"].contains(&ext.as_str()) {
                pair.0.push(aid);
            }
        }
    }
    // Only a unique still/movie match is trusted. Duplicate identifiers remain
    // ambiguous; a basename alone never promotes a link to confirmed.
    for (images, movies) in identifiers.values() {
        if images.len() == 1 && movies.len() == 1 {
            c.execute("INSERT INTO linked_media VALUES(?,?,?,?,?,0.99,1) ON CONFLICT(primary_asset_id,secondary_asset_id,link_type) DO UPDATE SET match_method=CASE WHEN match_method='manual' THEN match_method ELSE excluded.match_method END,confidence=CASE WHEN match_method='manual' THEN confidence ELSE 0.99 END,confirmed=1 WHERE match_method<>'manual_exclusion'",
                params![id(),images[0],movies[0],"live_photo","apple_content_identifier"])?;
        }
    }
    for group in stems.values() {
        for (a, ext, _) in group {
            for (b, other, kind) in group {
                if a == b {
                    continue;
                }
                let link = if media::is_raw(ext) && ["jpg", "jpeg"].contains(&other.as_str()) {
                    Some(("raw_jpeg", "same_stem", 0.8))
                } else if ["heic", "heif", "jpg", "jpeg"].contains(&ext.as_str())
                    && kind == "video"
                    && other == "mov"
                {
                    Some(("live_photo", "name_candidate_requires_confirmation", 0.3))
                } else {
                    None
                };
                if let Some((t, m, confidence)) = link {
                    c.execute(
                        "INSERT OR IGNORE INTO linked_media VALUES(?,?,?,?,?,?,0)",
                        params![id(), a, b, t, m, confidence],
                    )?;
                }
            }
        }
    }
    Ok(())
}
fn thumbnails(cat: &Catalog, jid: &str, r: &Value) -> Result<()> {
    let mut c = cat.connection()?;
    let mut st=c.prepare("SELECT id FROM media_assets WHERE source_id=? AND file_state NOT IN ('missing','offline','corrupt')")?;
    let ids = st
        .query_map([required(r, "sourceId")?], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(st);
    let mut failures = 0;
    for (n, aid) in ids.iter().enumerate() {
        safe_point(cat, jid)?;
        if let Err(e) = media::thumbnail(cat, &mut c, aid, "small") {
            failures += 1;
            record_error(&c, jid, Some(aid), &e.message)?;
        }
        if n % 8 == 0 {
            progress(&c, jid, n + 1, Some(ids.len()), failures)?;
        }
    }
    progress(&c, jid, ids.len(), Some(ids.len()), failures)?;
    Ok(())
}
fn exact(cat: &Catalog, jid: &str, r: &Value) -> Result<()> {
    let mut c = cat.connection()?;
    if r["operation"] == "recomputeExact" {
        return recompute_exact(cat, &mut c, jid, r);
    }
    let ids = query::target_ids(&c, &r["scope"])?;
    let mut buckets: HashMap<i64, Vec<String>> = HashMap::new();
    for aid in &ids {
        let a = query::asset(&c, aid)?;
        if files::source_path(&c, aid).is_ok() {
            buckets
                .entry(a["byteSize"].as_i64().unwrap_or(0))
                .or_default()
                .push(aid.clone());
        }
    }
    let candidates: Vec<String> = buckets
        .into_values()
        .filter(|v| v.len() > 1)
        .flatten()
        .collect();
    let mut hashes: HashMap<String, Vec<String>> = HashMap::new();
    let mut verified = Vec::new();
    let mut failures = 0;
    for (n, aid) in candidates.iter().enumerate() {
        safe_point(cat, jid)?;
        let calculated = files::source_path(&c, aid).and_then(|path| {
            let hash = files::sha256(&path, Some((cat, jid)))?;
            let (size, mtime) = files::stamp(&path)?;
            Ok((hash, size, mtime))
        });
        match calculated {
            Ok((hash, size, mtime)) => {
                verified.push((aid, hash.clone(), size, mtime));
                hashes.entry(hash).or_default().push(aid.clone());
            }
            Err(e) => {
                if e.code == "CANCELLED" {
                    return Err(e);
                }
                failures += 1;
                record_error(&c, jid, Some(aid), &e.message)?;
            }
        }
        if n % 8 == 0 {
            progress(&c, jid, n + 1, Some(candidates.len()), failures)?;
        }
    }
    let tx = c.transaction()?;
    for (aid, hash, size, mtime) in verified {
        store_exact_hash(&tx, aid, &hash, (size, mtime))?;
    }
    let mut decisions = clear_scoped_groups(&tx, "exact", "SHA-256", &ids)?;
    for members in hashes.into_values().filter(|v| v.len() > 1) {
        let members = members.into_iter().map(|aid| (aid, None)).collect::<Vec<_>>();
        insert_group(&tx, "exact", "SHA-256", &members, &mut decisions)?;
    }
    db::bump(&tx)?;
    tx.commit()?;
    progress(&c, jid, candidates.len(), Some(candidates.len()), failures)?;
    Ok(())
}

fn store_exact_hash(c: &Connection, aid: &str, hash: &str, (size, mtime): (u64, i64)) -> Result<()> {
    c.execute(
        "UPDATE media_assets SET sha256=?,sha256_byte_size=?,sha256_mtime_ms=? WHERE id=?",
        params![hash, size, mtime, aid],
    )?;
    Ok(())
}

fn exact_group_ids(c: &Connection, gid: &str) -> Result<Vec<String>> {
    let mut st = c.prepare(
        "SELECT gi.asset_id FROM similar_group_items gi JOIN similar_groups g ON g.id=gi.group_id
         WHERE g.id=? AND g.group_type='exact' ORDER BY gi.asset_id",
    )?;
    let ids = st
        .query_map([gid], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(ids)
}

struct ExactMember {
    aid: String,
    path: PathBuf,
    stamp: (u64, i64),
    metadata: Option<media::Metadata>,
    hash: Option<String>,
}

fn recompute_exact(cat: &Catalog, c: &mut Connection, jid: &str, r: &Value) -> Result<()> {
    let groups: Vec<String> = serde_json::from_value(r["groupIds"].clone())?;
    let mut failures = 0;
    progress(c, jid, 0, Some(groups.len()), failures)?;
    for (i, gid) in groups.iter().enumerate() {
        safe_point(cat, jid)?;
        if let Err(error) = recompute_exact_group(cat, c, jid, gid) {
            if error.code == "CANCELLED" {
                return Err(error);
            }
            failures += 1;
            record_error(c, jid, None, &error.message)?;
        }
        progress(c, jid, i + 1, Some(groups.len()), failures)?;
    }
    if failures > 0 {
        return Err(AppError::new(
            "DB_CONFLICT",
            "部分精确组无法重算。请检查来源和任务详情后重试",
        ));
    }
    Ok(())
}

fn exact_change_conflict() -> AppError {
    AppError::new("DB_CONFLICT", "重算期间分组或文件发生变化，请重试")
}

fn exact_stamp(path: &Path) -> Result<(u64, i64)> {
    if !path.is_file() {
        return Err(AppError::new("ASSET_NOT_FOUND", "原文件缺失或不是普通文件"));
    }
    files::stamp(path)
}

fn recompute_exact_group(cat: &Catalog, c: &mut Connection, jid: &str, gid: &str) -> Result<()> {
    let ids = exact_group_ids(c, gid)?;
    if ids.is_empty() {
        // A manual exact scan may have superseded this queued result.
        return Ok(());
    }
    let mut buckets = HashMap::<u64, usize>::new();
    let mut members = Vec::with_capacity(ids.len());
    for aid in &ids {
        safe_point(cat, jid)?;
        let path = files::source_path(c, aid)?;
        let stamp = exact_stamp(&path)?;
        let (size, mtime, ext, kind) = c.query_row(
            "SELECT byte_size,mtime_ms,extension,media_kind FROM media_assets WHERE id=?",
            [aid],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )?;
        let metadata = ((size, mtime) != (stamp.0, Some(stamp.1)))
            .then(|| media::metadata(&path, &ext, &kind, stamp.1));
        if exact_stamp(&path)? != stamp {
            return Err(exact_change_conflict());
        }
        *buckets.entry(stamp.0).or_default() += 1;
        members.push(ExactMember {
            aid: aid.clone(),
            path,
            stamp,
            metadata,
            hash: None,
        });
    }
    let mut hashes = HashMap::<String, Vec<(String, Option<u32>)>>::new();
    for member in &mut members {
        safe_point(cat, jid)?;
        if buckets[&member.stamp.0] > 1 {
            if exact_stamp(&member.path)? != member.stamp {
                return Err(exact_change_conflict());
            }
            let hash = files::sha256(&member.path, Some((cat, jid)))?;
            if exact_stamp(&member.path)? != member.stamp {
                return Err(exact_change_conflict());
            }
            hashes
                .entry(hash.clone())
                .or_default()
                .push((member.aid.clone(), None));
            member.hash = Some(hash);
        }
    }

    safe_point(cat, jid)?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if exact_group_ids(&tx, gid)? != ids {
        return Err(exact_change_conflict());
    }
    let mut changed_sources = HashSet::new();
    for member in members {
        if files::source_path(&tx, &member.aid)? != member.path
            || exact_stamp(&member.path)? != member.stamp
        {
            return Err(exact_change_conflict());
        }
        let (sid, rel, ext, kind, size, mtime, generation) = tx.query_row(
            "SELECT source_id,relative_path,extension,media_kind,byte_size,mtime_ms,
             last_seen_scan_generation FROM media_assets WHERE id=?",
            [&member.aid],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, u64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )?;
        if (size, mtime) != (member.stamp.0, Some(member.stamp.1)) {
            let meta = member.metadata.as_ref().ok_or_else(exact_change_conflict)?;
            let identity = files::identity(&member.path);
            invalidate_indexed_file(&tx, &member.aid)?;
            IndexedFile {
                aid: &member.aid,
                sid: &sid,
                rel: &rel,
                path: Path::new(&rel),
                ext: &ext,
                kind: &kind,
                stamp: member.stamp,
                identity: identity.as_deref(),
                generation,
            }
            .save(&tx, meta)?;
            changed_sources.insert(sid);
        }
        if let Some(hash) = member.hash {
            store_exact_hash(&tx, &member.aid, &hash, member.stamp)?;
        }
    }
    let mut decisions = clear_scoped_groups(&tx, "exact", "SHA-256", &ids)?;
    for members in hashes.into_values().filter(|v| v.len() > 1) {
        insert_group(&tx, "exact", "SHA-256", &members, &mut decisions)?;
    }
    for sid in changed_sources {
        refresh_pairs(&tx, &sid)?;
    }
    db::bump(&tx)?;
    tx.commit()?;
    Ok(())
}
#[derive(Default)]
struct GroupDecision {
    representative: Option<String>,
    excluded: HashSet<String>,
}
type GroupDecisions = HashMap<Vec<String>, GroupDecision>;

fn clear_scoped_groups(
    c: &Connection,
    kind: &str,
    algorithm: &str,
    ids: &[String],
) -> Result<GroupDecisions> {
    c.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS scan_scope(id TEXT PRIMARY KEY); DELETE FROM scan_scope;",
    )?;
    for aid in ids {
        c.execute("INSERT OR IGNORE INTO scan_scope VALUES(?)", [aid])?;
    }
    // Capture only manual decisions in this scope, inside the same transaction
    // that replaces the groups. A different membership or algorithm gets fresh decisions.
    let mut previous = GroupDecisions::new();
    let mut statement = c.prepare("SELECT g.id,g.representative_asset_id FROM similar_groups g WHERE g.group_type=? AND g.algorithm=? AND g.algorithm_version='1' AND (g.representative_asset_id IS NOT NULL OR EXISTS(SELECT 1 FROM similar_group_items WHERE group_id=g.id AND manually_excluded=1)) AND g.id IN(SELECT group_id FROM similar_group_items gi JOIN scan_scope s ON s.id=gi.asset_id)")?;
    let groups = statement.query_map(params![kind, algorithm], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    for group in groups {
        let (gid, representative) = group?;
        let mut items = c.prepare("SELECT asset_id,manually_excluded FROM similar_group_items WHERE group_id=? ORDER BY asset_id")?;
        let mut members = Vec::new();
        let mut excluded = HashSet::new();
        for item in items.query_map([gid], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
        })? {
            let (aid, is_excluded) = item?;
            if is_excluded {
                excluded.insert(aid.clone());
            }
            members.push(aid);
        }
        previous.insert(members, GroupDecision { representative, excluded });
    }
    drop(statement);
    c.execute("DELETE FROM similar_groups WHERE group_type=? AND id IN(SELECT group_id FROM similar_group_items gi JOIN scan_scope s ON s.id=gi.asset_id)",[kind])?;
    Ok(previous)
}

fn insert_group(
    c: &Connection,
    kind: &str,
    algorithm: &str,
    members: &[(String, Option<u32>)],
    decisions: &mut GroupDecisions,
) -> Result<()> {
    let mut key: Vec<_> = members.iter().map(|(aid, _)| aid.clone()).collect();
    key.sort();
    let decision = decisions.remove(&key).unwrap_or_default();
    let representative = decision.representative.filter(|aid| {
        !decision.excluded.contains(aid) && key.binary_search(aid).is_ok()
    });
    let gid = id();
    c.execute("INSERT INTO similar_groups VALUES(?,?,?,'1',?,?)",
        params![gid, kind, algorithm, representative, now()])?;
    for (aid, distance) in members {
        c.execute("INSERT INTO similar_group_items VALUES(?,?,?,?)",
            params![gid, aid, distance, decision.excluded.contains(aid)])?;
    }
    Ok(())
}
pub fn dhash(img: &image::DynamicImage) -> u64 {
    let gray = img
        .resize_exact(9, 8, image::imageops::FilterType::Triangle)
        .to_luma8();
    let mut hash = 0;
    for y in 0..8 {
        for x in 0..8 {
            hash = (hash << 1) | u64::from(gray.get_pixel(x, y)[0] > gray.get_pixel(x + 1, y)[0]);
        }
    }
    hash
}
fn similar(cat: &Catalog, jid: &str, r: &Value) -> Result<()> {
    let mut c = cat.connection()?;
    let ids = query::target_ids(&c, &r["scope"])?;
    let threshold = r["similarity"]["maxHammingDistance"].as_u64().unwrap_or(8);
    let window = r["similarity"]["timeWindowSeconds"].as_i64().unwrap_or(60);
    if threshold > 20 || !(0..=86400).contains(&window) {
        return Err(AppError::new(
            "INVALID_FILTER",
            "相似距离须为 0–20，时间窗口须为 0–86400 秒",
        ));
    }
    let mut entries = vec![];
    let mut failures = 0;
    for (n, aid) in ids.iter().enumerate() {
        safe_point(cat, jid)?;
        let a = query::asset(&c, aid)?;
        if a["mediaKind"] != "image" {
            continue;
        }
        let preview = (|| -> Result<_> {
            let source = files::source_path(&c, aid)?;
            let before = files::stamp(&source)?;
            if Some(before.0 as i64) != a["byteSize"].as_i64()
                || Some(before.1) != a["mtimeMs"].as_i64()
            {
                return Err(AppError::new(
                    "DB_CONFLICT",
                    "媒体已变化，请重新扫描再计算相似度",
                ));
            }
            let path = media::thumbnail(cat, &mut c, aid, "small")?;
            let image = media::decode(&path, 1)?;
            if files::stamp(&source)? != before {
                return Err(AppError::new(
                    "DB_CONFLICT",
                    "媒体在相似度计算期间变化，请重新扫描",
                ));
            }
            Ok((image, before))
        })();
        match preview {
            Ok((img, (size, mtime))) => {
                let h = dhash(&img);
                let t = a["captureUtcMs"]
                    .as_i64()
                    .or(a["mtimeMs"].as_i64())
                    .unwrap_or(0);
                let ratio = img.width() as f64 / img.height() as f64;
                let source = if a["extension"].as_str().is_some_and(media::is_raw) {
                    "raw_embedded_preview"
                } else if a["extension"]
                    .as_str()
                    .is_some_and(|e| ["heic", "heif"].contains(&e))
                {
                    "platform_converted_preview"
                } else {
                    "decoded_image"
                };
                c.execute("UPDATE media_assets SET perceptual_hash=?,perceptual_hash_algorithm='dHash-64-v1',phash_mtime_ms=?,phash_byte_size=?,metadata_json=json_set(metadata_json,'$.perceptualHashSource',?) WHERE id=?",params![format!("{h:016x}"),mtime,size,source,aid])?;
                entries.push((aid.clone(), h, t, ratio));
            }
            Err(e) => {
                failures += 1;
                record_error(&c, jid, Some(aid), &e.message)?;
            }
        }
        if n % 8 == 0 {
            progress(&c, jid, n + 1, Some(ids.len()), failures)?;
        }
    }
    entries.sort_by_key(|e| e.2);
    let mut trees: HashMap<i64, CandidateIndex> = HashMap::new();
    let mut centers: Vec<usize> = vec![];
    let mut clusters: HashMap<usize, Vec<(usize, u32)>> = HashMap::new();
    for (index, (_, hash, time, ratio)) in entries.iter().enumerate() {
        safe_point(cat, jid)?;
        let bucket = if window == 0 {
            0
        } else {
            time.div_euclid(window * 1000)
        };
        trees.retain(|key, _| window == 0 || *key >= bucket - 1);
        let candidate = trees
            .values_mut()
            .flat_map(|tree| tree.search(*hash))
            .into_iter()
            .filter(|i| window == 0 || (*time - entries[*i].2).abs() <= window * 1000)
            .filter(|i| (entries[*i].3 / ratio - 1.).abs() < 0.15)
            .min_by_key(|i| ((entries[*i].1 ^ hash).count_ones(), *i));
        if let Some(center) = candidate {
            clusters
                .entry(center)
                .or_default()
                .push((index, (entries[center].1 ^ hash).count_ones()));
        } else {
            trees
                .entry(bucket)
                .or_insert_with(|| CandidateIndex::new(threshold as u32))
                .add(*hash, index);
            centers.push(index);
        }
    }
    let tx = c.transaction()?;
    let mut decisions = clear_scoped_groups(&tx, "similar", "dHash-64", &ids)?;
    for center in centers {
        if let Some(members) = clusters.get(&center) {
            let members = std::iter::once((entries[center].0.clone(), Some(0)))
                .chain(members.iter().map(|(member, distance)| {
                    (entries[*member].0.clone(), Some(*distance))
                }))
                .collect::<Vec<_>>();
            insert_group(&tx, "similar", "dHash-64", &members, &mut decisions)?;
        }
    }
    db::bump(&tx)?;
    tx.commit()?;
    progress(&c, jid, ids.len(), Some(ids.len()), failures)?;
    Ok(())
}
#[derive(Default)]
struct GroupStaleness {
    changed: bool,
    unavailable: bool,
}

fn group_staleness(c: &Connection, gid: &str, kind: &str, limit: i64) -> Result<GroupStaleness> {
    let mut st = c.prepare(
        "SELECT a.id,a.byte_size,a.mtime_ms,a.sha256 IS NOT NULL,a.sha256_byte_size,a.sha256_mtime_ms
         FROM similar_group_items gi JOIN media_assets a ON a.id=gi.asset_id
         WHERE gi.group_id=? ORDER BY gi.asset_id LIMIT ?",
    )?;
    let rows = st.query_map(params![gid, limit], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, u64>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, bool>(3)?,
            row.get::<_, Option<u64>>(4)?,
            row.get::<_, Option<i64>>(5)?,
        ))
    })?;
    let mut state = GroupStaleness::default();
    for row in rows {
        let (aid, indexed_size, indexed_mtime, hashed, hash_size, hash_mtime) = row?;
        match files::source_path(c, &aid).and_then(|path| exact_stamp(&path)) {
            Ok((size, mtime)) => {
                state.changed |= indexed_size != size || indexed_mtime != Some(mtime);
                if kind == "exact" {
                    state.changed |=
                        !hashed || hash_size != Some(size) || hash_mtime != Some(mtime);
                }
            }
            Err(_) => state.unavailable = true,
        }
    }
    Ok(state)
}

fn queue_exact_recompute(cat: &Catalog, c: &mut Connection, candidates: &[String]) -> Result<()> {
    if candidates.is_empty() {
        return Ok(());
    }
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs WHERE type='HASH_EXACT' AND status IN ('queued','running','paused'))",
        [],
        |row| row.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    // Each stale group ID gets one automatic attempt. A failed or
    // cancelled attempt remains visible and can be retried from the Jobs page.
    let group_ids = {
        let mut st = tx.prepare(
            "SELECT g.id FROM json_each(?) candidate JOIN similar_groups g ON g.id=candidate.value
             WHERE g.group_type='exact' AND NOT EXISTS (
                 SELECT 1 FROM jobs j, json_each(j.checkpoint_json,'$.groupIds') previous
                 WHERE j.type='HASH_EXACT'
                   AND json_extract(j.checkpoint_json,'$.operation')='recomputeExact'
                   AND previous.value=g.id
             ) ORDER BY g.id",
        )?;
        let rows = st.query_map([serde_json::to_string(candidates)?], |row| {
            row.get::<_, String>(0)
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    if group_ids.is_empty() {
        return Ok(());
    }
    let jid = id();
    let request = json!({"operation":"recomputeExact","groupIds":group_ids});
    insert(&tx, &jid, "HASH_EXACT", &request)?;
    tx.commit()?;
    cat.spawn_job(jid, "HASH_EXACT".into(), request);
    Ok(())
}

pub fn groups(cat: &Catalog, c: &mut Connection, r: &Value) -> Result<Value> {
    let kind = required(r, "kind")?;
    if !["exact", "similar"].contains(&kind) {
        return Err(AppError::new("INVALID_FILTER", "分组类型无效"));
    }
    let page = r["pageSize"].as_i64().unwrap_or(30).clamp(1, 100);
    let offset = r["cursor"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
        .max(0);
    let tx = c.transaction()?;
    let total: i64 = tx.query_row(
        "SELECT count(*) FROM similar_groups WHERE group_type=?",
        [kind],
        |r| r.get(0),
    )?;
    let mut st=tx.prepare("SELECT id,algorithm,algorithm_version,representative_asset_id FROM similar_groups WHERE group_type=? ORDER BY created_at DESC,id LIMIT ? OFFSET ?")?;
    let rows = st
        .query_map(params![kind, page, offset], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(st);
    let mut groups = vec![];
    let mut candidates = vec![];
    for (gid, algorithm, version, representative) in rows {
        let mut s=tx.prepare("SELECT asset_id,distance,manually_excluded FROM similar_group_items WHERE group_id=? ORDER BY asset_id LIMIT 12")?;
        let members = s
            .query_map([&gid], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut v = vec![];
        let status = group_staleness(&tx, &gid, kind, if kind == "exact" { -1 } else { 12 })?;
        let stale = status.changed || status.unavailable;
        if kind == "exact" && status.changed && !status.unavailable {
            candidates.push(gid.clone());
        }
        for (aid, distance, excluded) in members {
            let a = query::asset(&tx, &aid)?;
            v.push(json!({"asset":a,"distance":distance,"excluded":excluded}));
        }
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM similar_group_items WHERE group_id=? AND manually_excluded=0",
            [&gid],
            |r| r.get(0),
        )?;
        let all_count: i64 = tx.query_row(
            "SELECT count(*) FROM similar_group_items WHERE group_id=?",
            [&gid],
            |r| r.get(0),
        )?;
        groups.push(json!({"totalMembers":all_count,"id":gid,"kind":kind,"algorithm":algorithm,"algorithmVersion":version,"representativeAssetId":representative,"memberCount":count,"members":v,"stale":stale}));
    }
    tx.commit()?;
    queue_exact_recompute(cat, c, &candidates)?;
    Ok(
        json!({"items":groups,"total":total,"nextCursor":if offset+page<total{Some((offset+page).to_string())}else{None},"queryFingerprint":kind}),
    )
}
pub fn group_action(c: &mut Connection, name: &str, r: &Value) -> Result<Value> {
    let gid = required(r, "groupId")?;
    let aid = required(r, "assetId")?;
    let (excluded, kind) = c.query_row(
        "SELECT gi.manually_excluded,g.group_type FROM similar_group_items gi JOIN similar_groups g ON g.id=gi.group_id WHERE g.id=? AND gi.asset_id=?",
        params![gid, aid], |row| Ok((row.get::<_, bool>(0)?, row.get::<_, String>(1)?)),
    ).optional()?.ok_or_else(|| AppError::new("INVALID_SELECTION", "分组或成员已变化，请刷新后重试"))?;
    if name == "set_group_representative" && excluded {
        return Err(AppError::new("INVALID_SELECTION", "请选择组内有效媒体"));
    }
    let status = group_staleness(c, gid, &kind, -1)?;
    if status.changed || status.unavailable {
        return Err(AppError::new(
            "DB_CONFLICT",
            "分组结果已过期，请刷新分组后重试",
        ));
    }
    // Keep file checks outside the write transaction. Validate the affected
    // row again so a concurrent rebuild cannot turn an old action into success.
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if kind == "exact" && tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM similar_group_items gi JOIN media_assets a ON a.id=gi.asset_id
         WHERE gi.group_id=? AND (a.sha256 IS NULL OR a.sha256_byte_size IS NOT a.byte_size OR a.sha256_mtime_ms IS NOT a.mtime_ms))",
        [gid], |row| row.get::<_, bool>(0),
    )? {
        return Err(AppError::new("DB_CONFLICT", "分组结果已过期，请刷新分组后重试"));
    }
    let changed = if name == "set_group_representative" {
        tx.execute(
            "UPDATE similar_groups SET representative_asset_id=? WHERE id=? AND EXISTS(SELECT 1 FROM similar_group_items WHERE group_id=similar_groups.id AND asset_id=? AND manually_excluded=0)",
            params![aid, gid, aid],
        )?
    } else {
        tx.execute(
            "UPDATE similar_group_items SET manually_excluded=? WHERE group_id=? AND asset_id=?",
            params![r["excluded"].as_bool().unwrap_or(true), gid, aid],
        )?
    };
    if changed != 1 {
        return Err(AppError::new("DB_CONFLICT", "分组已变化，请刷新后重试"));
    }
    if name == "exclude_group_member" {
        tx.execute("UPDATE similar_groups SET representative_asset_id=NULL WHERE id=? AND representative_asset_id=? AND EXISTS(SELECT 1 FROM similar_group_items WHERE group_id=? AND asset_id=? AND manually_excluded=1)",params![gid,aid,gid,aid])?;
    }
    db::bump(&tx)?;
    tx.commit()?;
    Ok(json!({"updated":true}))
}

pub fn group_members(c: &Connection, r: &Value) -> Result<Value> {
    let gid = required(r, "groupId")?;
    let cursor = r["cursor"].as_str().unwrap_or("");
    let limit = r["pageSize"].as_i64().unwrap_or(50).clamp(1, 200);
    let mut st=c.prepare("SELECT asset_id,distance,manually_excluded FROM similar_group_items WHERE group_id=? AND asset_id>? ORDER BY asset_id LIMIT ?")?;
    let mut rows = st
        .query_map(params![gid, cursor, limit + 1], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = if more {
        rows.last().map(|r| r.0.clone())
    } else {
        None
    };
    let mut items = vec![];
    for (id, distance, excluded) in rows {
        items.push(json!({"asset":query::asset(c,&id)?,"distance":distance,"excluded":excluded}));
    }
    let total: i64 = c.query_row(
        "SELECT count(*) FROM similar_group_items WHERE group_id=?",
        [gid],
        |r| r.get(0),
    )?;
    Ok(json!({"items":items,"total":total,"nextCursor":next,"queryFingerprint":gid}))
}

#[cfg(test)]
mod priority_tests {
    use super::*;
    use std::sync::{atomic::Ordering, mpsc};
    #[test]
    fn heavy_hash_yields_to_foreground_but_cancel_is_immediate() {
        let temp = tempfile::tempdir().unwrap();
        let cat = Catalog::new(&temp.path().join("data"), &temp.path().join("cache")).unwrap();
        let c = cat.connection().unwrap();
        insert(&c, "heavy", "HASH_EXACT", &json!({})).unwrap();
        insert(&c, "light", "SCAN", &json!({"sourceId":"s"})).unwrap();
        cat.inner
            .interactive_until
            .store(now() + 5000, Ordering::Relaxed);
        safe_point(&cat, "light").unwrap();
        let worker = cat.clone();
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || tx.send(safe_point(&worker, "heavy")).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        c.execute(
            "UPDATE jobs SET checkpoint_json='{\"cancelRequested\":true}' WHERE id='heavy'",
            [],
        )
        .unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap_err()
                .code,
            "CANCELLED"
        );
        handle.join().unwrap();
    }
    #[test]
    fn battery_gate_resumes_on_ac_and_preserves_manual_pause() {
        let temp = tempfile::tempdir().unwrap();
        let cat = Catalog::new(&temp.path().join("data"), &temp.path().join("cache")).unwrap();
        cat.call("save_settings", json!({"pauseOnBattery":true}))
            .unwrap();
        {
            let mut power = cat.inner.power.lock().unwrap();
            power.value = Some(true);
            power.checked = Instant::now();
        }
        let c = cat.connection().unwrap();
        insert(&c, "battery-work", "THUMBNAIL", &json!({})).unwrap();
        insert(&c, "copy-work", "EXPORT", &json!({})).unwrap();
        safe_point(&cat, "copy-work").unwrap();
        let worker = cat.clone();
        let (tx, rx) = mpsc::channel();
        let handle =
            std::thread::spawn(move || tx.send(safe_point(&worker, "battery-work")).unwrap());
        let start = Instant::now();
        while job(&c, "battery-work").unwrap()["autoPauseReason"] != "battery" {
            assert!(start.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(10));
        }
        c.execute(
            "UPDATE jobs SET status='paused' WHERE id='battery-work'",
            [],
        )
        .unwrap();
        {
            let mut power = cat.inner.power.lock().unwrap();
            power.value = Some(false);
            power.checked = Instant::now();
        }
        assert!(rx.recv_timeout(Duration::from_millis(150)).is_err());
        c.execute(
            "UPDATE jobs SET status='queued' WHERE id='battery-work'",
            [],
        )
        .unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        assert!(job(&c, "battery-work").unwrap()["autoPauseReason"].is_null());
        handle.join().unwrap();
    }
    #[test]
    fn paused_workers_release_slots_for_unrelated_copy_jobs() {
        let temp = tempfile::tempdir().unwrap();
        let cat = Catalog::new(&temp.path().join("data"), &temp.path().join("cache")).unwrap();
        cat.call(
            "save_settings",
            json!({"pauseOnBattery":true,"scanConcurrency":1}),
        )
        .unwrap();
        let c = cat.connection().unwrap();
        insert(&c, "background", "THUMBNAIL", &json!({})).unwrap();
        insert(&c, "copy", "EXPORT", &json!({})).unwrap();
        {
            let mut power = cat.inner.power.lock().unwrap();
            power.value = Some(false);
            power.checked = Instant::now();
        }
        let worker = cat.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (continue_tx, continue_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let _guard = WorkerGuard::new(&worker, "background");
            safe_point(&worker, "background").unwrap();
            started_tx.send(()).unwrap();
            continue_rx.recv().unwrap();
            finished_tx.send(safe_point(&worker, "background")).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(cat.inner.workers.0.lock().unwrap().active.len(), 1);
        {
            let mut power = cat.inner.power.lock().unwrap();
            power.value = Some(true);
            power.checked = Instant::now();
        }
        continue_tx.send(()).unwrap();
        let start = Instant::now();
        while !cat.inner.workers.0.lock().unwrap().active.is_empty() {
            assert!(start.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(10));
        }
        let copy_guard = WorkerGuard::new(&cat, "copy");
        safe_point(&cat, "copy").unwrap();
        assert_eq!(cat.inner.workers.0.lock().unwrap().active.len(), 1);
        assert!(finished_rx.try_recv().is_err());
        {
            let mut power = cat.inner.power.lock().unwrap();
            power.value = Some(false);
            power.checked = Instant::now();
        }
        assert!(finished_rx
            .recv_timeout(Duration::from_millis(150))
            .is_err());
        drop(copy_guard);
        finished_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        handle.join().unwrap();
        assert!(cat.inner.workers.0.lock().unwrap().active.is_empty());
    }
}
