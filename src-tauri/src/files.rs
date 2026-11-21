use crate::{
    catalog::Catalog,
    db,
    error::{required, AppError, Result},
    id, now, query, review, tasks,
};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

pub fn stamp(path: &Path) -> Result<(u64, i64)> {
    let m = fs::metadata(path)?;
    Ok((m.len(), modified_ms(&m)?))
}
fn modified_ms(metadata: &fs::Metadata) -> Result<i64> {
    Ok(metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64)
}
pub(crate) fn check_source_access(root: &Path) -> Result<()> {
    // A sandbox may allow directory metadata while denying enumeration.
    fs::read_dir(root)
        .and_then(|mut entries| entries.next().transpose())
        .map(drop)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::PermissionDenied {
                AppError::new("PERMISSION_DENIED", "无法读取来源目录，请重新授权来源")
            } else {
                AppError::new("SOURCE_OFFLINE", "来源目录不可读取，请检查磁盘连接后重试")
            }
        })
}
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct FileVersion {
    pub size: u64,
    pub mtime_ms: i64,
    pub identity: Option<String>,
    pub token: String,
}
impl FileVersion {
    pub fn read(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            Self::from_metadata(fs::metadata(path)?)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::FILE_READ_ATTRIBUTES;
            Self::from_file(
                &fs::OpenOptions::new()
                    .access_mode(FILE_READ_ATTRIBUTES)
                    .open(path)?,
            )
        }
    }
    pub fn matches_index(&self, size: u64, mtime_ms: Option<i64>, token: Option<&str>) -> bool {
        size == self.size
            && mtime_ms == Some(self.mtime_ms)
            && token.is_none_or(|token| token == self.token.as_str())
    }
    #[cfg(unix)]
    fn from_metadata(metadata: fs::Metadata) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let identity = format!("{}:{}", metadata.dev(), metadata.ino());
        let token = format!(
            "unix:{identity}:{}:{}:{}:{}",
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec()
        );
        Self::regular(metadata, Some(identity), token)
    }
    #[cfg(unix)]
    fn from_file(file: &File) -> Result<Self> {
        Self::from_metadata(file.metadata()?)
    }
    #[cfg(windows)]
    fn from_file(file: &File) -> Result<Self> {
        use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
            BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO,
        };
        let metadata = file.metadata()?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        let mut basic = FILE_BASIC_INFO::default();
        // The handle and the matching writable structures outlive both synchronous calls.
        let identity = (unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) }
            != 0)
            .then(|| {
                format!(
                    "{}:{}",
                    info.dwVolumeSerialNumber,
                    (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)
                )
            });
        let basic_available = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                (&mut basic as *mut FILE_BASIC_INFO).cast(),
                std::mem::size_of::<FILE_BASIC_INFO>() as u32,
            )
        } != 0;
        let change_time = if basic_available { basic.ChangeTime } else { 0 };
        let token = format!(
            "windows:{}:{}:{change_time}",
            identity.as_deref().unwrap_or("-"),
            metadata.last_write_time()
        );
        Self::regular(metadata, identity, token)
    }
    fn regular(metadata: fs::Metadata, identity: Option<String>, token: String) -> Result<Self> {
        if !metadata.is_file() {
            return Err(AppError::new("ASSET_NOT_FOUND", "原文件缺失或不是普通文件"));
        }
        Ok(Self {
            size: metadata.len(),
            mtime_ms: modified_ms(&metadata)?,
            identity,
            token,
        })
    }
}
pub fn source_path(c: &Connection, aid: &str) -> Result<PathBuf> {
    let (root,rel):(String,String)=c.query_row("SELECT s.absolute_path,a.relative_path FROM media_assets a JOIN source_roots s ON s.id=a.source_id WHERE a.id=?",[aid],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or_else(||AppError::new("ASSET_NOT_FOUND","媒体不存在"))?;
    let root = PathBuf::from(root);
    if !root.is_dir() {
        return Err(AppError::new(
            "SOURCE_OFFLINE",
            "来源目录离线，请重新连接磁盘",
        ));
    }
    safe_relative(Path::new(&rel))?;
    let path = root
        .join(&rel)
        .canonicalize()
        .map_err(|_| AppError::new("ASSET_NOT_FOUND", "原文件缺失或无法读取"))?;
    if !path.starts_with(root.canonicalize()?) {
        return Err(AppError::new("PERMISSION_DENIED", "媒体路径超出授权目录"));
    }
    Ok(path)
}
pub fn source_directory(c: &Connection, aid: &str) -> Result<PathBuf> {
    source_path(c, aid)?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| AppError::new("ASSET_NOT_FOUND", "媒体所在文件夹不可用"))
}
pub fn safe_relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(AppError::new("EXPORT_TARGET_UNSAFE", "目标相对路径无效"));
    }
    Ok(())
}
pub fn assert_not_app_path(cat: &Catalog, path: &Path) -> Result<()> {
    for app in [&cat.inner.data, &cat.inner.cache] {
        let app = app.canonicalize()?;
        if path.starts_with(&app) || app.starts_with(path) {
            return Err(AppError::new(
                "EXPORT_TARGET_UNSAFE",
                "不能选择旅图自身的数据或缓存目录",
            ));
        }
    }
    Ok(())
}
pub fn safe_destination(
    cat: &Catalog,
    c: &Connection,
    path: &Path,
    purpose: &str,
) -> Result<PathBuf> {
    let path = cat.authorized(path, purpose)?;
    if !path.is_dir() {
        return Err(export_directory_error(io::Error::from(io::ErrorKind::NotADirectory)));
    }
    assert_not_app_path(cat, &path)?;
    let mut s = c.prepare("SELECT absolute_path FROM source_roots")?;
    for root in s.query_map([], |r| r.get::<_, String>(0))? {
        let root = PathBuf::from(root?);
        let root = root.canonicalize().unwrap_or(root);
        if path.starts_with(&root) || root.starts_with(&path) {
            return Err(AppError::new(
                "EXPORT_TARGET_UNSAFE",
                "目标目录不能与任何来源目录互相包含",
            ));
        }
    }
    Ok(path)
}
pub fn backup(cat: &Catalog, jid: &str, r: &Value) -> Result<()> {
    let c = cat.connection()?;
    let dest = safe_destination(
        cat,
        &c,
        Path::new(required(r, "destinationPath")?),
        "backup",
    )?;
    let dir = Dir::open_ambient_dir(&dest, ambient_authority()).map_err(export_directory_error)?;
    let stage = cat.inner.data.join(format!("backup-stage-{}.sqlite", id()));
    let name = format!("travel-gallery-{}-{}.sqlite", now(), &jid[..8]);
    let part = format!(".{name}.travelgallery-part-{jid}");
    tasks::progress(&c, jid, 0, Some(1), 0)?;
    let result = (|| -> Result<()> {
        db::backup_controlled(&c, &stage, || tasks::safe_point(cat, jid))?;
        tasks::safe_point(cat, jid)?;
        let mut input = File::open(&stage)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut output = dir.open_with(&part, &options)?;
        let mut bytes = vec![0; 1024 * 1024];
        let mut expected = Sha256::new();
        loop {
            tasks::safe_point(cat, jid)?;
            let count = input.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            output.write_all(&bytes[..count])?;
            expected.update(&bytes[..count]);
        }
        output.sync_all()?;
        drop(output);
        let mut verify = dir.open(&part)?;
        let mut actual = Sha256::new();
        loop {
            tasks::safe_point(cat, jid)?;
            let count = verify.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            actual.update(&bytes[..count]);
        }
        if expected.finalize() != actual.finalize() {
            return Err(AppError::new("INTERNAL", "备份副本校验失败，请重试"));
        }
        drop(verify);
        tasks::safe_point(cat, jid)?;
        crate::publish::no_replace(&dir, Path::new(&part), Path::new(&name))?;
        tasks::progress(&c, jid, 1, Some(1), 0)
    })();
    let _ = fs::remove_file(stage);
    if result.is_err() {
        let _ = dir.remove_file(part);
    }
    result
}
/// Construct a bounded, explicit whitelist in the backend; never serialize request or error details.
pub fn export_diagnostics(cat: &Catalog, c: &Connection, r: &Value) -> Result<Value> {
    let destination = safe_destination(cat, c, Path::new(required(r, "destinationPath")?), "diagnostics")?;
    let source_count: i64 = c.query_row("SELECT count(*) FROM source_roots", [], |r| r.get(0))?;
    let asset_count: i64 = c.query_row("SELECT count(*) FROM media_assets", [], |r| r.get(0))?;
    let mut statement = c.prepare("SELECT type,status,progress_done,CASE WHEN total_is_known THEN progress_total END,failure_count FROM jobs ORDER BY created_at DESC,id DESC LIMIT 100")?;
    let jobs = statement.query_map([], |r| {
        let kind: String = r.get(0)?;
        let status: String = r.get(1)?;
        let kind = match kind.as_str() {
            "SCAN" | "THUMBNAIL" | "METADATA" | "HASH_EXACT" | "HASH_SIMILAR" | "EXPORT" | "BACKUP" | "REPAIR" => kind.as_str(),
            _ => "other",
        };
        let status = match status.as_str() {
            "queued" | "running" | "paused" | "completed" | "failed" | "cancelled" => status.as_str(),
            _ => "other",
        };
        Ok(json!({"type":kind,"status":status,"progressDone":r.get::<_,i64>(2)?,"progressTotal":r.get::<_,Option<i64>>(3)?,"failures":r.get::<_,i64>(4)?}))
    })?.collect::<std::result::Result<Vec<_>,_>>()?;
    let report = json!({"application":"Travel Gallery","version":env!("CARGO_PKG_VERSION"),
        "generatedAt":chrono::Utc::now().to_rfc3339(),"offlineMode":true,
        "catalogVersion":db::CATALOG_VERSION,"platform":std::env::consts::OS,
        "sourceCount":source_count,"assetCount":asset_count,"jobs":jobs});
    let mut payload = serde_json::to_vec_pretty(&report)?;
    payload.push(b'\n');
    let name = format!("travel-gallery-diagnostics-{}-{}.json", now(), id());
    let part = format!(".{name}.travelgallery-part");
    let dir = Dir::open_ambient_dir(destination, ambient_authority()).map_err(export_directory_error)?;
    let result = (|| -> Result<Value> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut output = dir.open_with(&part, &options).map_err(export_io_error)?;
        output.write_all(&payload).map_err(export_io_error)?;
        output.sync_all().map_err(export_io_error)?;
        drop(output);
        crate::publish::no_replace(&dir, Path::new(&part), Path::new(&name)).map_err(export_publish_error)?;
        Ok(json!({"fileName":name}))
    })();
    if result.is_err() { let _ = dir.remove_file(&part); }
    result
}
fn rel_exists(dir: &Dir, p: &Path) -> bool {
    dir.symlink_metadata(p).is_ok()
}
fn name_key(p: &Path) -> String {
    let s = p.to_string_lossy().into_owned();
    if cfg!(windows) || cfg!(target_os = "macos") {
        s.to_lowercase()
    } else {
        s
    }
}
pub fn preview_export(cat: &Catalog, c: &mut Connection, r: &Value) -> Result<Value> {
    if r.get("includeXmp").is_some_and(|v| !v.is_boolean()) {
        return Err(AppError::new("INVALID_FILTER", "XMP 导出选项必须为布尔值"));
    }
    let include_xmp = r["includeXmp"] == true;
    let naming: Option<crate::export_options::NamingRule> =
        serde_json::from_value(r.get("naming").cloned().unwrap_or(Value::Null))?;
    if let Some(rule) = &naming {
        rule.validate()?;
    }
    let dest = safe_destination(cat, c, Path::new(required(r, "destinationPath")?), "export")?;
    let dir = Dir::open_ambient_dir(&dest, ambient_authority()).map_err(export_directory_error)?;
    let policy = required(r, "conflictPolicy")?;
    if !["skip", "rename"].contains(&policy) {
        return Err(AppError::new(
            "INVALID_FILTER",
            "不支持覆盖，冲突策略须为跳过或改名",
        ));
    }
    let layout = required(r, "layout")?;
    if !["flat", "captureMonth", "sourceRelative"].contains(&layout) {
        return Err(AppError::new("INVALID_FILTER", "导出目录规则无效"));
    }
    let tx = c.transaction()?;
    let mut ids = query::target_ids(&tx, &r["target"])?;
    if r["includeLinked"] == true {
        tx.execute_batch("CREATE TEMP TABLE export_selected_ids(id TEXT PRIMARY KEY)")?;
        {
            let mut insert = tx.prepare("INSERT INTO export_selected_ids VALUES(?)")?;
            for id in &ids {
                insert.execute([id])?;
            }
        }
        let mut st=tx.prepare("WITH RECURSIVE members(id) AS (SELECT id FROM export_selected_ids UNION SELECT CASE WHEN lm.primary_asset_id=members.id THEN lm.secondary_asset_id ELSE lm.primary_asset_id END FROM linked_media lm JOIN members ON members.id=lm.primary_asset_id OR members.id=lm.secondary_asset_id WHERE lm.confirmed=1) SELECT id FROM members ORDER BY id")?;
        ids = st
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
    }
    if ids.is_empty() {
        return Err(AppError::new("INVALID_SELECTION", "请先选择媒体"));
    }
    let (mut total, mut conflicts, mut offline, mut duplicates) = (0i64, 0, 0, 0);
    let mut stale = 0;
    let mut used = HashSet::new();
    let mut items = vec![];
    let mut replaced_characters = 0;
    if naming.is_some() {
        let mut sorted=tx.prepare("SELECT a.id FROM media_assets a JOIN json_each(?) j ON j.value=a.id ORDER BY a.filename COLLATE NOCASE,a.relative_path,a.source_id,a.id")?;
        ids = sorted
            .query_map([serde_json::to_string(&ids)?], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<_, _>>()?;
    }
    let mut examples = vec![];
    for (index, aid) in ids.iter().enumerate() {
        let a = cat.detail(&tx, aid)?;
        let filename = required(&a, "filename")?;
        let relative = required(&a, "relativePath")?;
        let (source, indexed_version): (String, Option<String>) = tx.query_row(
            "SELECT s.display_name,a.file_version FROM media_assets a JOIN source_roots s ON s.id=a.source_id WHERE a.id=?",
            [aid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let renamed = naming
            .as_ref()
            .map(|rule| {
                rule.filename(
                    filename,
                    &source,
                    if a["captureTimeSource"] == "mtime" {
                        None
                    } else {
                        a["captureLocal"].as_str()
                    },
                    index,
                )
            })
            .transpose()?;
        let output_filename = renamed.as_deref().unwrap_or(filename);
        let base = match layout {
            "captureMonth" => {
                let date = a["captureLocal"].as_str().unwrap_or("未知日期");
                PathBuf::from(date.get(..7).unwrap_or("未知日期")).join(output_filename)
            }
            "sourceRelative" => {
                let root: String = tx.query_row(
                    "SELECT display_name FROM source_roots WHERE id=?",
                    [a["sourceId"].as_str()],
                    |r| r.get(0),
                )?;
                PathBuf::from(root).join(Path::new(relative).with_file_name(output_filename))
            }
            _ => PathBuf::from(output_filename),
        };
        safe_relative(&base)?;
        let mut output = base.clone();
        let duplicate = used.contains(&name_key(&base))
            || (include_xmp && used.contains(&name_key(&base.with_extension("xmp"))));
        let conflict = rel_exists(&dir, &base)
            || duplicate
            || (include_xmp && rel_exists(&dir, &base.with_extension("xmp")));
        if conflict {
            conflicts += 1;
        }
        if duplicate {
            duplicates += 1;
        }
        if policy == "rename" {
            let mut n = 2;
            while used.contains(&name_key(&output))
                || rel_exists(&dir, &output)
                || (include_xmp
                    && (used.contains(&name_key(&output.with_extension("xmp")))
                        || rel_exists(&dir, &output.with_extension("xmp"))))
            {
                let stem = base.file_stem().unwrap_or_default().to_string_lossy();
                let ext = base
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                output = base.with_file_name(format!("{stem} ({n}){ext}"));
                n += 1;
            }
        }
        used.insert(name_key(&output));
        let sidecar = if include_xmp {
            let path = output.with_extension("xmp");
            used.insert(name_key(&path));
            let packet = crate::xmp::packet(&tx, &a)?;
            total += packet.content.len() as i64;
            replaced_characters += packet.replaced_characters;
            Some(json!({"relative":path,"content":packet.content}))
        } else {
            None
        };
        let current = source_path(&tx, aid)
            .and_then(|path| FileVersion::read(&path))
            .ok();
        let available = current.is_some();
        if current.as_ref().is_some_and(|version| {
            !version.matches_index(
                a["byteSize"].as_u64().unwrap_or(u64::MAX),
                a["mtimeMs"].as_i64(),
                indexed_version.as_deref(),
            )
        }) {
            stale += 1;
        }
        // An indexed version also guards items from an offline source that reconnects before copying.
        let source_version = indexed_version
            .as_deref()
            .or_else(|| current.as_ref().map(|v| v.token.as_str()));
        if examples.len() < 100 {
            examples.push(json!({"assetId":aid,"sourceRelativePath":relative,"destinationRelativePath":output,"skipped":conflict&&policy=="skip"}));
        }
        if !available {
            offline += 1;
        }
        total += a["byteSize"].as_i64().unwrap_or(0);
        items.push(json!({"assetId":aid,"relative":output,"skip":conflict&&policy=="skip","unavailable":!available,
            "size":a["byteSize"],"mtime":a["mtimeMs"],"sourceVersion":source_version,"xmp":sidecar}));
    }
    let token = id();
    let expires = now() + 900_000;
    let plan = json!({"destination":dest,"items":items,"policy":policy});
    tx.execute(
        "INSERT INTO export_plans VALUES(?,NULL,?,?,?,?,?,NULL)",
        params![token, plan.to_string(), total, ids.len(), now(), expires],
    )?;
    tx.commit()?;
    let mut warnings = vec![];
    if offline > 0 {
        warnings.push(format!("{offline} 个原文件不可用，执行时会列入失败明细"));
    }
    if stale > 0 {
        warnings.push(format!(
            "{stale} 个原文件与索引版本不一致，请重新扫描和预演"
        ));
    }
    if replaced_characters > 0 {
        warnings.push(format!(
            "XMP 不支持的 {replaced_characters} 个控制字符会替换为 �，资料库原记录不变"
        ));
    }
    if fs2::available_space(&dest).unwrap_or(u64::MAX) < total as u64 {
        warnings.push("目标可用空间可能不足".into());
    }
    Ok(
        json!({"planToken":token,"expiresAt":expires,"fileCount":ids.len(),"xmpCount":if include_xmp{ids.len()}else{0},"totalBytes":total,"conflictCount":conflicts,"offlineCount":offline,"duplicateDestinationCount":duplicates,"warnings":warnings,"examples":examples,"examplesTruncated":ids.len()>100}),
    )
}
pub fn start_export(cat: &Catalog, c: &mut Connection, r: &Value) -> Result<Value> {
    let key = required(r, "clientMutationId")?;
    let token = required(r, "planToken")?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(v) = review::receipt(&tx, key, r)? {
        return Ok(v);
    }
    let (data, ttl, used): (String, i64, Option<String>) = tx
        .query_row(
            "SELECT plan_json,expires_at,consumed_by_job_id FROM export_plans WHERE token=?",
            [token],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::new("SNAPSHOT_EXPIRED", "导出预演不存在"))?;
    if ttl < now() || used.is_some() {
        return Err(AppError::new(
            "SNAPSHOT_EXPIRED",
            "此预演已过期或已执行，请重新预演",
        ));
    }
    let plan: Value = serde_json::from_str(&data)?;
    if plan["kind"] == "renamePreview" || plan["kind"] == "storyExport" {
        return Err(AppError::new("INVALID_FILTER", "请使用对应操作执行此预演"));
    }
    safe_destination(
        cat,
        &tx,
        Path::new(required(&plan, "destination")?),
        "export",
    )?;
    let jid = id();
    tasks::insert(&tx, &jid, "EXPORT", &plan)?;
    for item in plan["items"].as_array().unwrap() {
        tx.execute(
            "INSERT INTO export_run_items VALUES(?,?,?,'queued',0,NULL,NULL,?)",
            params![
                jid,
                item["assetId"].as_str(),
                item["relative"].as_str(),
                now()
            ],
        )?;
        if let Some(sidecar) = item.get("xmp").filter(|v| v.is_object()) {
            tx.execute(
                "INSERT INTO export_sidecar_items VALUES(?,?,?,'queued',0,NULL,NULL,?)",
                params![
                    jid,
                    item["assetId"].as_str(),
                    required(sidecar, "relative")?,
                    now()
                ],
            )?;
        }
    }
    tx.execute(
        "UPDATE export_plans SET consumed_by_job_id=? WHERE token=?",
        params![jid, token],
    )?;
    let res = json!({"jobId":jid});
    review::save_receipt(&tx, key, r, &res)?;
    tx.commit()?;
    cat.spawn_job(jid, "EXPORT".into(), plan);
    Ok(res)
}
fn hash_file(f: &mut File, cat: Option<(&Catalog, &str)>) -> Result<String> {
    let mut h = Sha256::new();
    let mut b = vec![0; 1024 * 1024];
    loop {
        if let Some((cat, jid)) = cat {
            tasks::safe_point(cat, jid)?;
        }
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
pub fn sha256(path: &Path, cat: Option<(&Catalog, &str)>) -> Result<String> {
    sha256_at_version(path, &FileVersion::read(path)?, cat)
}
pub(crate) fn sha256_at_version(
    path: &Path,
    version: &FileVersion,
    cat: Option<(&Catalog, &str)>,
) -> Result<String> {
    let mut file = File::open(path)?;
    if FileVersion::from_file(&file)? != *version {
        return Err(AppError::new("DB_CONFLICT", "文件在读取期间已变化"));
    }
    let hash = hash_file(&mut file, cat)?;
    if FileVersion::from_file(&file)? != *version || FileVersion::read(path)? != *version {
        return Err(AppError::new("DB_CONFLICT", "文件在读取期间已变化"));
    }
    Ok(hash)
}

fn export_source_changed() -> AppError {
    AppError::new("DB_CONFLICT", "复制期间原文件发生变化，请重新扫描和预演")
}

fn copy_export_media(
    mut staged: crate::publish::StagedFile,
    source: &Path,
    mut input: File,
    before: &FileVersion,
    mut control: impl FnMut() -> Result<()>,
) -> Result<(u64, String)> {
    let mut target = staged.writer().map_err(export_io_error)?;
    let mut hash = Sha256::new();
    let mut bytes = 0;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        control()?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        target
            .write_all(&buffer[..count])
            .map_err(export_io_error)?;
        hash.update(&buffer[..count]);
        bytes += count as u64;
    }
    target.sync_all().map_err(export_io_error)?;
    drop(target);
    if bytes != before.size || FileVersion::from_file(&input)? != *before {
        return Err(export_source_changed());
    }
    let expected = format!("{:x}", hash.finalize());
    let mut verify = staged.reader()?;
    let mut hash = Sha256::new();
    loop {
        control()?;
        let count = verify.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    drop(verify);
    if format!("{:x}", hash.finalize()) != expected {
        return Err(AppError::new("ASSET_CORRUPT", "复制校验不一致"));
    }
    control()?;
    if FileVersion::read(source).as_ref().ok() != Some(before) {
        return Err(export_source_changed());
    }
    // Publish with the OS no-replace primitive, relative to a capability directory handle.
    staged.commit().map_err(export_publish_error)?;
    Ok((bytes, expected))
}
fn export_io_error(error: io::Error) -> AppError {
    export_target_error(error, crate::export_errors::Operation::Write)
}
fn export_directory_error(error: io::Error) -> AppError {
    export_target_error(error, crate::export_errors::Operation::Directory)
}
fn export_target_error(error: io::Error, operation: crate::export_errors::Operation) -> AppError {
    let (code, message) = crate::export_errors::classify(operation, error.kind());
    AppError::new(code, message).details(json!({"reason":error.to_string(),"osError":error.raw_os_error()}))
}
fn export_publish_error(error: io::Error) -> AppError {
    export_target_error(error, crate::export_errors::Operation::Publish)
}
pub(crate) fn write_generated(
    dir: &Dir,
    relative: &Path,
    content: &[u8],
    control: impl FnMut() -> Result<()>,
) -> Result<(u64, String)> {
    write_generated_checked(dir, relative, content, control, || Ok(()))
}
pub(crate) fn write_generated_checked(
    dir: &Dir,
    relative: &Path,
    content: &[u8],
    mut control: impl FnMut() -> Result<()>,
    before_publish: impl FnOnce() -> Result<()>,
) -> Result<(u64, String)> {
    safe_relative(relative)?;
    let part = relative.with_file_name(format!(
        ".{}.travelgallery-part-{}",
        relative.file_name().unwrap().to_string_lossy(),
        id()
    ));
    let result = (|| {
        control()?;
        let mut staged = crate::publish::StagedFile::create(dir, &part, relative).map_err(export_io_error)?;
        let mut target = staged.writer().map_err(export_io_error)?;
        for chunk in content.chunks(64 * 1024) {
            control()?;
            target.write_all(chunk).map_err(export_io_error)?;
        }
        target.sync_all().map_err(export_io_error)?;
        drop(target);
        control()?;
        let mut verify = staged.reader()?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        let mut bytes = 0;
        loop {
            control()?;
            let n = verify.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
            bytes += n;
        }
        drop(verify);
        let hash = format!("{:x}", hash.finalize());
        if bytes != content.len() || hash != format!("{:x}", Sha256::digest(content)) {
            return Err(AppError::new("ASSET_CORRUPT", "导出文件副本校验失败"));
        }
        control()?;
        before_publish()?;
        staged.commit().map_err(export_publish_error)?;
        Ok((bytes as u64, hash))
    })();
    result
}
pub fn export(cat: &Catalog, jid: &str, r: &Value) -> Result<()> {
    let c = cat.connection()?;
    let dest = safe_destination(cat, &c, Path::new(required(r, "destination")?), "export")?;
    let dir = Dir::open_ambient_dir(&dest, ambient_authority()).map_err(export_directory_error)?;
    let items = r["items"]
        .as_array()
        .ok_or_else(|| AppError::new("INTERNAL", "导出计划损坏"))?;
    tasks::progress(&c, jid, 0, Some(items.len()), 0)?;
    let mut failures = 0;
    for (index, item) in items.iter().enumerate() {
        tasks::safe_point(cat, jid)?;
        let aid = required(item, "assetId")?;
        let relative = PathBuf::from(required(item, "relative")?);
        safe_relative(&relative)?;
        if item["skip"] == true {
            c.execute("UPDATE export_run_items SET state='skipped',error_code='EXPORT_CONFLICT',updated_at=? WHERE job_id=? AND asset_id=?",params![now(),jid,aid])?;
            c.execute("UPDATE export_sidecar_items SET state='skipped',error_code='EXPORT_CONFLICT',updated_at=? WHERE job_id=? AND asset_id=?",params![now(),jid,aid])?;
            tasks::progress(&c, jid, index + 1, Some(items.len()), failures)?;
            continue;
        }
        let part = relative.with_file_name(format!(
            ".{}.travelgallery-part-{jid}",
            relative.file_name().unwrap().to_string_lossy()
        ));
        let copy = (|| -> Result<(u64, String)> {
            let source = source_path(&c, aid)?;
            let before = FileVersion::read(&source)?;
            let expected_version = item["sourceVersion"].as_str().ok_or_else(|| {
                AppError::new("DB_CONFLICT", "预演缺少源文件版本，请重新预演")
            })?;
            if before.size != item["size"].as_u64().unwrap_or(u64::MAX)
                || Some(before.mtime_ms) != item["mtime"].as_i64()
                || before.token != expected_version
            {
                return Err(AppError::new(
                    "DB_CONFLICT",
                    "原文件在预演后已变化，请重新扫描和预演",
                ));
            }
            let input = File::open(&source)?;
            if FileVersion::from_file(&input)? != before {
                return Err(export_source_changed());
            }
            if let Some(parent) = relative.parent() {
                if !parent.as_os_str().is_empty() {
                    dir.create_dir_all(parent).map_err(export_directory_error)?;
                }
            }
            // cap-std resolves relative paths beneath this directory handle, including symlink races.
            let staged = crate::publish::StagedFile::create(&dir, &part, &relative)
                .map_err(export_io_error)?;
            c.execute("UPDATE export_run_items SET state='copying',updated_at=? WHERE job_id=? AND asset_id=?",params![now(),jid,aid])?;
            copy_export_media(staged, &source, input, &before, || {
                tasks::safe_point(cat, jid)
            })
        })();
        match copy {
            Ok((bytes, hash)) => {
                c.execute("UPDATE export_run_items SET state='copied',copied_bytes=?,verified_sha256=?,updated_at=? WHERE job_id=? AND asset_id=?",params![bytes,hash,now(),jid,aid])?;
                if let Some(sidecar) = item.get("xmp").filter(|v| v.is_object()) {
                    c.execute("UPDATE export_sidecar_items SET state='copying',updated_at=? WHERE job_id=? AND asset_id=?",params![now(),jid,aid])?;
                    let result = write_generated(
                        &dir,
                        Path::new(required(sidecar, "relative")?),
                        required(sidecar, "content")?.as_bytes(),
                        || tasks::safe_point(cat, jid),
                    );
                    match result {
                        Ok((bytes, hash)) => {
                            c.execute("UPDATE export_sidecar_items SET state='copied',copied_bytes=?,verified_sha256=?,updated_at=? WHERE job_id=? AND asset_id=?",params![bytes,hash,now(),jid,aid])?;
                        }
                        Err(e) => {
                            c.execute("UPDATE export_sidecar_items SET state='failed',error_code=?,updated_at=? WHERE job_id=? AND asset_id=?",params![e.code,now(),jid,aid])?;
                            if e.code == "CANCELLED" {
                                return Err(e);
                            }
                            failures += 1;
                            tasks::record_error(
                                &c,
                                jid,
                                Some(aid),
                                &format!("XMP 附属文件：{}", e.message),
                            )?;
                        }
                    }
                }
            }
            Err(e) => {
                c.execute("UPDATE export_run_items SET state='failed',error_code=?,updated_at=? WHERE job_id=? AND asset_id=?",params![e.code,now(),jid,aid])?;
                c.execute("UPDATE export_sidecar_items SET state='skipped',error_code='MEDIA_EXPORT_FAILED',updated_at=? WHERE job_id=? AND asset_id=?",params![now(),jid,aid])?;
                if e.code == "CANCELLED" {
                    return Err(e);
                }
                failures += 1;
                tasks::record_error(&c, jid, Some(aid), &e.message)?;
            }
        }
        tasks::progress(&c, jid, index + 1, Some(items.len()), failures)?;
    }
    // Keep CSV streaming and publish only after all rows have been synchronized.
    let manifest = format!("travel-gallery-{jid}.csv");
    let part = format!(".{manifest}.travelgallery-part-{}", id());
    let mut created = false;
    let result = (|| {
        tasks::safe_point(cat, jid)?;
        let mut file = dir.open_with(&part, OpenOptions::new().write(true).create_new(true)).map_err(export_io_error)?;
        created = true;
        file.write_all(&[0xef, 0xbb, 0xbf]).map_err(export_io_error)?;
        file.write_all(b"assetId,filename,result,sha256,type,errorCode\r\n").map_err(export_io_error)?;
        for (kind, table) in [
            ("media", "export_run_items"),
            ("xmp", "export_sidecar_items"),
        ] {
            let mut st=c.prepare(&format!("SELECT asset_id,dest_relative_path,state,COALESCE(verified_sha256,''),COALESCE(error_code,'') FROM {table} WHERE job_id=?"))?;
            for row in st.query_map([jid], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })? {
                let (aid, p, state, hash, error) = row?;
                writeln!(
                    file,
                    "{},{},{},{},{},{}",
                    csv(&aid),
                    csv(&p),
                    csv(&state),
                    csv(&hash),
                    csv(kind),
                    csv(&error)
                ).map_err(export_io_error)?;
            }
        }
        file.sync_all().map_err(export_io_error)?;
        drop(file);
        tasks::safe_point(cat, jid)?;
        crate::publish::no_replace(&dir, Path::new(&part), Path::new(&manifest)).map_err(export_publish_error)?;
        Ok(())
    })();
    if result.is_err() && created { let _ = dir.remove_file(&part); }
    result
}
fn csv(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_string()
    };
    format!("\"{}\"", s.replace('"', "\"\""))
}
pub fn report(c: &Connection, jid: &str) -> Result<Value> {
    let (status, checkpoint): (String, String) = c
        .query_row(
            "SELECT status,COALESCE(checkpoint_json,'{}') FROM jobs WHERE id=? AND type='EXPORT'",
            [jid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::new("INVALID_SELECTION", "复制导出任务不存在"))?;
    let mut st=c.prepare("SELECT asset_id,dest_relative_path,state,error_code,verified_sha256 FROM export_run_items WHERE job_id=? ORDER BY asset_id")?;
    let rows=st.query_map([jid],|r|Ok(json!({"assetId":r.get::<_,String>(0)?,"destination":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"errorCode":r.get::<_,Option<String>>(3)?,"sha256":r.get::<_,Option<String>>(4)?})))?;
    let items = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    let mut statement=c.prepare("SELECT asset_id,dest_relative_path,state,error_code,verified_sha256 FROM export_sidecar_items WHERE job_id=? ORDER BY asset_id")?;
    let sidecars=statement.query_map([jid],|r|Ok(json!({"assetId":r.get::<_,String>(0)?,"destination":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"errorCode":r.get::<_,Option<String>>(3)?,"sha256":r.get::<_,Option<String>>(4)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
    let checkpoint: Value = serde_json::from_str(&checkpoint)?;
    let story = (checkpoint["operation"] == "storyExport").then(|| json!({"folder":checkpoint["folder"],"includeNotes":checkpoint["includeNotes"],"result":checkpoint["storyResult"]}));
    let mut result = json!({"jobId":jid,"copiedCount":items.iter().filter(|i|i["state"]=="copied").count(),"skippedCount":items.iter().filter(|i|i["state"]=="skipped").count(),"failedCount":items.iter().filter(|i|i["state"]=="failed").count(),"verifiedCount":items.iter().filter(|i|i["sha256"].is_string()).count(),"manifestPath":if status=="completed" && story.is_none(){Some(format!("travel-gallery-{jid}.csv"))}else{None},"items":items,"xmpCopiedCount":sidecars.iter().filter(|i|i["state"]=="copied").count(),"xmpFailedCount":sidecars.iter().filter(|i|i["state"]=="failed").count(),"xmpSkippedCount":sidecars.iter().filter(|i|i["state"]=="skipped").count(),"sidecars":sidecars});
    result["status"] = json!(status);
    if let Some(story) = story {
        result["story"] = story;
    }
    Ok(result)
}
pub fn preview_relink(cat: &Catalog, c: &Connection, r: &Value) -> Result<Value> {
    let sid = required(r, "sourceId")?;
    let path = cat.authorized(Path::new(required(r, "selectedNewPath")?), "relink")?;
    assert_not_app_path(cat, &path)?;
    check_source_access(&path)?;
    let old_path: String = c
        .query_row("SELECT absolute_path FROM source_roots WHERE id=?", [sid], |row| row.get(0))
        .optional()?
        .ok_or_else(|| AppError::new("SOURCE_OFFLINE", "来源已不存在"))?;
    let mut roots = c.prepare("SELECT absolute_path FROM source_roots WHERE id<>?")?;
    for root in roots.query_map([sid], |r| r.get::<_, String>(0))? {
        let root = PathBuf::from(root?);
        if path == root
            || (path != Path::new(&old_path) && (path.starts_with(&root) || root.starts_with(&path)))
        {
            return Err(AppError::new("SOURCE_OVERLAP", "新路径与其他来源重叠"));
        }
    }
    let mut s = c.prepare(
        "SELECT id,relative_path,byte_size,mtime_ms,sha256 FROM media_assets WHERE source_id=? ORDER BY id",
    )?;
    let rows = s
        .query_map([sid], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let (mut matched, mut conflicts, mut missing) = (0, 0, 0);
    let mut samples = vec![];
    let mut signature = Sha256::new();
    for row in rows {
        signature.update(serde_json::to_vec(&row)?);
        let (_, rel, size, mtime, hash) = row;
        safe_relative(Path::new(&rel))?;
        let candidate = path.join(&rel);
        let mut ok = false;
        let mut reason = "文件缺失";
        if let Ok(real) = candidate.canonicalize() {
            if real.starts_with(&path) {
                if let Ok((s, t)) = stamp(&real) {
                    ok = s == size && Some(t) == mtime;
                    if ok {
                        if let Some(hash) = hash {
                            ok = sha256(&real, None).is_ok_and(|h| h == hash);
                        }
                    }
                    if ok {
                        matched += 1;
                        reason = "相对路径、大小和时间匹配";
                    } else {
                        conflicts += 1;
                        reason = "文件特征不匹配";
                    }
                } else {
                    missing += 1;
                }
            } else {
                conflicts += 1;
                reason = "路径超出新来源";
            }
        } else {
            missing += 1;
        }
        if samples.len() < 100 {
            samples.push(json!({"relativePath":rel,"matched":ok,"reason":reason}));
        }
    }
    let token = id();
    cat.inner.tokens.lock().unwrap().insert(
        token.clone(),
        json!({"purpose":"relink","sourceId":sid,"oldPath":old_path,"newPath":path,"signature":format!("{:x}",signature.finalize()),"epoch":db::epoch(c)?,"expiresAt":now()+900000,"conflicts":conflicts,"missing":missing}),
    );
    Ok(
        json!({"sourceId":sid,"newPath":path,"matched":matched,"conflicts":conflicts,"missing":missing,"sampleChecks":samples,"verificationToken":token}),
    )
}
pub fn commit_relink(cat: &Catalog, c: &mut Connection, r: &Value) -> Result<Value> {
    let token = required(r, "verificationToken")?;
    let v = cat
        .inner
        .tokens
        .lock()
        .unwrap()
        .get(token)
        .cloned()
        .ok_or_else(|| AppError::new("SNAPSHOT_EXPIRED", "路径验证已过期"))?;
    let epoch = db::epoch(c)?;
    let same_directory = v["oldPath"] == v["newPath"];
    if v["purpose"] != "relink"
        || v["sourceId"] != r["sourceId"]
        || v["expiresAt"].as_i64().unwrap_or(0) < now()
        || v["conflicts"] != 0
        || v["missing"] != 0
        || (!same_directory && v["epoch"] != epoch)
    {
        return Err(AppError::new(
            "DB_CONFLICT",
            "匹配不完整或资料库已变化，请重新验证",
        ));
    }
    let path = cat.authorized(Path::new(required(&v, "newPath")?), "relink")?;
    let checked = preview_relink(
        cat,
        c,
        &json!({"sourceId":r["sourceId"],"selectedNewPath":path}),
    )?;
    let checked_token = required(&checked, "verificationToken")?;
    let verified = cat.inner.tokens.lock().unwrap().remove(checked_token);
    // The picker can restore OS access before confirmation, so normal status polling can
    // advance the catalog epoch. Revalidate the same directory against the confirmed media.
    if checked["conflicts"] != 0
        || checked["missing"] != 0
        || !verified.is_some_and(|entry| {
            entry["oldPath"] == v["oldPath"] && entry["signature"] == v["signature"]
        })
    {
        return Err(AppError::new(
            "DB_CONFLICT",
            "新目录在预演后发生变化，请重新验证",
        ));
    }
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if db::epoch(&tx)? != epoch {
        return Err(AppError::new("DB_CONFLICT", "资料库已变化，请重新验证"));
    }
    tx.execute(
        "UPDATE source_roots SET absolute_path=?,is_online=1,updated_at=? WHERE id=?",
        params![path.to_string_lossy(), now(), required(&v, "sourceId")?],
    )?;
    tx.execute("UPDATE media_assets SET file_state='available' WHERE source_id=? AND file_state IN ('offline','missing')",[required(&v,"sourceId")?])?;
    db::bump(&tx)?;
    tx.commit()?;
    cat.inner.tokens.lock().unwrap().remove(token);
    Ok(json!({"updated":true}))
}
impl Catalog {
    pub(crate) fn prepare_restore(&self, r: &Value) -> Result<Value> {
        let path = self.authorized(Path::new(required(r, "selectedBackupPath")?), "restore")?;
        self.validate_backup(&path)?;
        let token = id();
        self.inner.tokens.lock().unwrap().insert(
            token.clone(),
            json!({"purpose":"restore","path":path,"stamp":stamp(&path)?,"expiresAt":now()+300000}),
        );
        Ok(json!({"confirmToken":token,"message":"恢复前将自动备份当前资料库，备份不包含源媒体"}))
    }
    fn validate_backup(&self, path: &Path) -> Result<i64> {
        if path == self.inner.db_path {
            return Err(AppError::new(
                "INVALID_SELECTION",
                "不能将当前资料库作为恢复文件",
            ));
        }
        let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let check: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        let version: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if check != "ok" || !(2..=db::CATALOG_VERSION).contains(&version) {
            return Err(AppError::new(
                "DB_MIGRATION_FAILED",
                "备份完整性或版本检查失败",
            ));
        }
        for table in [
            "media_assets",
            "asset_reviews",
            "source_roots",
            "tags",
            "jobs",
            "collections",
            "collection_assets",
            "saved_filters",
            "linked_media",
            "similar_groups",
            "similar_group_items",
            "thumbnail_cache",
            "asset_tags",
            "app_settings",
            "change_batches",
            "change_batch_items",
            "selection_snapshots",
            "selection_snapshot_items",
            "mutation_receipts",
            "export_plans",
            "export_run_items",
        ] {
            c.prepare(&format!("SELECT * FROM {table} LIMIT 0"))?;
        }
        c.prepare(&format!("SELECT {}{} LIMIT 0", query::SELECT, query::FROM))?;
        if version >= 4 {
            c.prepare("SELECT asset_id,original_local,original_utc_ms,original_tz_min,original_source,corrected_local,corrected_utc_ms,corrected_tz_min,is_corrected,revision,updated_at FROM capture_time_corrections LIMIT 0")?;
        }
        if version >= 5 {
            c.prepare("SELECT job_id,asset_id,dest_relative_path,state,copied_bytes,error_code,verified_sha256,updated_at FROM export_sidecar_items LIMIT 0")?;
        }
        if version >= 6 {
            c.prepare("SELECT asset_id,undo_revision,restored_revision,change_batch_id FROM review_undo_lineage LIMIT 0")?;
        }
        if version >= 7 {
            c.prepare("SELECT asset_id,model_version,source_size,source_mtime_ms,orientation,input_key,dimensions,vector,created_at FROM ai_embeddings LIMIT 0")?;
            c.prepare("SELECT id,job_id,name,model_version,algorithm_version,created_at FROM ai_clusters LIMIT 0")?;
            c.prepare(
                "SELECT cluster_id,asset_id,input_key,similarity FROM ai_cluster_members LIMIT 0",
            )?;
            crate::ai_models::settings(&c)?;
            let mut vectors = c.prepare("SELECT vector FROM ai_embeddings")?;
            let mut rows = vectors.query([])?;
            while let Some(row) = rows.next()? {
                crate::ai_vectors::unpack(&row.get::<_, Vec<u8>>(0)?)
                    .map_err(|_| AppError::new("DB_MIGRATION_FAILED", "备份的 AI 向量无效"))?;
            }
        }
        if version >= 8 {
            crate::quality::validate_backup(&c).map_err(|_| {
                AppError::new("DB_MIGRATION_FAILED", "备份中的照片检查记录格式无效")
            })?;
        }
        if version >= 10 {
            c.prepare("SELECT file_version,sha256_file_version FROM media_assets LIMIT 0")?;
        }
        crate::export_options::list(&c)?;
        let violations: i64 =
            c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })?;
        if violations != 0 {
            return Err(AppError::new(
                "DB_MIGRATION_FAILED",
                "备份包含损坏的关联记录",
            ));
        }
        Ok(version)
    }
    pub(crate) fn restore(&self, r: &Value) -> Result<Value> {
        let _gate = self
            .inner
            .maintenance
            .try_write()
            .map_err(|_| AppError::new("DB_CONFLICT", "后台仍有写入，请稍后恢复"))?;
        let token = required(r, "confirmToken")?;
        let v = self
            .inner
            .tokens
            .lock()
            .unwrap()
            .get(token)
            .cloned()
            .ok_or_else(|| AppError::new("SNAPSHOT_EXPIRED", "请重新选择备份并确认恢复"))?;
        let selected = self.authorized(Path::new(required(r, "selectedBackupPath")?), "restore")?;
        if serde_json::to_value(stamp(&selected)?)? != v["stamp"] {
            return Err(AppError::new(
                "DB_CONFLICT",
                "备份文件在确认前已变化，请重新选择",
            ));
        }
        if v["purpose"] != "restore"
            || v["expiresAt"].as_i64().unwrap_or(0) < now()
            || Path::new(required(&v, "path")?) != selected
        {
            return Err(AppError::new("PERMISSION_DENIED", "恢复确认无效"));
        }
        let path = self.authorized(Path::new(required(&v, "path")?), "restore")?;
        self.validate_backup(&path)?;
        let mut c = self.connection()?;
        if c.query_row(
            "SELECT EXISTS(SELECT 1 FROM jobs WHERE status IN ('running','queued','paused'))",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(AppError::new("DB_CONFLICT", "请先完成或取消所有任务"));
        }
        let previous_epoch = db::epoch(&c)?;
        db::backup(
            &c,
            &self
                .inner
                .data
                .join(format!("before-restore-{}.sqlite", now())),
        )?;
        let source =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let staged_path = self
            .inner
            .data
            .join(format!("restore-staging-{}.sqlite", id()));
        let restored = (|| -> Result<()> {
            db::backup(&source, &staged_path)?;
            db::initialize(&staged_path)?;
            self.validate_backup(&staged_path)?;
            let staged = Connection::open(&staged_path)?;
            // Invalidate every pre-restore query, even if the backup's epoch is older.
            let next_epoch = db::epoch(&staged)?.max(previous_epoch).checked_add(1)
                .ok_or_else(|| AppError::new("DB_CONFLICT", "资料库版本超出范围，无法安全恢复"))?;
            staged.execute(
                "UPDATE app_meta SET value=? WHERE key='epoch'",
                [next_epoch],
            )?;
            let b = rusqlite::backup::Backup::new(&staged, &mut c)?;
            b.run_to_completion(100, std::time::Duration::from_millis(10), None)?;
            Ok(())
        })();
        let _ = fs::remove_file(&staged_path);
        restored?;
        drop(c);
        db::initialize(&self.inner.db_path)?;
        self.inner.tokens.lock().unwrap().remove(token);
        self.inner.previews.lock().unwrap().clear();
        self.inner.ai.stop();
        self.inner.eyes.stop();
        self.inner.grants.lock().unwrap().clear();
        let mut c = self.connection()?;
        self.clear_cache(&mut c)?;
        Ok(json!({"restoredVersion":db::CATALOG_VERSION}))
    }
}

#[cfg(test)]
mod sidecar_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn same_stamp_source_change_during_copy_or_verification_drops_the_owned_part() {
        for mutate_at in [2, 3] {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source.png");
            fs::write(&source, b"original").unwrap();
            let mtime = fs::metadata(&source).unwrap().modified().unwrap();
            let original_stamp = stamp(&source).unwrap();
            let version = FileVersion::read(&source).unwrap();
            let target = temp.path().join("target");
            fs::create_dir(&target).unwrap();
            fs::write(target.join("keep.txt"), b"existing").unwrap();
            let dir = Dir::open_ambient_dir(&target, ambient_authority()).unwrap();
            let staged = crate::publish::StagedFile::create(
                &dir,
                Path::new(".export.png.part"),
                Path::new("export.png"),
            )
            .unwrap();
            let mut calls = 0;
            let result = copy_export_media(
                staged,
                &source,
                File::open(&source).unwrap(),
                &version,
                || {
                    calls += 1;
                    // For this one-chunk source, call 2 is before EOF and call 3 begins output verification.
                    if calls == mutate_at {
                        fs::write(&source, b"modified").unwrap();
                        File::options()
                            .write(true)
                            .open(&source)
                            .unwrap()
                            .set_times(fs::FileTimes::new().set_modified(mtime))
                            .unwrap();
                        assert_eq!(stamp(&source).unwrap(), original_stamp);
                    }
                    Ok(())
                },
            );
            assert_eq!(result.unwrap_err().code, "DB_CONFLICT");
            assert_eq!(fs::read(&source).unwrap(), b"modified");
            assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
            assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"existing");
        }
    }
    #[test]
    fn destination_errors_keep_space_permission_and_conflicts_distinct() {
        let full=export_io_error(io::Error::from(io::ErrorKind::StorageFull));
        assert_eq!(full.code,"EXPORT_NO_SPACE");
        assert!(full.message.contains("空间"));
        assert_eq!(export_io_error(io::Error::from(io::ErrorKind::PermissionDenied)).code,"PERMISSION_DENIED");
        assert_eq!(export_publish_error(io::Error::from(io::ErrorKind::StorageFull)).code,"EXPORT_NO_SPACE");
        assert_eq!(export_publish_error(io::Error::from(io::ErrorKind::AlreadyExists)).code,"EXPORT_CONFLICT");
        let parent=export_directory_error(io::Error::from(io::ErrorKind::AlreadyExists));
        assert_eq!(parent.code,"EXPORT_TARGET_UNSAFE");
        assert!(parent.message.contains("重新选择"));
    }
    #[test]
    fn cancelled_sidecar_removes_only_its_part_and_never_publishes() {
        let temp = tempfile::tempdir().unwrap();
        let dir = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
        fs::write(temp.path().join("keep.xmp"), b"existing").unwrap();
        let mut calls = 0;
        let result = write_generated(&dir, Path::new("cancel.xmp"), &vec![b'x'; 200_000], || {
            calls += 1;
            if calls == 3 {
                Err(AppError::new("CANCELLED", "cancel during a sidecar write"))
            } else {
                Ok(())
            }
        });
        assert_eq!(result.unwrap_err().code, "CANCELLED");
        assert!(!temp.path().join("cancel.xmp").exists());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        assert_eq!(fs::read(temp.path().join("keep.xmp")).unwrap(), b"existing");
    }
    #[test]
    fn sidecar_publication_race_keeps_the_existing_file_and_cleans_staging() {
        let temp = tempfile::tempdir().unwrap();
        let dir = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
        let target = temp.path().join("race.xmp");
        let mut calls = 0;
        // Before the final publication, another writer claims the destination.
        let result = write_generated(&dir, Path::new("race.xmp"), b"new metadata", || {
            calls += 1;
            if calls == 6 {
                fs::write(&target, b"concurrent file").unwrap();
            }
            Ok(())
        });
        assert_eq!(result.unwrap_err().code, "EXPORT_CONFLICT");
        assert_eq!(fs::read(target).unwrap(), b"concurrent file");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }
    #[test]
    fn sidecar_parent_swap_after_verification_preserves_the_foreign_part() {
        let temp = tempfile::tempdir().unwrap();
        let album = temp.path().join("album");
        let held = temp.path().join("held");
        fs::create_dir(&album).unwrap();
        let dir = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
        let mut calls = 0;
        let mut foreign = None;
        let result = write_generated(&dir, Path::new("album/metadata.xmp"), b"verified metadata", || {
            calls += 1;
            if calls == 6 {
                let part = fs::read_dir(&album).unwrap().next().unwrap().unwrap().file_name();
                fs::rename(&album, &held).unwrap();
                fs::create_dir(&album).unwrap();
                let path = album.join(part);
                fs::write(&path, b"foreign metadata").unwrap();
                foreign = Some(path);
            }
            Ok(())
        });
        assert_eq!(result.unwrap_err().code, "EXPORT_TARGET_UNSAFE");
        assert_eq!(fs::read(foreign.unwrap()).unwrap(), b"foreign metadata");
        assert!(!album.join("metadata.xmp").exists());
        assert_eq!(fs::read_dir(held).unwrap().count(), 0);
    }
    #[test]
    fn sidecar_part_swap_after_verification_is_neither_published_nor_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let dir = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
        let mut calls = 0;
        let mut foreign = None;
        let result = write_generated(&dir, Path::new("metadata.xmp"), b"verified metadata", || {
            calls += 1;
            if calls == 6 {
                let part = fs::read_dir(temp.path()).unwrap().next().unwrap().unwrap().path();
                fs::rename(&part, temp.path().join("held")).unwrap();
                fs::write(&part, b"foreign metadata").unwrap();
                foreign = Some(part);
            }
            Ok(())
        });
        assert_eq!(result.unwrap_err().code, "EXPORT_TARGET_UNSAFE");
        assert_eq!(fs::read(foreign.unwrap()).unwrap(), b"foreign metadata");
        assert_eq!(fs::read(temp.path().join("held")).unwrap(), b"verified metadata");
        assert!(!temp.path().join("metadata.xmp").exists());
    }
    #[cfg(unix)]
    #[test]
    fn sidecar_symlink_escape_cannot_write_outside_the_authorized_directory() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), temp.path().join("escape")).unwrap();
        let dir = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
        assert!(
            write_generated(&dir, Path::new("escape/metadata.xmp"), b"metadata", || Ok(
                ()
            ))
            .is_err()
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}
