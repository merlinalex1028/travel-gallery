use crate::{
    db,
    error::{required, AppError, Result},
    files, id, media, now, query, review, tasks,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{atomic::AtomicI64, Arc, Condvar, Mutex, RwLock},
};

#[derive(Clone)]
pub struct Catalog {
    pub(crate) inner: Arc<Inner>,
}
pub(crate) struct Inner {
    pub _catalog_lock: std::fs::File,
    pub db_path: PathBuf,
    pub cache: PathBuf,
    pub data: PathBuf,
    pub grants: Mutex<HashMap<PathBuf, (String, i64)>>,
    pub previews: Mutex<HashMap<String, (PathBuf, String)>>,
    pub cache_publish: Mutex<()>,
    pub tokens: Mutex<HashMap<String, Value>>,
    pub workers: (Mutex<tasks::WorkerState>, Condvar),
    pub decoders: (Mutex<media::DecodeState>, Condvar),
    pub source_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    pub maintenance: RwLock<()>,
    pub watcher: Mutex<crate::watcher::WatchControl>,
    pub interactive_until: AtomicI64,
    pub power: Mutex<crate::power::PowerCache>,
    pub ai: crate::ai_runtime::Runtime,
    pub eyes: crate::ai_runtime::Runtime,
}

fn source_overlaps(c: &Connection, selected: &Path) -> Result<Vec<(String, PathBuf, i64)>> {
    let mut roots = c.prepare("SELECT id,absolute_path FROM source_roots ORDER BY id")?;
    let roots = roots.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    let mut overlaps = Vec::new();
    for root in roots {
        let (sid, path) = root?;
        let path = PathBuf::from(path);
        let path = path.canonicalize().unwrap_or(path);
        if selected == path {
            return Err(AppError::new("SOURCE_EXISTS", "此目录已加入资料库")
                .details(json!({"sourceId":sid})));
        }
        let count = if let Ok(relative) = selected.strip_prefix(&path) {
            let relative = relative.to_string_lossy();
            let relative = if cfg!(windows) {
                relative.replace('\\', "/").into()
            } else {
                relative
            };
            let prefix = format!("{relative}/");
            let upper_bound = format!("{relative}0");
            // '/' and '0' are adjacent in binary order.
            c.query_row(
                "SELECT count(*) FROM media_assets WHERE source_id=? AND relative_path>=? AND relative_path<?",
                params![sid, prefix, upper_bound],
                |row| row.get(0),
            )?
        } else if path.starts_with(selected) {
            c.query_row(
                "SELECT count(*) FROM media_assets WHERE source_id=?",
                [&sid],
                |row| row.get(0),
            )?
        } else {
            continue;
        };
        overlaps.push((sid, path, count));
    }
    Ok(overlaps)
}

