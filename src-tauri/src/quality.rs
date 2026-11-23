//! Optional background hints. No automatic human review or source writes.
use crate::{
    ai_models::{self, ModelKind},
    catalog::Catalog,
    db,
    error::{AppError, Result},
    files, id, media, model_resources, now, quality_metrics,
    quality_types::{DetailMetrics, EyeHints},
    query, review, tasks,
};
use rusqlite::{params, params_from_iter, types::Value as Sql, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
pub const CURRENT:&str="o.source_size=a.byte_size AND o.source_mtime_ms IS a.mtime_ms AND o.orientation=COALESCE(a.orientation,1)";
#[derive(Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetailSettings {
    pub enabled: bool,
}
pub fn detail_settings(c: &Connection) -> Result<DetailSettings> {
    Ok(db::setting(c, "qualityDetailV1")?
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default())
}
pub fn eye_version() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| {
        format!(
            "{}/{};hint-ear016-blink035-v1",
            model_resources::eye_profile().id,
            model_resources::EYE_PREPROCESS_VERSION
        )
    })
}
fn version(kind: &str) -> Result<&'static str> {
    match kind {
        "eyes" => Ok(eye_version()),
        "detail" => Ok(quality_metrics::VERSION),
        _ => Err(AppError::new("INVALID_FILTER", "分析类型无效")),
    }
}
fn condition(c: &Connection, filter: &query::Filter) -> Result<(String, Vec<Sql>)> {
    let (w, p) = query::scoped_conditions(c, filter)?;
    Ok((
        format!("({w}) AND a.media_kind='image' AND a.file_state<>'missing'"),
        p,
    ))
}
pub fn status(cat: &Catalog, c: &Connection, r: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        #[serde(default)]
        filter: query::Filter,
    }
    let r: Request = serde_json::from_value(r.clone())?;
    let (w, p) = condition(c, &r.filter)?;
    let mut args = vec![
        Sql::Text(eye_version().into()),
        Sql::Text(quality_metrics::VERSION.into()),
    ];
    args.extend(p);
    let (total,eyes,detail):(usize,usize,usize)=c.query_row(&format!("SELECT count(*),COALESCE(sum(EXISTS(SELECT 1 FROM quality_observations o WHERE o.asset_id=a.id AND o.kind='eyes' AND o.algorithm_version=? AND {CURRENT})),0),COALESCE(sum(EXISTS(SELECT 1 FROM quality_observations o WHERE o.asset_id=a.id AND o.kind='detail' AND o.algorithm_version=? AND {CURRENT})),0) {} WHERE {w}",query::FROM),params_from_iter(args),|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    let state = ai_models::settings_for(c, ModelKind::Eyes)?;
    let available = ai_models::available_for(cat, c, ModelKind::Eyes);
    let p = model_resources::eye_profile();
    Ok(
        json!({"eyesEnabled":state.enabled&&available,"detailEnabled":detail_settings(c)?.enabled,"modelAvailable":available,"modelFolderPresent":model_resources::eye_installed_path(&cat.inner.data).exists(),"profileId":p.id,"profileName":p.name,"modelBytes":p.files.iter().map(|f|f.bytes).sum::<u64>(),"installedAt":state.installed_at,"runtime":cat.inner.eyes.phase(),"inferenceThreads":1,"idleReleaseSeconds":60,"total":total,"eyesCount":eyes,"detailCount":detail,"eyeVersion":eye_version(),"detailVersion":quality_metrics::VERSION}),
    )
}
pub fn set_detail_enabled(c: &mut Connection, r: &Value) -> Result<Value> {
    let state: DetailSettings = serde_json::from_value(r.clone())?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute("INSERT INTO app_settings VALUES('qualityDetailV1',?) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",[serde_json::to_string(&state)?])?;
    if !state.enabled {
        tx.execute("UPDATE jobs SET checkpoint_json=json_set(checkpoint_json,'$.cancelRequested',1),updated_at=? WHERE status IN ('queued','running','paused') AND json_extract(checkpoint_json,'$.operation')='qualityAnalyze' AND json_extract(checkpoint_json,'$.useDetail')=1",[now()])?;
    }
    tx.commit()?;
    Ok(json!({"updated":true}))
}
fn enabled(cat: &Catalog, c: &Connection, eyes: bool, detail: bool) -> Result<()> {
    if !eyes && !detail {
        return Err(AppError::new("INVALID_FILTER", "请至少启用一项本地分析"));
    }
    if eyes {
        ai_models::ensure_enabled_for(cat, c, ModelKind::Eyes)?;
    }
    if detail && !detail_settings(c)?.enabled {
        return Err(AppError::new("AI_DISABLED", "细节测量已关闭"));
    }
    Ok(())
}
fn control(cat: &Catalog, jid: &str, eyes: bool, detail: bool, may_wait: bool) -> Result<()> {
    let c = cat.connection()?;
    enabled(cat, &c, eyes, detail)
        .map_err(|_| AppError::new("CANCELLED", "分析已关闭，任务已停止"))?;
    if may_wait{tasks::safe_point(cat,jid)?;}else if c.query_row("SELECT status='cancelled' OR COALESCE(json_extract(checkpoint_json,'$.cancelRequested'),0) FROM jobs WHERE id=?",[jid],|r|r.get::<_,bool>(0))?{return Err(AppError::new("CANCELLED","分析任务已取消"));}
    Ok(())
}
pub fn start(cat: &Catalog, c: &mut Connection, r: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Request {
        filter: query::Filter,
        use_eyes: bool,
        use_detail: bool,
        client_mutation_id: String,
    }
    let r: Request = serde_json::from_value(r.clone())?;
    if r.client_mutation_id.is_empty() || r.client_mutation_id.len() > 128 {
        return Err(AppError::new("INVALID_FILTER", "操作编号无效"));
    }
    let receipt = json!({"operation":"qualityAnalyze","filter":r.filter,"useEyes":r.use_eyes,"useDetail":r.use_detail});
    if let Some(v) = review::receipt(c, &r.client_mutation_id, &receipt)? {
        return Ok(v);
    }
    enabled(cat, c, r.use_eyes, r.use_detail)?;
    ai_models::ensure_no_job(c)?;
    let (w, p) = condition(c, &r.filter)?;
    let count: usize = c.query_row(
        &format!("SELECT count(*){} WHERE {w}", query::FROM),
        params_from_iter(p),
        |r| r.get(0),
    )?;
    if count == 0 || count > 100_000 {
        return Err(AppError::new(
            "INVALID_SELECTION",
            "分析范围须包含 1–100000 张照片，请调整范围",
        ));
    }
    // Preserve all supplied scope constraints while restricting this job to images.
    let mut filter = r.filter.clone();
    filter.media_kinds = Some(vec!["image".into()]);
    filter.file_states = Some(
        r.filter
            .file_states
            .clone()
            .unwrap_or_else(|| {
                vec![
                    "available".into(),
                    "offline".into(),
                    "unsupported".into(),
                    "corrupt".into(),
                ]
            })
            .into_iter()
            .filter(|state| state != "missing")
            .collect(),
    );
    if r.filter
        .media_kinds
        .as_ref()
        .is_some_and(|k| !k.iter().any(|k| k == "image"))
    {
        return Err(AppError::new("INVALID_SELECTION", "当前范围没有照片"));
    }
    let snapshot = query::snapshot(
        c,
        &json!({"filter":filter,"sort":{"field":"filename","direction":"asc"}}),
    )?;
    if snapshot["count"]
        .as_u64()
        .is_none_or(|n| n == 0 || n > 100_000)
    {
        return Err(AppError::new("DB_CONFLICT", "分析范围已变化，请重新开始"));
    }
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(v) = review::receipt(&tx, &r.client_mutation_id, &receipt)? {
        return Ok(v);
    }
    enabled(cat, &tx, r.use_eyes, r.use_detail)?;
    ai_models::ensure_no_job(&tx)?;
    let jid = id();
    let plan = json!({"operation":"qualityAnalyze","target":{"kind":"snapshot","snapshotId":snapshot["id"]},"filter":filter,"useEyes":r.use_eyes,"useDetail":r.use_detail,"eyeVersion":eye_version(),"detailVersion":quality_metrics::VERSION,"scopeCount":snapshot["count"]});
    tasks::insert(&tx, &jid, "REPAIR", &plan)?;
    let result = json!({"jobId":jid});
    review::save_receipt(&tx, &r.client_mutation_id, &receipt, &result)?;
    tx.commit()?;
    cat.spawn_job(jid, "REPAIR".into(), plan);
    Ok(result)
}
#[derive(PartialEq)]
struct Input {
    size: u64,
    mtime: Option<i64>,
    orientation: i64,
    key: String,
}
fn input(c: &Connection, aid: &str, kind: &str) -> Result<Input> {
    let (size,mtime,orientation,extension):(u64,Option<i64>,i64,String)=c.query_row("SELECT a.byte_size,a.mtime_ms,COALESCE(a.orientation,1),a.extension FROM media_assets a JOIN source_roots s ON s.id=a.source_id WHERE a.id=? AND a.media_kind='image' AND a.file_state<>'missing' AND s.is_enabled=1",[aid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?.ok_or_else(||AppError::new("ASSET_NOT_FOUND","照片已移除、缺失或来源已停用"))?;
    let mut descriptor = json!([version(kind)?, aid, size, mtime, orientation]);
    let pixel_version = media::preview_format_version(&extension);
    if pixel_version > 1 {
        descriptor.as_array_mut().unwrap().push(json!(pixel_version));
    }
    let key = format!("{:x}", Sha256::digest(descriptor.to_string()));
    Ok(Input {
        size,
        mtime,
        orientation,
        key,
    })
}
fn validate_value(kind: &str, value: &str) -> Result<Value> {
    let valid = match kind {
        "eyes" => serde_json::from_str::<EyeHints>(value).is_ok_and(|v| v.validate()),
        "detail" => serde_json::from_str::<DetailMetrics>(value).is_ok_and(|v| v.validate()),
        _ => false,
    };
    if !valid {
        return Err(AppError::new(
            "QUALITY_RESULT_INVALID",
            "分析记录格式无效，请重新分析",
        ));
    }
    Ok(serde_json::from_str(value)?)
}
fn analyze_one(
    cat: &Catalog,
    c: &mut Connection,
    jid: &str,
    aid: &str,
    kind: &str,
    eyes: bool,
    detail: bool,
) -> Result<bool> {
    let before = input(c, aid, kind)?;
    let path = files::source_path(c, aid)?;
    if files::stamp(&path)? != (before.size, before.mtime.unwrap_or(-1)) {
        return Err(AppError::new("DB_CONFLICT", "照片已变化，请先重新扫描来源"));
    }
    let existing:Option<String>=c.query_row("SELECT result_json FROM quality_observations WHERE asset_id=? AND kind=? AND algorithm_version=? AND input_key=?",params![aid,kind,version(kind)?,before.key],|r|r.get(0)).optional()?;
    if existing
        .as_ref()
        .is_some_and(|v| validate_value(kind, v).is_ok())
    {
        return Ok(false);
    }
    let preview = media::thumbnail(cat, c, aid, "screen")?;
    control(cat, jid, eyes, detail, true)?;
    let value = if kind == "eyes" {
        let cache_file = preview
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| AppError::new("QUALITY_INFERENCE_FAILED", "预览缓存标识无效"))?;
        serde_json::to_value(cat.inner.eyes.eye_request(
            cat,
            json!({"op":"image","cacheFile":cache_file}),
            |wait| control(cat, jid, eyes, detail, wait),
        )?)?
    } else {
        let cache =
            cap_std::fs::Dir::open_ambient_dir(&cat.inner.cache, cap_std::ambient_authority())?;
        let filename = preview
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| AppError::new("QUALITY_UNSUPPORTED", "预览缓存标识无效"))?;
        serde_json::to_value(quality_metrics::measure(&crate::cache_pixels::read_image(
            &cache, filename,
        )?)?)?
    };
    control(cat, jid, eyes, detail, true)?;
    if files::source_path(c, aid)? != path
        || files::stamp(&path)? != (before.size, before.mtime.unwrap_or(-1))
        || input(c, aid, kind)? != before
    {
        return Err(AppError::new(
            "DB_CONFLICT",
            "照片在分析时发生变化，未保存此项",
        ));
    }
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    enabled(cat, &tx, eyes, detail)?;
    if input(&tx, aid, kind)? != before {
        return Err(AppError::new("DB_CONFLICT", "照片元数据已变化"));
    }
    tx.execute("INSERT INTO quality_observations VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(asset_id,kind) DO UPDATE SET algorithm_version=excluded.algorithm_version,source_size=excluded.source_size,source_mtime_ms=excluded.source_mtime_ms,orientation=excluded.orientation,input_key=excluded.input_key,result_json=excluded.result_json,created_at=excluded.created_at",params![aid,kind,version(kind)?,before.size,before.mtime,before.orientation,before.key,serde_json::to_string(&value)?,now()])?;
    db::bump(&tx)?;
    tx.commit()?;
    Ok(true)
}
pub fn run(cat: &Catalog, jid: &str, r: &Value) -> Result<()> {
    let eyes = r["useEyes"]
        .as_bool()
        .ok_or_else(|| AppError::new("INVALID_FILTER", "分析任务参数无效"))?;
    let detail = r["useDetail"]
        .as_bool()
        .ok_or_else(|| AppError::new("INVALID_FILTER", "分析任务参数无效"))?;
    if r["eyeVersion"] != eye_version() || r["detailVersion"] != quality_metrics::VERSION {
        return Err(AppError::new(
            "QUALITY_RESULT_INVALID",
            "分析版本已变化，请重新开始",
        ));
    }
    let mut c = cat.connection()?;
    enabled(cat, &c, eyes, detail)?;
    let ids = query::target_ids(&c, &r["target"])?;
    if ids.is_empty() || ids.len() > 100_000 {
        return Err(AppError::new("INVALID_SELECTION", "分析任务范围无效"));
    }
    let total = ids.len() * (usize::from(eyes) + usize::from(detail));
    let (mut done, mut failures, mut updated) = (0, 0, 0);
    tasks::progress(&c, jid, 0, Some(total), 0)?;
    for aid in &ids {
        for kind in ["eyes", "detail"] {
            if (kind == "eyes" && !eyes) || (kind == "detail" && !detail) {
                continue;
            }
            control(cat, jid, eyes, detail, true)?;
            match analyze_one(cat, &mut c, jid, aid, kind, eyes, detail) {
                Ok(changed) => updated += usize::from(changed),
                Err(e) => {
                    if [
                        "CANCELLED",
                        "AI_DISABLED",
                        "AI_MODEL_MISSING",
                        "AI_MODEL_INVALID",
                        "AI_RUNTIME_FAILED",
                    ]
                    .contains(&e.code.as_str())
                    {
                        return Err(e);
                    }
                    failures += 1;
                    tasks::record_error(&c, jid, Some(aid), &e.message)?;
                }
            }
            done += 1;
            tasks::progress(&c, jid, done, Some(total), failures)?;
        }
    }
    c.execute("UPDATE jobs SET checkpoint_json=json_set(checkpoint_json,'$.summary',json(?)),updated_at=? WHERE id=?",params![json!({"photos":ids.len(),"updated":updated,"reused":done-failures-updated,"failed":failures}).to_string(),now(),jid])?;
    Ok(())
}
pub fn asset(c: &Connection, aid: &str) -> Result<Value> {
    let current = files::source_path(c, aid)
        .ok()
        .and_then(|p| files::stamp(&p).ok());
    let mut observations = vec![];
    let mut stale = 0;
    let mut st=c.prepare("SELECT kind,algorithm_version,input_key,result_json,created_at,source_size,source_mtime_ms FROM quality_observations WHERE asset_id=? ORDER BY kind")?;
    let rows = st.query_map([aid], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, u64>(5)?,
            r.get::<_, Option<i64>>(6)?,
        ))
    })?;
    for row in rows {
        let (kind, v, key, value, created, size, mtime) = row?;
        if input(c, aid, &kind).is_ok_and(|i| i.key == key)
            && v == version(&kind)?
            && current == Some((size, mtime.unwrap_or(-1)))
        {
            let dismissed=c.query_row("SELECT EXISTS(SELECT 1 FROM quality_hint_reviews WHERE asset_id=? AND kind=? AND input_key=?)",params![aid,kind,key],|r|r.get::<_,bool>(0))?;
            if let Ok(value) = validate_value(&kind, &value) {
                observations.push(json!({"kind":kind,"version":v,"inputKey":key,"value":value,"createdAt":created,"dismissed":dismissed}));
            } else {
                stale += 1;
            }
        } else {
            stale += 1;
        }
    }
    Ok(json!({"assetId":aid,"observations":observations,"staleCount":stale}))
}
pub fn results(c: &Connection, r: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Request {
        #[serde(default)]
        filter: query::Filter,
        #[serde(default)]
        offset: usize,
        #[serde(default)]
        only_hints: bool,
        #[serde(default)]
        include_dismissed: bool,
    }
    let r: Request = serde_json::from_value(r.clone())?;
    if r.offset > 100_000 {
        return Err(AppError::new("INVALID_FILTER", "结果偏移无效"));
    }
    let (mut w, mut p) = condition(c, &r.filter)?;
    let dismissed = if r.include_dismissed {
        ""
    } else {
        " AND NOT EXISTS(SELECT 1 FROM quality_hint_reviews qr WHERE qr.asset_id=o.asset_id AND qr.kind=o.kind AND qr.input_key=o.input_key)"
    };
    let hinted = if r.only_hints {
        " AND o.kind='eyes' AND EXISTS(SELECT 1 FROM json_each(o.result_json,'$.faces') f WHERE json_extract(f.value,'$.status') IN ('possibleClosed','possibleBlink'))"
    } else {
        ""
    };
    w.push_str(&format!(" AND EXISTS(SELECT 1 FROM quality_observations o WHERE o.asset_id=a.id AND {CURRENT} AND ((o.kind='eyes' AND o.algorithm_version=?) OR (o.kind='detail' AND o.algorithm_version=?)){dismissed}{hinted})"));
    p.extend([
        Sql::Text(eye_version().into()),
        Sql::Text(quality_metrics::VERSION.into()),
    ]);
    let total: usize = c.query_row(
        &format!("SELECT count(*){} WHERE {w}", query::FROM),
        params_from_iter(p.iter()),
        |r| r.get(0),
    )?;
    let mut st = c.prepare(&format!(
        "SELECT a.id{} WHERE {w} ORDER BY a.filename COLLATE NOCASE,a.id LIMIT 60 OFFSET ?",
        query::FROM
    ))?;
    p.push(Sql::Integer(r.offset as i64));
    let ids = st
        .query_map(params_from_iter(p), |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut items = vec![];
    for aid in ids {
        let mut analysis = asset(c, &aid)?;
        if !r.include_dismissed {
            analysis["observations"]
                .as_array_mut()
                .unwrap()
                .retain(|v| v["dismissed"] != true);
        }
        items.push(json!({"asset":query::asset(c,&aid)?,"analysis":analysis}));
    }
    Ok(json!({"items":items,"total":total,"offset":r.offset,"pageSize":60}))
}
pub fn dismiss(c: &mut Connection, r: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Request {
        asset_id: String,
        kind: String,
        input_key: String,
        dismissed: bool,
        client_mutation_id: String,
    }
    let request: Request = serde_json::from_value(r.clone())?;
    version(&request.kind)?;
    if request.client_mutation_id.is_empty() || request.client_mutation_id.len() > 128 {
        return Err(AppError::new("INVALID_FILTER", "操作编号无效"));
    }
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let receipt = json!({"operation":"qualityDismiss","request":r});
    if let Some(v) = review::receipt(&tx, &request.client_mutation_id, &receipt)? {
        return Ok(v);
    }
    let analysis = asset(&tx, &request.asset_id)?;
    if !analysis["observations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["kind"] == request.kind && v["inputKey"] == request.input_key)
    {
        return Err(AppError::new("DB_CONFLICT", "此项分析已过期，请重新加载"));
    }
    if request.dismissed {
        tx.execute("INSERT INTO quality_hint_reviews VALUES(?,?,?,?) ON CONFLICT(asset_id,kind,input_key) DO UPDATE SET dismissed_at=excluded.dismissed_at",params![request.asset_id,request.kind,request.input_key,now()])?;
    } else {
        tx.execute(
            "DELETE FROM quality_hint_reviews WHERE asset_id=? AND kind=? AND input_key=?",
            params![request.asset_id, request.kind, request.input_key],
        )?;
    }
    db::bump(&tx)?;
    let result = json!({"updated":true});
    review::save_receipt(&tx, &request.client_mutation_id, &receipt, &result)?;
    tx.commit()?;
    Ok(result)
}
pub fn validate_backup(c: &Connection) -> Result<()> {
    c.prepare("SELECT asset_id,kind,algorithm_version,source_size,source_mtime_ms,orientation,input_key,result_json,created_at FROM quality_observations LIMIT 0")?;
    c.prepare("SELECT asset_id,kind,input_key,dismissed_at FROM quality_hint_reviews LIMIT 0")?;
    detail_settings(c)?;
    ai_models::settings_for(c, ModelKind::Eyes)?;
    let mut st = c.prepare("SELECT kind,result_json FROM quality_observations")?;
    let mut rows = st.query([])?;
    while let Some(row) = rows.next()? {
        validate_value(&row.get::<_, String>(0)?, &row.get::<_, String>(1)?)?;
    }
    Ok(())
}
