//! Model installation/settings. The fixed, data-only package is read from a
//! native-picker grant and copied into app-owned storage; sources stay read-only.
use crate::{
    catalog::Catalog,
    db,
    error::{AppError, Result},
    files, id, model_resources, now, review, tasks,
};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelKind {
    Content,
    Eyes,
}
impl ModelKind {
    pub fn profile(self) -> &'static model_resources::Profile {
        match self {
            Self::Content => model_resources::profile(),
            Self::Eyes => model_resources::eye_profile(),
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Content => "localAiV1",
            Self::Eyes => "localEyesV1",
        }
    }
    pub fn runtime(self, cat: &Catalog) -> &crate::ai_runtime::Runtime {
        match self {
            Self::Content => &cat.inner.ai,
            Self::Eyes => &cat.inner.eyes,
        }
    }
    fn importing(self) -> &'static str {
        match self {
            Self::Content => "aiImportModel",
            Self::Eyes => "eyesImportModel",
        }
    }
    fn removing(self) -> &'static str {
        match self {
            Self::Content => "aiRemoveModel",
            Self::Eyes => "eyesRemoveModel",
        }
    }
    fn installed_path(self, cat: &Catalog) -> PathBuf {
        cat.inner.data.join("models").join(&self.profile().id)
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub format_version: u32,
    pub enabled: bool,
    pub installed_at: Option<i64>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            format_version: 1,
            enabled: false,
            installed_at: None,
        }
    }
}
pub fn settings(c: &Connection) -> Result<Settings> {
    settings_for(c, ModelKind::Content)
}
pub fn settings_for(c: &Connection, kind: ModelKind) -> Result<Settings> {
    let state = match db::setting(c, kind.key())? {
        Some(value) => serde_json::from_str(&value)?,
        None => Settings::default(),
    };
    if state.format_version != 1 || state.installed_at.is_some_and(|at| at < 0) {
        return Err(AppError::new("INVALID_FILTER", "本地 AI 设置格式无效"));
    }
    Ok(state)
}
fn save(c: &Connection, state: &Settings, kind: ModelKind) -> Result<()> {
    c.execute("INSERT INTO app_settings(key,value_json) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",params![kind.key(),serde_json::to_string(state)?])?;
    Ok(())
}
pub fn available(cat: &Catalog, c: &Connection) -> bool {
    available_for(cat, c, ModelKind::Content)
}
pub fn available_for(cat: &Catalog, c: &Connection, kind: ModelKind) -> bool {
    if settings_for(c, kind)
        .ok()
        .and_then(|s| s.installed_at)
        .is_none()
    {
        return false;
    }
    let path = kind.installed_path(cat);
    let Ok(dir) = Dir::open_ambient_dir(path, ambient_authority()) else {
        return false;
    };
    model_resources::validate_marker_for(&dir, kind.profile()).is_ok()
        && kind.profile().files.iter().all(|file| {
            dir.symlink_metadata(&file.path)
                .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && m.len() == file.bytes)
        })
}
pub fn ensure_enabled(cat: &Catalog, c: &Connection) -> Result<()> {
    ensure_enabled_for(cat, c, ModelKind::Content)
}
pub fn ensure_enabled_for(cat: &Catalog, c: &Connection, kind: ModelKind) -> Result<()> {
    if !settings_for(c, kind)?.enabled {
        return Err(AppError::new(
            "AI_DISABLED",
            "本地分析默认关闭，请先在对应页面启用",
        ));
    }
    if !available_for(cat, c, kind) {
        return Err(AppError::new(
            "AI_MODEL_MISSING",
            "离线模型包不可用，请重新导入",
        ));
    }
    Ok(())
}
pub fn ensure_no_job(c: &Connection) -> Result<()> {
    if c.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE status IN ('queued','running','paused') AND json_extract(checkpoint_json,'$.operation') IN ('aiImportModel','aiRemoveModel','aiIndex','aiCluster','eyesImportModel','eyesRemoveModel','qualityAnalyze'))",[],|r|r.get::<_,bool>(0))? {return Err(AppError::new("DB_CONFLICT","已有本地 AI 任务，请先完成或取消该任务"));}
    Ok(())
}
pub fn set_enabled(cat: &Catalog, c: &mut Connection, r: &Value) -> Result<Value> {
    set_enabled_for(cat, c, r, ModelKind::Content)
}
pub fn set_enabled_for(
    cat: &Catalog,
    c: &mut Connection,
    r: &Value,
    kind: ModelKind,
) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        enabled: bool,
    }
    let r: Request = serde_json::from_value(r.clone())?;
    if r.enabled && !available_for(cat, c, kind) {
        return Err(AppError::new(
            "AI_MODEL_MISSING",
            "请先导入完整的离线模型包",
        ));
    }
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut state = settings_for(&tx, kind)?;
    state.enabled = r.enabled;
    if r.enabled && tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE status IN ('queued','running','paused') AND json_extract(checkpoint_json,'$.operation') IN (?,?))",params![kind.importing(),kind.removing()],|r|r.get::<_,bool>(0))? {return Err(AppError::new("DB_CONFLICT","请等待模型导入或移除完成"));}
    if !r.enabled {
        if kind == ModelKind::Content {
            tx.execute("UPDATE jobs SET checkpoint_json=json_set(checkpoint_json,'$.cancelRequested',1),updated_at=? WHERE status IN ('queued','running','paused') AND json_extract(checkpoint_json,'$.operation') IN ('aiIndex','aiCluster')",[now()])?;
        } else {
            tx.execute("UPDATE jobs SET checkpoint_json=json_set(checkpoint_json,'$.cancelRequested',1),updated_at=? WHERE status IN ('queued','running','paused') AND json_extract(checkpoint_json,'$.operation')='qualityAnalyze' AND json_extract(checkpoint_json,'$.useEyes')=1",[now()])?;
        }
    }
    save(&tx, &state, kind)?;
    tx.commit()?;
    if !r.enabled {
        kind.runtime(cat).stop();
    }
    Ok(json!({"updated":true}))
}
fn models_root(cat: &Catalog, c: &Connection) -> Result<Dir> {
    let data = cat.inner.data.canonicalize()?;
    let mut st = c.prepare("SELECT absolute_path FROM source_roots")?;
    for root in st.query_map([], |r| r.get::<_, String>(0))? {
        let root = PathBuf::from(root?);
        let root = root.canonicalize().unwrap_or(root);
        if data.starts_with(&root) || root.starts_with(&data) {
            return Err(AppError::new(
                "EXPORT_TARGET_UNSAFE",
                "模型存储不能与媒体来源目录重叠",
            ));
        }
    }
    let base = Dir::open_ambient_dir(&data, ambient_authority())?;
    if let Err(error) = base.create_dir("models") {
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(error.into());
        }
    }
    if base.symlink_metadata("models")?.file_type().is_symlink()
        || !data.join("models").canonicalize()?.starts_with(&data)
    {
        return Err(AppError::new(
            "EXPORT_TARGET_UNSAFE",
            "模型存储目录不能是重定向路径",
        ));
    }
    Ok(base.open_dir("models")?)
}
pub fn start_import(cat: &Catalog, c: &mut Connection, r: &Value) -> Result<Value> {
    start_import_for(cat, c, r, ModelKind::Content)
}
pub fn start_import_for(
    cat: &Catalog,
    c: &mut Connection,
    r: &Value,
    kind: ModelKind,
) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Request {
        selected_path: PathBuf,
        client_mutation_id: String,
    }
    let request: Request = serde_json::from_value(r.clone())?;
    if request.client_mutation_id.is_empty() {
        return Err(AppError::new("INVALID_FILTER", "操作编号不能为空"));
    }
    let receipt = json!({"operation":kind.importing(),"request":r});
    if let Some(value) = review::receipt(c, &request.client_mutation_id, &receipt)? {
        return Ok(value);
    }
    let source = cat.authorized(&request.selected_path, "model")?;
    let dir = Dir::open_ambient_dir(&source, ambient_authority())?;
    model_resources::validate_marker_for(&dir, kind.profile())?;
    for file in &kind.profile().files {
        let stat = dir.symlink_metadata(&file.path)?;
        if stat.file_type().is_symlink() || !stat.is_file() || stat.len() != file.bytes {
            return Err(model_resources::invalid(
                "模型文件缺失或大小不符，请选择完整离线模型包",
            ));
        }
    }
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(value) = review::receipt(&tx, &request.client_mutation_id, &receipt)? {
        return Ok(value);
    }
    ensure_no_job(&tx)?;
    let mut state = settings_for(&tx, kind)?;
    state.enabled = false;
    save(&tx, &state, kind)?;
    let jid = id();
    let plan =
        json!({"operation":kind.importing(),"selectedPath":source,"profileId":kind.profile().id});
    tasks::insert(&tx, &jid, "REPAIR", &plan)?;
    let result = json!({"jobId":jid});
    review::save_receipt(&tx, &request.client_mutation_id, &receipt, &result)?;
    tx.commit()?;
    kind.runtime(cat).stop();
    cat.spawn_job(jid, "REPAIR".into(), plan);
    Ok(result)
}
pub fn import(cat: &Catalog, jid: &str, r: &Value) -> Result<()> {
    let kind = match r["operation"].as_str() {
        Some("aiImportModel") => ModelKind::Content,
        Some("eyesImportModel") => ModelKind::Eyes,
        _ => return Err(model_resources::invalid("模型导入任务类型无效")),
    };
    if r["profileId"] != kind.profile().id {
        return Err(model_resources::invalid("模型任务版本无效"));
    }
    let mut c = cat.connection()?;
    let source = cat.authorized(
        Path::new(
            r["selectedPath"]
                .as_str()
                .ok_or_else(|| model_resources::invalid("模型包路径无效"))?,
        ),
        "model",
    )?;
    let input = Dir::open_ambient_dir(source, ambient_authority())?;
    model_resources::validate_marker_for(&input, kind.profile())?;
    let root = models_root(cat, &c)?;
    let wanted = kind.profile().files.iter().map(|f| f.bytes).sum::<u64>();
    let total = kind.profile().files.len() + 1;
    tasks::progress(&c, jid, 0, Some(total), 0)?;
    if root.try_exists(&kind.profile().id)? {
        let installed = root.open_dir(&kind.profile().id)?;
        model_resources::validate_files_for(&installed, kind.profile(), || {
            tasks::safe_point(cat, jid)
        })?;
    } else {
        if fs2::available_space(&cat.inner.data)? < wanted + 16 * 1024 * 1024 {
            return Err(AppError::new(
                "EXPORT_NO_SPACE",
                "应用数据磁盘的模型存储空间不足",
            ));
        }
        let staging = format!(".model-import-{}", id());
        root.create_dir(&staging)?;
        let outcome = (|| -> Result<()> {
            let output = root.open_dir(&staging)?;
            for (index, file) in kind.profile().files.iter().enumerate() {
                tasks::safe_point(cat, jid)?;
                let metadata = input.symlink_metadata(&file.path)?;
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || metadata.len() != file.bytes
                {
                    return Err(model_resources::invalid("模型包文件在导入前发生变化"));
                }
                let mut from = input.open(&file.path)?;
                let mut to = output
                    .open_with(&file.path, OpenOptions::new().write(true).create_new(true))?;
                let mut hash = Sha256::new();
                let mut copied = 0u64;
                let mut buffer = vec![0; 1024 * 1024];
                loop {
                    tasks::safe_point(cat, jid)?;
                    let n = from.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    copied += n as u64;
                    if copied > file.bytes {
                        return Err(model_resources::invalid("模型文件超过预期大小"));
                    }
                    to.write_all(&buffer[..n]).map_err(|_| {
                        AppError::new("EXPORT_NO_SPACE", "模型写入失败，请检查磁盘空间和权限")
                    })?;
                    hash.update(&buffer[..n]);
                }
                to.sync_all()?;
                if copied != file.bytes || format!("{:x}", hash.finalize()) != file.sha256 {
                    return Err(model_resources::invalid(
                        "模型文件的 SHA-256 校验失败，未安装此包",
                    ));
                }
                tasks::progress(&c, jid, index + 1, Some(total), 0)?;
            }
            files::write_generated(
                &output,
                Path::new("travel-gallery-model.json"),
                serde_json::to_vec_pretty(kind.profile())?.as_slice(),
                || tasks::safe_point(cat, jid),
            )?;
            model_resources::validate_files_for(&output, kind.profile(), || {
                tasks::safe_point(cat, jid)
            })?;
            tasks::safe_point(cat, jid)?;
            crate::publish::no_replace(&root, Path::new(&staging), Path::new(&kind.profile().id))?;
            Ok(())
        })();
        if outcome.is_err() {
            let _ = root.remove_dir_all(&staging);
        }
        outcome?;
    }
    let tx = c.transaction()?;
    let mut state = settings_for(&tx, kind)?;
    state.enabled = false;
    state.installed_at = Some(now());
    save(&tx, &state, kind)?;
    tx.commit()?;
    tasks::progress(&c, jid, total, Some(total), 0)?;
    Ok(())
}
pub fn start_remove(cat: &Catalog, c: &mut Connection, r: &Value) -> Result<Value> {
    start_remove_for(cat, c, r, ModelKind::Content)
}
pub fn start_remove_for(
    cat: &Catalog,
    c: &mut Connection,
    r: &Value,
    kind: ModelKind,
) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Request {
        confirmed: bool,
        client_mutation_id: String,
    }
    let request: Request = serde_json::from_value(r.clone())?;
    if !request.confirmed || request.client_mutation_id.is_empty() {
        return Err(AppError::new(
            "INVALID_FILTER",
            "请确认移除模型和可重建的 AI 索引",
        ));
    }
    let receipt = json!({"operation":kind.removing(),"request":r});
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(value) = review::receipt(&tx, &request.client_mutation_id, &receipt)? {
        return Ok(value);
    }
    ensure_no_job(&tx)?;
    let mut state = settings_for(&tx, kind)?;
    state.enabled = false;
    save(&tx, &state, kind)?;
    let jid = id();
    let plan = json!({"operation":kind.removing(),"profileId":kind.profile().id});
    tasks::insert(&tx, &jid, "REPAIR", &plan)?;
    let result = json!({"jobId":jid});
    review::save_receipt(&tx, &request.client_mutation_id, &receipt, &result)?;
    tx.commit()?;
    kind.runtime(cat).stop();
    cat.spawn_job(jid, "REPAIR".into(), plan);
    Ok(result)
}
pub fn remove(cat: &Catalog, jid: &str) -> Result<()> {
    remove_for(cat, jid, ModelKind::Content)
}
pub fn remove_for(cat: &Catalog, jid: &str, kind: ModelKind) -> Result<()> {
    kind.runtime(cat).stop();
    let deadline = Instant::now() + Duration::from_secs(10);
    while kind.runtime(cat).phase() != "idle" {
        tasks::safe_point(cat, jid)?;
        if Instant::now() > deadline {
            return Err(AppError::new(
                "DB_CONFLICT",
                "推理进程尚未释放，请稍后重试移除",
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let mut c = cat.connection()?;
    let root = models_root(cat, &c)?;
    let mut allowed = kind
        .profile()
        .files
        .iter()
        .map(|f| f.path.clone())
        .collect::<Vec<_>>();
    allowed.push("travel-gallery-model.json".into());
    if root.try_exists(&kind.profile().id)? {
        if root
            .symlink_metadata(&kind.profile().id)?
            .file_type()
            .is_symlink()
        {
            return Err(AppError::new(
                "EXPORT_TARGET_UNSAFE",
                "模型目录为重定向路径，未删除",
            ));
        }
        let directory = root.open_dir(&kind.profile().id)?;
        for entry in directory.entries()? {
            if !allowed
                .iter()
                .any(|name| entry.as_ref().is_ok_and(|e| e.file_name() == name.as_str()))
            {
                return Err(AppError::new(
                    "DB_CONFLICT",
                    "模型目录包含额外文件，已保留；请移出额外文件后重试",
                ));
            }
        }
        for file in &allowed {
            tasks::safe_point(cat, jid)?;
            if let Err(error) = directory.remove_file(file) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(error.into());
                }
            }
        }
        root.remove_dir(&kind.profile().id)?;
    }
    let tx = c.transaction()?;
    if kind == ModelKind::Content {
        tx.execute("DELETE FROM ai_clusters", [])?;
        tx.execute("DELETE FROM ai_embeddings", [])?;
    } else {
        tx.execute("DELETE FROM quality_observations WHERE kind='eyes'", [])?;
    }
    save(&tx, &Settings::default(), kind)?;
    db::bump(&tx)?;
    tx.commit()?;
    tasks::progress(&c, jid, 1, Some(1), 0)?;
    Ok(())
}