impl Catalog {
    pub fn new(data: &Path, cache: &Path) -> Result<Self> {
        std::fs::create_dir_all(data)?;
        std::fs::create_dir_all(cache)?;
        let catalog_lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(data.join("catalog.lock"))?;
        fs2::FileExt::try_lock_exclusive(&catalog_lock).map_err(|_| {
            AppError::new(
                "DB_CONFLICT",
                "资料库已在另一个旅图进程中打开，请先关闭另一个实例",
            )
        })?;
        let db_path = data.join("catalog.sqlite");
        db::initialize(&db_path)?;
        let catalog = Self {
            inner: Arc::new(Inner {
                _catalog_lock: catalog_lock,
                db_path,
                cache: cache.into(),
                data: data.into(),
                grants: Mutex::new(HashMap::new()),
                previews: Mutex::new(HashMap::new()),
                cache_publish: Mutex::new(()),
                tokens: Mutex::new(HashMap::new()),
                workers: (Mutex::new(tasks::WorkerState::default()), Condvar::new()),
                decoders: (Mutex::new(media::DecodeState::default()), Condvar::new()),
                source_locks: Mutex::new(HashMap::new()),
                maintenance: RwLock::new(()),
                watcher: Mutex::new(crate::watcher::WatchControl::default()),
                interactive_until: AtomicI64::new(0),
                power: Mutex::new(crate::power::PowerCache::default()),
                ai: crate::ai_runtime::Runtime::default(),
                eyes: crate::ai_runtime::Runtime::eyes(),
            }),
        };
        crate::ai_runtime::watch_idle(&catalog);
        Ok(catalog)
    }
    pub fn connection(&self) -> Result<Connection> {
        db::connect(&self.inner.db_path)
    }
    pub fn startup_scan(&self) -> Result<()> {
        let c = self.connection()?;
        let mut st = c.prepare("SELECT id,absolute_path FROM source_roots WHERE is_enabled=1")?;
        let roots = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (id, path) in roots {
            if files::check_source_access(Path::new(&path)).is_ok() {
                self.queue(&c, "SCAN", json!({"sourceId":id}))?;
            }
        }
        Ok(())
    }
    /// Called by the native picker only; never registered as an IPC command.
    pub fn authorize(&self, path: &Path, purpose: &str) -> Result<PathBuf> {
        let path = path
            .canonicalize()
            .map_err(|_| AppError::new("SOURCE_PATH_INVALID", "无法读取所选路径"))?;
        self.inner
            .grants
            .lock()
            .unwrap()
            .insert(path.clone(), (purpose.into(), now() + 3_600_000));
        Ok(path)
    }
    pub fn authorized(&self, path: &Path, purpose: &str) -> Result<PathBuf> {
        let canonical = path
            .canonicalize()
            .map_err(|_| AppError::new("PERMISSION_DENIED", "所选目录已不可访问，请重新选择"))?;
        let grants = self.inner.grants.lock().unwrap();
        if !grants
            .get(&canonical)
            .is_some_and(|(p, ttl)| p == purpose && *ttl > now())
        {
            return Err(AppError::new(
                "PERMISSION_DENIED",
                "请通过系统选择器重新授权此路径",
            ));
        }
        Ok(canonical)
    }
    pub fn call(&self, name: &str, r: Value) -> Result<Value> {
        #[cfg(feature = "performance-probe")]
        let _timing = crate::performance_probe::CommandTimer::start(name);
        if name == "restore_catalog" {
            return self.restore(&r);
        }
        let _gate = self.inner.maintenance.read().unwrap();
        let mut c = self.connection()?;
        match name {
            "get_app_status" => Ok(
                json!({"databaseReady":true,"catalogVersion":db::CATALOG_VERSION,"catalogEpoch":db::epoch(&c)?,"offlineMode":true,"databasePath":self.inner.db_path,"cachePath":self.inner.cache,"watcher":self.watch_status(),"power":{"onBattery":self.on_battery()}}),
            ),
            "list_sources" => self.sources(&mut c),
            "add_source" => self.add_source(&mut c, &r),
            "rescan_source" => {
                let sid = required(&r, "sourceId")?;
                self.queue(&c, "SCAN", json!({"sourceId":sid}))
            }
            "set_source_enabled" => {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.execute(
                    "UPDATE source_roots SET is_enabled=?,updated_at=? WHERE id=?",
                    params![
                        r["enabled"].as_bool().unwrap_or(true),
                        now(),
                        required(&r, "sourceId")?
                    ],
                )?;
                db::bump(&tx)?;
                tx.commit()?;
                Ok(json!({"updated":true}))
            }
            "remove_source" => self.remove_source(&mut c, &r),
            "query_assets" => query::query(&c, serde_json::from_value(r)?),
            "query_burst_groups" => crate::bursts::query(&mut c, &r),
            "create_burst_snapshot" => crate::bursts::snapshot(&mut c, &r),
            "get_cull_progress" => query::cull_progress(&c, &r),
            "get_asset" => self.detail(&c, required(&r, "assetId")?),
            "save_capture_time" => crate::capture_time::save(&mut c, &r),
            "get_asset_neighbors" => self.neighbors(&c, &r),
            "resolve_preview" => media::resolve(self, &mut c, &r),
            "resolve_original" => media::original(self, &c, &r),
            "create_selection_snapshot" => query::snapshot(&mut c, &r),
            "apply_review_batch" => review::apply(&mut c, &r, false),
            "apply_tags_batch" => review::apply(&mut c, &r, true),
            "undo_review_batch" => review::undo(&mut c, &r),
            "list_tags" => self.tags(&c),
            "upsert_tag" => self.upsert_tag(&mut c, &r),
            "merge_tags" => self.merge_tags(&mut c, &r),
            "delete_tag" => self.delete_tag(&mut c, &r),
            "start_duplicate_scan" => {
                let kind = required(&r, "kind")?;
                if !["exact", "similar"].contains(&kind) {
                    return Err(AppError::new("INVALID_FILTER", "查重类型无效"));
                }
                query::target_ids(&c, &r["scope"])?;
                self.queue(
                    &c,
                    if kind == "exact" {
                        "HASH_EXACT"
                    } else {
                        "HASH_SIMILAR"
                    },
                    json!({"kind":kind,"scope":r["scope"],"similarity":r["similarity"]}),
                )
            }
            "query_duplicate_groups" => tasks::groups(self, &mut c, &r),
            "query_group_members" => tasks::group_members(&c, &r),
            "set_group_representative" | "exclude_group_member" => {
                tasks::group_action(&mut c, name, &r)
            }
            "preview_export" => files::preview_export(self, &mut c, &r),
            "get_trip_story" => crate::story::page(&mut c, &r),
            "get_quality_status" => crate::quality::status(self, &c, &r),
            "set_detail_enabled" => crate::quality::set_detail_enabled(&mut c, &r),
            "set_eyes_enabled" => crate::ai_models::set_enabled_for(
                self,
                &mut c,
                &r,
                crate::ai_models::ModelKind::Eyes,
            ),
            "import_eyes_model" => crate::ai_models::start_import_for(
                self,
                &mut c,
                &r,
                crate::ai_models::ModelKind::Eyes,
            ),
            "remove_eyes_model" => crate::ai_models::start_remove_for(
                self,
                &mut c,
                &r,
                crate::ai_models::ModelKind::Eyes,
            ),
            "get_eyes_model_info" => Ok(
                serde_json::json!({"profile":crate::model_resources::eye_profile(),"card":include_str!("../resources/ai/EYE_MODEL_CARD.md"),"licenseText":include_str!("../resources/ai/EYE_LICENSE.txt")}),
            ),
            "start_quality_analysis" => crate::quality::start(self, &mut c, &r),
            "get_quality_results" => crate::quality::results(&c, &r),
            "get_asset_quality" => crate::quality::asset(&c, required(&r, "assetId")?),
            "set_quality_dismissed" => crate::quality::dismiss(&mut c, &r),
            "get_ai_status" => crate::local_ai::status(self, &c, &r),
            "get_ai_model_info" => Ok(crate::local_ai::model_info()),
            "set_ai_enabled" => crate::ai_models::set_enabled(self, &mut c, &r),
            "import_ai_model" => crate::ai_models::start_import(self, &mut c, &r),
            "remove_ai_model" => crate::ai_models::start_remove(self, &mut c, &r),
            "start_ai_index" => crate::local_ai::start(self, &mut c, &r, false),
            "start_ai_cluster" => crate::local_ai::start(self, &mut c, &r, true),
            "semantic_search" => crate::local_ai::search(self, &mut c, &r),
            "get_ai_clusters" => crate::local_ai::groups(&c, &r),
            "preview_story_export" => crate::story::preview(self, &mut c, &r),
            "start_story_export" => crate::story::start(self, &mut c, &r),
            "preview_rename" => crate::rename_preview::start(self, &mut c, &r),
            "get_rename_preview" => crate::rename_preview::report(&c, &r),
            "list_export_presets" => Ok(json!(crate::export_options::list(&c)?)),
            "save_export_preset" | "delete_export_preset" => {
                crate::export_options::mutate(&mut c, name, &r)
            }
            "start_export" => files::start_export(self, &mut c, &r),
            "get_export_report" => files::report(&c, required(&r, "jobId")?),
            "list_jobs" => tasks::list(&c, &r),
            "get_job" => tasks::job(&c, required(&r, "jobId")?),
            "get_job_errors" => {
                let mut s = c.prepare(
                    "SELECT asset_id,message FROM job_errors WHERE job_id=? ORDER BY id LIMIT 500",
                )?;
                let rows=s.query_map([required(&r,"jobId")?],|row|Ok(json!({"assetId":row.get::<_,Option<String>>(0)?,"message":row.get::<_,String>(1)?})))?;
                Ok(json!(rows.collect::<std::result::Result<Vec<_>, _>>()?))
            }
            "cancel_job" | "pause_job" | "resume_job" => tasks::control(&c, name, &r),
            "retry_job" => {
                let old = tasks::job(&c, required(&r, "jobId")?)?;
                if ["running", "queued", "paused"].contains(&old["status"].as_str().unwrap_or("")) {
                    return Err(AppError::new("JOB_NOT_CANCELLABLE", "任务仍在运行"));
                }
                let checkpoint: String = c.query_row(
                    "SELECT checkpoint_json FROM jobs WHERE id=?",
                    [required(&r, "jobId")?],
                    |r| r.get(0),
                )?;
                let mut req: Value = serde_json::from_str(&checkpoint)?;
                if let Some(object) = req.as_object_mut() {
                    object.remove("cancelRequested");
                }
                if req["operation"] == "aiImportModel" || req["operation"] == "eyesImportModel" {
                    return Err(AppError::new(
                        "PERMISSION_DENIED",
                        "请在本地 AI 页面重新选择模型包后导入",
                    ));
                }
                if req["operation"] == "aiIndex" || req["operation"] == "aiCluster" {
                    crate::ai_models::ensure_enabled(self, &c)?;
                    crate::ai_models::ensure_no_job(&c)?;
                    crate::query::validate_snapshot(&c, required(&req["target"], "snapshotId")?)?;
                }
                if req["operation"] == "qualityAnalyze" {
                    crate::ai_models::ensure_no_job(&c)?;
                    crate::query::validate_snapshot(&c, required(&req["target"], "snapshotId")?)?;
                }
                if old["type"] == "EXPORT" {
                    return Err(AppError::new(
                        "EXPORT_CONFLICT",
                        "请重新预演导出，已完成的文件会按冲突策略处理",
                    ));
                }
                let j = self.queue(&c, old["type"].as_str().unwrap(), req)?;
                Ok(json!({"newJobId":j["jobId"]}))
            }
            "backup_catalog" => {
                self.authorized(Path::new(required(&r, "destinationPath")?), "backup")?;
                self.queue(&c, "BACKUP", r)
            }
            "prepare_restore" => self.prepare_restore(&r),
            "export_diagnostics" => files::export_diagnostics(self, &c, &r),
            "get_settings" => {
                let mut settings: Value = serde_json::from_str(include_str!(
                    "../../src/services/default-settings.json"
                ))?;
                if let Some(stored) = db::setting(&c, "preferences")? {
                    let saved: Value = serde_json::from_str(&stored)?;
                    let fields = saved.as_object().ok_or_else(|| {
                        AppError::new("INVALID_FILTER", "偏好设置格式无效")
                    })?;
                    self.validate_settings(&saved)?;
                    settings.as_object_mut().unwrap().extend(fields.clone());
                }
                Ok(settings)
            }
            "save_settings" => {
                self.validate_settings(&r)?;
                c.execute(
                    "INSERT OR REPLACE INTO app_settings VALUES('preferences',?)",
                    [r.to_string()],
                )?;
                Ok(json!({"updated":true}))
            }
            "get_session" => Ok(db::setting(&c, "cullSession")?
                .map(|s| serde_json::from_str(&s))
                .transpose()?
                .unwrap_or(Value::Null)),
            "save_session" => {
                c.execute(
                    "INSERT OR REPLACE INTO app_settings VALUES('cullSession',?)",
                    [r.to_string()],
                )?;
                Ok(json!({"updated":true}))
            }
            "clear_cache" => {
                self.clear_cache(&mut c)?;
                Ok(json!({"updated":true}))
            }
            "cache_stats" => media::cache_stats(self, &c),
            "preview_relink_source" => files::preview_relink(self, &c, &r),
            "commit_relink_source" => files::commit_relink(self, &mut c, &r),
            "list_trips" | "save_trip" | "delete_trip" | "add_to_trip" | "remove_from_trip"
            | "set_trip_cover" => self.trips(&mut c, name, &r),
            "list_saved_filters" | "save_filter" | "delete_filter" => {
                self.saved_filters(&c, name, &r)
            }
            "timeline_stats" | "map_assets" => self.aggregates(&c, name, &r),
            "map_trails" => crate::trails::query(&c, &r),
            "get_media_links" | "link_media" | "unlink_media" => self.links(&mut c, name, &r),
            _ => Err(AppError::new("INVALID_FILTER", "未知命令")),
        }
    }
    fn validate_settings(&self, r: &Value) -> Result<()> {
        if r.get("pauseOnBattery").is_some_and(|v| !v.is_boolean()) {
            return Err(AppError::new("INVALID_FILTER", "电池暂停选项须为布尔值"));
        }
        if let Some(n) = r["scanConcurrency"].as_i64() {
            if !(1..=4).contains(&n) {
                return Err(AppError::new("INVALID_FILTER", "并发数须为 1–4"));
            }
        }
        if let Some(n) = r["cacheLimitGb"].as_f64() {
            if !(0.1..=100.0).contains(&n) {
                return Err(AppError::new("INVALID_FILTER", "缓存限额须为 0.1–100 GB"));
            }
        }
        Ok(())
    }
    fn sources(&self, c: &mut Connection) -> Result<Value> {
        let mut st=c.prepare("SELECT id,display_name,absolute_path,volume_id,is_enabled,is_online,last_scan_at,(SELECT count(*) FROM media_assets a WHERE a.source_id=s.id),(SELECT count(*) FROM media_assets a WHERE a.source_id=s.id AND a.file_state IN ('offline','missing')) FROM source_roots s ORDER BY created_at")?;
        let rows = st.query_map([], |r| {
            let path: String = r.get(2)?;
            let access = files::check_source_access(Path::new(&path));
            Ok((json!({
                "id": r.get::<_, String>(0)?, "displayName": r.get::<_, String>(1)?,
                "absolutePath": path, "volumeId": r.get::<_, Option<String>>(3)?,
                "isEnabled": r.get::<_, bool>(4)?, "isOnline": access.is_ok(),
                "requiresAuthorization": access.is_err_and(|e| e.code == "PERMISSION_DENIED"),
                "lastScanAt": r.get::<_, Option<i64>>(6)?, "totalAssets": r.get::<_, i64>(7)?,
                "offlineAssets": r.get::<_, i64>(8)?
            }), r.get::<_, bool>(5)?))
        })?;
        let mut items = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        drop(st);
        for (s, was_online) in &mut items {
            let offline = s["isOnline"] == false;
            let available: bool = offline && c.query_row(
                "SELECT EXISTS(SELECT 1 FROM media_assets WHERE source_id=? AND file_state='available')",
                [s["id"].as_str()], |r| r.get(0),
            )?;
            if s["isOnline"].as_bool() == Some(*was_online) && !available {
                continue;
            }
            // Unchanged polling stays read-only; a transition and its epoch commit together.
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut changed = tx.execute(
                "UPDATE source_roots SET is_online=? WHERE id=? AND is_online<>?",
                params![
                    s["isOnline"].as_bool(),
                    s["id"].as_str(),
                    s["isOnline"].as_bool()
                ],
            )?;
            if offline {
                let marked = tx.execute("UPDATE media_assets SET file_state='offline' WHERE source_id=? AND file_state='available'",[s["id"].as_str()])?;
                s["offlineAssets"] = json!(s["offlineAssets"].as_i64().unwrap_or(0) + marked as i64);
                changed += marked;
            }
            if changed > 0 {
                db::bump(&tx)?;
            }
            tx.commit()?;
        }
        Ok(json!(items.into_iter().map(|(source,_)|source).collect::<Vec<_>>()))
    }
    fn add_source(&self, c: &mut Connection, r: &Value) -> Result<Value> {
        let path = self.authorized(Path::new(required(r, "selectedPath")?), "source")?;
        if !path.is_dir() {
            return Err(AppError::new("SOURCE_PATH_INVALID", "请选择目录"));
        }
        files::assert_not_app_path(self, &path)?;
        files::check_source_access(&path)?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let overlaps = source_overlaps(&tx, &path)?;
        let opts = tasks::SourceOptions::parse(&r["options"])?;
        let confirmation = r["overlapConfirmation"].as_str();
        if !overlaps.is_empty() {
            let signature = format!("{:x}", Sha256::digest(serde_json::to_vec(&(
                &path, &opts, &overlaps,
            ))?));
            if confirmation != Some(signature.as_str()) {
                let indexed_count: i64 = overlaps.iter().map(|(_, _, count)| count).sum();
                return Err(AppError::new("SOURCE_OVERLAP", "此目录与已有来源重叠，继续可能重复索引照片")
                    .details(json!({"sourceCount":overlaps.len(),"indexedCount":indexed_count,
                        "overlapConfirmation":signature})));
            }
        } else if confirmation.is_some() {
            return Err(AppError::new("DB_CONFLICT", "重叠范围已变化，请重新检查来源"));
        }
        let sid = id();
        let jid = id();
        let at = now();
        tx.execute("INSERT INTO source_roots(id,display_name,absolute_path,created_at,updated_at) VALUES(?,?,?,?,?)",params![sid,path.file_name().map(|s|s.to_string_lossy().into_owned()).unwrap_or("照片目录".into()),path.to_string_lossy(),at,at])?;
        tx.execute(
            "INSERT INTO source_options VALUES(?,?)",
            params![sid, serde_json::to_string(&opts)?],
        )?;
        let request = json!({"sourceId":sid});
        tasks::insert(&tx, &jid, "SCAN", &request)?;
        db::bump(&tx)?;
        tx.commit()?;
        self.spawn_job(jid.clone(), "SCAN".into(), request);
        Ok(json!({"sourceId":sid,"jobId":jid}))
    }
    fn remove_source(&self, c: &mut Connection, r: &Value) -> Result<Value> {
        let sid = required(r, "sourceId")?;
        if r["confirmed"] != true {
            return Err(AppError::new(
                "INVALID_SELECTION",
                "请确认移除索引及其整理记录",
            ));
        }
        if c.query_row(
            "SELECT EXISTS(SELECT 1 FROM jobs WHERE status IN ('running','queued','paused'))",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(AppError::new("DB_CONFLICT", "请先完成或取消后台任务"));
        }
        let tx = c.transaction()?;
        tx.execute("DELETE FROM export_run_items WHERE asset_id IN (SELECT id FROM media_assets WHERE source_id=?)",[sid])?;
        tx.execute("DELETE FROM media_assets WHERE source_id=?", [sid])?;
        tx.execute("DELETE FROM source_roots WHERE id=?", [sid])?;
        db::bump(&tx)?;
        tx.commit()?;
        Ok(json!({"updated":true}))
    }
    pub fn detail(&self, c: &Connection, aid: &str) -> Result<Value> {
        let mut v = query::asset(c, aid)?;
        v["captureTimeCorrection"] = crate::capture_time::detail(c, aid)?;
        let extra:Value=c.query_row("SELECT s.absolute_path,a.relative_path,a.camera_make,a.codec,a.gps_lat,a.gps_lon,a.orientation,a.capture_tz_min,a.capture_time_source,a.metadata_json FROM media_assets a JOIN source_roots s ON s.id=a.source_id WHERE a.id=?",[aid],|r|Ok(json!({"absolutePath":Path::new(&r.get::<_,String>(0)?).join(r.get::<_,String>(1)?),"cameraMake":r.get::<_,Option<String>>(2)?,"codec":r.get::<_,Option<String>>(3)?,"gpsLat":r.get::<_,Option<f64>>(4)?,"gpsLon":r.get::<_,Option<f64>>(5)?,"orientation":r.get::<_,Option<i64>>(6)?,"captureTzMin":r.get::<_,Option<i64>>(7)?,"captureTimeSource":r.get::<_,Option<String>>(8)?,"metadata":r.get::<_,Option<String>>(9)?.and_then(|s|serde_json::from_str::<Value>(&s).ok()).unwrap_or(json!({}))})))?;
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        if let Some(metadata) = v["metadata"].as_object_mut() {
            metadata.remove("_version");
        }
        let supported = !matches!(
            v["fileState"].as_str(),
            Some("unsupported" | "corrupt" | "missing" | "offline")
        );
        let video = v["mediaKind"] == "video";
        let original = supported
            && !video
            && v["extension"]
                .as_str()
                .is_some_and(|s| ["jpg", "jpeg", "png", "webp", "gif"].contains(&s))
            && v["byteSize"]
                .as_u64()
                .is_some_and(|n| n <= 64 * 1024 * 1024);
        let screen_cached: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM thumbnail_cache WHERE asset_id=? AND variant='screen')",
            [aid],
            |r| r.get(0),
        )?;
        v["preview"] = json!({"canThumbnail":supported||v["thumbnailState"]=="ready","canScreenPreview":supported||screen_cached,"canOriginalPreview":original,"canPlayInline":supported&&video,"canOpenSystem":!matches!(v["fileState"].as_str(),Some("missing"|"offline"))});
        let mut st = c.prepare("SELECT tag_id FROM asset_tags WHERE asset_id=?")?;
        v["tagIds"] = json!(st
            .query_map([aid], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?);
        Ok(v)
    }
    fn tags(&self, c: &Connection) -> Result<Value> {
        let mut s=c.prepare("SELECT t.id,t.name,t.color,count(a.asset_id) FROM tags t LEFT JOIN asset_tags a ON a.tag_id=t.id GROUP BY t.id ORDER BY t.name COLLATE NOCASE")?;
        let rows=s.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"color":r.get::<_,Option<String>>(2)?,"count":r.get::<_,i64>(3)?})))?;
        Ok(json!(rows.collect::<std::result::Result<Vec<_>, _>>()?))
    }
    fn upsert_tag(&self, c: &mut Connection, r: &Value) -> Result<Value> {
        let name = required(r, "name")?.trim();
        if name.is_empty() || name.len() > 200 {
            return Err(AppError::new("INVALID_FILTER", "标签名须为 1–200 个字节"));
        }
        let tid = r["id"].as_str().map(String::from).unwrap_or_else(id);
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: Option<String> = tx
            .query_row(
                "SELECT id FROM tags WHERE name=? COLLATE NOCASE",
                [name],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some_and(|i| i != tid) {
            return Err(AppError::new("DB_CONFLICT", "已存在同名标签"));
        }
        tx.execute("INSERT INTO tags VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,color=excluded.color",params![tid,name,r["color"].as_str(),now()])?;
        db::bump(&tx)?;
        tx.commit()?;
        Ok(json!({"tagId":tid}))
    }
    fn merge_tags(&self, c: &mut Connection, r: &Value) -> Result<Value> {
        let from = required(r, "sourceTagId")?;
        let to = required(r, "targetTagId")?;
        if from == to {
            return Err(AppError::new("INVALID_SELECTION", "请选择不同标签"));
        }
        let tx = c.transaction()?;
        tx.execute("UPDATE asset_reviews SET revision=revision+1,updated_at=? WHERE asset_id IN(SELECT asset_id FROM asset_tags WHERE tag_id=?)",params![now(),from])?;
        let count = tx.execute(
            "INSERT OR IGNORE INTO asset_tags SELECT asset_id,? FROM asset_tags WHERE tag_id=?",
            params![to, from],
        )?;
        tx.execute("DELETE FROM tags WHERE id=?", [from])?;
        db::bump(&tx)?;
        tx.commit()?;
        Ok(json!({"affectedCount":count}))
    }
    fn delete_tag(&self, c: &mut Connection, r: &Value) -> Result<Value> {
        let tid = required(r, "tagId")?;
        let tx = c.transaction()?;
        tx.execute("UPDATE asset_reviews SET revision=revision+1,updated_at=? WHERE asset_id IN(SELECT asset_id FROM asset_tags WHERE tag_id=?)",params![now(),tid])?;
        tx.execute("DELETE FROM tags WHERE id=?", [tid])?;
        db::bump(&tx)?;
        tx.commit()?;
        Ok(json!({"updated":true}))
    }
    fn trips(&self, c: &mut Connection, name: &str, r: &Value) -> Result<Value> {
        match name {
            "list_trips" => {
                let mut st=c.prepare("SELECT id,name,description,start_at,end_at,cover_asset_id,(SELECT count(*) FROM collection_assets WHERE collection_id=t.id) FROM collections t ORDER BY updated_at DESC")?;
                let rows=st.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"description":r.get::<_,Option<String>>(2)?,"startAt":r.get::<_,Option<i64>>(3)?,"endAt":r.get::<_,Option<i64>>(4)?,"coverAssetId":r.get::<_,Option<String>>(5)?,"count":r.get::<_,i64>(6)?})))?;
                return Ok(json!(rows.collect::<std::result::Result<Vec<_>, _>>()?));
            }
            "save_trip" => {
                let title = required(r, "name")?.trim();
                if title.is_empty() {
                    return Err(AppError::new("INVALID_FILTER", "请输入旅行名称"));
                }
                let tid = r["id"].as_str().map(String::from).unwrap_or_else(id);
                c.execute("INSERT INTO collections(id,name,kind,description,start_at,end_at,created_at,updated_at) VALUES(?,?,'trip',?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,description=excluded.description,start_at=excluded.start_at,end_at=excluded.end_at,updated_at=excluded.updated_at",params![tid,title,r["description"].as_str(),r["startAt"].as_i64(),r["endAt"].as_i64(),now(),now()])?;
                return Ok(json!({"tripId":tid}));
            }
            "delete_trip" => {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.execute(
                    "DELETE FROM collections WHERE id=?",
                    [required(r, "tripId")?],
                )?;
                db::bump(&tx)?;
                tx.commit()?;
            }
            "set_trip_cover" => {
                let tid = required(r, "tripId")?;
                let aid = required(r, "assetId")?;
                if !c.query_row("SELECT EXISTS(SELECT 1 FROM collection_assets WHERE collection_id=? AND asset_id=?)",params![tid,aid],|r|r.get::<_,bool>(0))?{return Err(AppError::new("INVALID_SELECTION","封面须来自此旅行"));}
                c.execute(
                    "UPDATE collections SET cover_asset_id=?,updated_at=? WHERE id=?",
                    params![aid, now(), tid],
                )?;
            }
            _ => {
                let tid = required(r, "tripId")?;
                let tx = c.transaction()?;
                let ids = query::target_ids(&tx, &r["target"])?;
                for aid in ids {
                    if name == "add_to_trip" {
                        tx.execute(
                            "INSERT OR IGNORE INTO collection_assets VALUES(?,?,0)",
                            params![tid, aid],
                        )?;
                    } else {
                        tx.execute(
                            "DELETE FROM collection_assets WHERE collection_id=? AND asset_id=?",
                            params![tid, aid],
                        )?;
                    }
                }
                tx.execute(
                    "UPDATE collections SET updated_at=? WHERE id=?",
                    params![now(), tid],
                )?;
                tx.execute("UPDATE collections SET cover_asset_id=(SELECT asset_id FROM collection_assets WHERE collection_id=? ORDER BY sort_order,asset_id LIMIT 1) WHERE id=? AND (cover_asset_id IS NULL OR cover_asset_id NOT IN(SELECT asset_id FROM collection_assets WHERE collection_id=?))",params![tid,tid,tid])?;
                db::bump(&tx)?;
                tx.commit()?;
            }
        }
        Ok(json!({"updated":true}))
    }
    fn saved_filters(&self, c: &Connection, name: &str, r: &Value) -> Result<Value> {
        if name == "list_saved_filters" {
            let mut st =
                c.prepare("SELECT id,name,filter_json,sort_json FROM saved_filters ORDER BY name")?;
            let rows=st.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"filter":serde_json::from_str::<Value>(&r.get::<_,String>(2)?).unwrap_or(Value::Null),"sort":serde_json::from_str::<Value>(&r.get::<_,String>(3)?).unwrap_or(Value::Null)})))?;
            return Ok(json!(rows.collect::<std::result::Result<Vec<_>, _>>()?));
        }
        if name == "delete_filter" {
            c.execute(
                "DELETE FROM saved_filters WHERE id=?",
                [required(r, "filterId")?],
            )?;
        } else {
            let f: query::Filter = serde_json::from_value(r["filter"].clone())?;
            let s: query::Sort = serde_json::from_value(r["sort"].clone())?;
            query::conditions(&f)?;
            query::sort_column(&s)?;
            c.execute("INSERT INTO saved_filters VALUES(?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,filter_json=excluded.filter_json,sort_json=excluded.sort_json,updated_at=excluded.updated_at",params![r["id"].as_str().map(String::from).unwrap_or_else(id),required(r,"name")?.trim(),r["filter"].to_string(),r["sort"].to_string(),now(),now()])?;
        }
        Ok(json!({"updated":true}))
    }
    fn aggregates(&self, c: &Connection, name: &str, r: &Value) -> Result<Value> {
        let f: query::Filter =
            serde_json::from_value(r.get("filter").cloned().unwrap_or(json!({})))?;
        let (w, p) = query::scoped_conditions(c, &f)?;
        if name == "timeline_stats" {
            let mut st=c.prepare(&format!("SELECT {date} AS day,count(*),sum(r.pick_state<>'unreviewed'),sum(({date}) IS NOT NULL AND a.capture_tz_min IS NULL),sum(a.media_kind='image'),sum(a.media_kind='video') {from} WHERE {w} GROUP BY day ORDER BY day IS NULL,day DESC",date=query::CAPTURE_DATE,from=query::FROM))?;
            let rows=st.query_map(rusqlite::params_from_iter(p),|r|Ok(json!({"date":r.get::<_,Option<String>>(0)?,"count":r.get::<_,i64>(1)?,"reviewed":r.get::<_,i64>(2)?,"unknownTimezone":r.get::<_,i64>(3)?,"photos":r.get::<_,i64>(4)?,"videos":r.get::<_,i64>(5)?})))?;
            Ok(json!(rows.collect::<std::result::Result<Vec<_>, _>>()?))
        } else {
            let mut st=c.prepare(&format!("SELECT round(a.gps_lon,2),round(a.gps_lat,2),count(*),min(a.id) {} WHERE {} AND a.gps_lat BETWEEN -90 AND 90 AND a.gps_lon BETWEEN -180 AND 180 GROUP BY 1,2 ORDER BY 1,2",query::FROM,w))?;
            let rows=st.query_map(rusqlite::params_from_iter(p),|r|Ok(json!({"lon":r.get::<_,f64>(0)?,"lat":r.get::<_,f64>(1)?,"count":r.get::<_,i64>(2)?,"assetId":r.get::<_,String>(3)?})))?;
            Ok(json!(rows.collect::<std::result::Result<Vec<_>, _>>()?))
        }
    }
    fn links(&self, c: &mut Connection, name: &str, r: &Value) -> Result<Value> {
        let aid = required(r, "assetId")?;
        if name == "get_media_links" {
            let mut st=c.prepare("SELECT id,primary_asset_id,secondary_asset_id,link_type,match_method,confidence,confirmed FROM linked_media WHERE (primary_asset_id=? OR secondary_asset_id=?) AND match_method<>'manual_exclusion'")?;
            let rows=st.query_map(params![aid,aid],|r|Ok(json!({"id":r.get::<_,String>(0)?,"primaryAssetId":r.get::<_,String>(1)?,"secondaryAssetId":r.get::<_,String>(2)?,"linkType":r.get::<_,String>(3)?,"matchMethod":r.get::<_,String>(4)?,"confidence":r.get::<_,Option<f64>>(5)?,"confirmed":r.get::<_,bool>(6)?})))?;
            return Ok(json!(rows.collect::<std::result::Result<Vec<_>, _>>()?));
        }
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if name == "unlink_media" {
            tx.execute(
                "UPDATE linked_media SET confirmed=0,confidence=NULL,match_method='manual_exclusion' WHERE id=?",
                [required(r, "linkId")?],
            )?;
        } else {
            let second = required(r, "secondaryAssetId")?;
            let kind = required(r, "linkType")?;
            if !["live_photo", "raw_jpeg"].contains(&kind) {
                return Err(AppError::new("INVALID_SELECTION", "配对类型无效"));
            }
            let primary = query::asset(&tx, aid)?;
            let secondary = query::asset(&tx, second)?;
            let valid = aid != second
                && match kind {
                    "live_photo" => {
                        primary["mediaKind"] == "image" && secondary["extension"] == "mov"
                    }
                    "raw_jpeg" => {
                        primary["extension"]
                            .as_str()
                            .is_some_and(crate::media::is_raw)
                            && secondary["extension"]
                                .as_str()
                                .is_some_and(|s| ["jpg", "jpeg"].contains(&s))
                    }
                    _ => false,
                };
            if !valid {
                return Err(AppError::new(
                    "INVALID_SELECTION",
                    "配套素材的格式或主次顺序无效",
                ));
            }
            tx.execute("INSERT INTO linked_media VALUES(?,?,?,?,?,1,1) ON CONFLICT(primary_asset_id,secondary_asset_id,link_type) DO UPDATE SET confirmed=1,match_method='manual'",params![id(),aid,second,kind,"manual"])?;
        }
        db::bump(&tx)?;
        tx.commit()?;
        Ok(json!({"updated":true}))
    }
}
