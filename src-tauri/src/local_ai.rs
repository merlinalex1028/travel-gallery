//! Optional offline semantic search and content grouping. Model output never
//! writes human tags, ratings, favorites, pick decisions, or source media.
use crate::{
    ai_models, ai_vectors,
    catalog::Catalog,
    db,
    error::{AppError, Result},
    files, id, media,
    model_resources::{self, profile},
    now, query, review, tasks,
};
use rusqlite::{params, params_from_iter, types::Value as Sql, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{cmp::Ordering, collections::BinaryHeap, sync::OnceLock};
pub const CURRENT:&str="e.source_size=a.byte_size AND e.source_mtime_ms IS a.mtime_ms AND e.orientation=COALESCE(a.orientation,1)";
pub fn version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| format!("{}/{}", profile().id, model_resources::PREPROCESS_VERSION))
}
fn filter_json(filter: &query::Filter) -> Value {
    let mut value = serde_json::to_value(filter).expect("serializable AI filter");
    if let Some(fields) = value.as_object_mut() {
        fields.retain(|_, value| !value.is_null());
    }
    value
}
fn counts(c: &Connection, filter: &query::Filter) -> Result<(usize, usize)> {
    let (condition, args) = query::scoped_conditions(c, filter)?;
    let mut values = vec![Sql::Text(version().into())];
    values.extend(args);
    Ok(c.query_row(&format!("SELECT count(*),COALESCE(sum(EXISTS(SELECT 1 FROM ai_embeddings e WHERE e.asset_id=a.id AND e.model_version=? AND {CURRENT})),0) {} WHERE {condition}",query::FROM),params_from_iter(values),|r|Ok((r.get(0)?,r.get(1)?)))?)
}
pub fn status(cat: &Catalog, c: &Connection, r: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        #[serde(default)]
        filter: query::Filter,
    }
    let r: Request = serde_json::from_value(r.clone())?;
    let (total, indexed) = counts(c, &r.filter)?;
    let available = ai_models::available(cat, c);
    let state = ai_models::settings(c)?;
    Ok(
        json!({"enabled":state.enabled&&available,"modelAvailable":available,"modelFolderPresent":model_resources::installed_path(&cat.inner.data).exists(),"modelVersion":version(),"profileId":profile().id,"profileName":profile().name,"modelBytes":profile().files.iter().map(|f|f.bytes).sum::<u64>(),"installedAt":state.installed_at,"runtime":cat.inner.ai.phase(),"inferenceThreads":2,"idleReleaseSeconds":60,"total":total,"indexedCount":indexed}),
    )
}
pub fn model_info() -> Value {
    json!({"profile":profile(),"card":include_str!("../resources/ai/MODEL_CARD.md"),"licenseText":include_str!("../resources/ai/LICENSE.txt")})
}
pub fn control(cat: &Catalog, jid: Option<&str>, may_wait: bool) -> Result<()> {
    let c = cat.connection()?;
    if !ai_models::settings(&c)?.enabled {
        return Err(AppError::new("CANCELLED", "本地 AI 已关闭，处理已停止"));
    }
    if let Some(jid) = jid {
        if may_wait {
            tasks::safe_point(cat, jid)?;
        } else {
            let cancelled:bool=c.query_row("SELECT status='cancelled' OR COALESCE(json_extract(checkpoint_json,'$.cancelRequested'),0) FROM jobs WHERE id=?",[jid],|r|r.get(0))?;
            if cancelled {
                return Err(AppError::new("CANCELLED", "本地 AI 任务已取消"));
            }
        }
    }
    Ok(())
}
pub fn start(cat: &Catalog, c: &mut Connection, r: &Value, cluster: bool) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct IndexRequest {
        filter: query::Filter,
        client_mutation_id: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct GroupRequest {
        filter: query::Filter,
        client_mutation_id: String,
        groups: usize,
    }
    let (filter, key, k) = if cluster {
        let r: GroupRequest = serde_json::from_value(r.clone())?;
        (r.filter, r.client_mutation_id, Some(r.groups))
    } else {
        let r: IndexRequest = serde_json::from_value(r.clone())?;
        (r.filter, r.client_mutation_id, None)
    };
    if key.is_empty() || key.len() > 128 {
        return Err(AppError::new("INVALID_FILTER", "操作编号无效"));
    }
    let operation = if cluster { "aiCluster" } else { "aiIndex" };
    let receipt = json!({"operation":operation,"request":r});
    if let Some(result) = review::receipt(c, &key, &receipt)? {
        return Ok(result);
    }
    ai_models::ensure_enabled(cat, c)?;
    ai_models::ensure_no_job(c)?;
    let (count, _) = counts(c, &filter)?;
    let max = if cluster { 10_000 } else { 100_000 };
    if count == 0 || count > max || k.is_some_and(|k| !(2..=20).contains(&k) || k > count) {
        return Err(AppError::new(
            "INVALID_SELECTION",
            if cluster {
                "内容分组须包含 2–10000 项，组数为 2–20 且不能多于成员数"
            } else {
                "内容索引须包含 1–100000 项，请调整范围"
            },
        ));
    }
    let snapshot = query::snapshot(
        c,
        &json!({"filter":filter,"sort":{"field":"filename","direction":"asc"}}),
    )?;
    if snapshot["count"]
        .as_u64()
        .is_none_or(|n| n == 0 || n > max as u64)
    {
        return Err(AppError::new("DB_CONFLICT", "选择范围已变化，请重新开始"));
    }
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(result) = review::receipt(&tx, &key, &receipt)? {
        return Ok(result);
    }
    ai_models::ensure_enabled(cat, &tx)?;
    ai_models::ensure_no_job(&tx)?;
    let jid = id();
    let plan = json!({"operation":operation,"filter":filter_json(&filter),"target":{"kind":"snapshot","snapshotId":snapshot["id"]},"modelVersion":version(),"groups":k,"scopeCount":snapshot["count"]});
    tasks::insert(&tx, &jid, "REPAIR", &plan)?;
    let result = json!({"jobId":jid});
    review::save_receipt(&tx, &key, &receipt, &result)?;
    tx.commit()?;
    cat.spawn_job(jid, "REPAIR".into(), plan);
    Ok(result)
}
#[derive(Clone)]
struct Input {
    size: u64,
    mtime: Option<i64>,
    orientation: i64,
    key: String,
}
fn input(c: &Connection, aid: &str) -> Result<Input> {
    let (size,mtime,orientation,extension):(u64,Option<i64>,i64,String)=c.query_row("SELECT a.byte_size,a.mtime_ms,COALESCE(a.orientation,1),a.extension FROM media_assets a JOIN source_roots s ON s.id=a.source_id WHERE a.id=? AND s.is_enabled=1",[aid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?.ok_or_else(||AppError::new("ASSET_NOT_FOUND","素材已移除或来源已停用"))?;
    let mut descriptor = json!([version(), aid, size, mtime, orientation]);
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
fn existing(c: &Connection, aid: &str, input: &Input) -> Result<Option<Vec<f32>>> {
    let bytes: Option<Vec<u8>> = c
        .query_row(
            "SELECT vector FROM ai_embeddings WHERE asset_id=? AND model_version=? AND input_key=?",
            params![aid, version(), input.key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(bytes.and_then(|bytes| ai_vectors::unpack(&bytes).ok()))
}
fn index_one(
    cat: &Catalog,
    c: &mut Connection,
    jid: &str,
    aid: &str,
) -> Result<(Vec<f32>, String)> {
    let before = input(c, aid)?;
    if let Some(vector) = existing(c, aid, &before)? {
        return Ok((vector, before.key));
    }
    let path = files::source_path(c, aid)?;
    if files::stamp(&path)? != (before.size, before.mtime.unwrap_or(-1)) {
        return Err(AppError::new(
            "DB_CONFLICT",
            "素材已变化，请重新扫描来源后建立索引",
        ));
    }
    let preview = media::thumbnail(cat, c, aid, "screen")?;
    let cache_file = preview
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| AppError::new("AI_INFERENCE_FAILED", "预览缓存标识无效"))?;
    let vector =
        cat.inner
            .ai
            .request(cat, json!({"op":"image","cacheFile":cache_file}), |wait| {
                control(cat, Some(jid), wait)
            })?;
    control(cat, Some(jid), true)?;
    if files::source_path(c, aid)? != path
        || files::stamp(&path)? != (before.size, before.mtime.unwrap_or(-1))
        || input(c, aid)?.key != before.key
    {
        return Err(AppError::new(
            "DB_CONFLICT",
            "素材在内容编码时已变化，未保存此项索引",
        ));
    }
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if !ai_models::settings(&tx)?.enabled {
        return Err(AppError::new("CANCELLED", "本地 AI 已关闭"));
    }
    if input(&tx, aid)?.key != before.key {
        return Err(AppError::new("DB_CONFLICT", "素材元数据已变化"));
    }
    tx.execute("INSERT INTO ai_embeddings VALUES(?,?,?,?,?,?,768,?,?) ON CONFLICT(asset_id) DO UPDATE SET model_version=excluded.model_version,source_size=excluded.source_size,source_mtime_ms=excluded.source_mtime_ms,orientation=excluded.orientation,input_key=excluded.input_key,dimensions=excluded.dimensions,vector=excluded.vector,created_at=excluded.created_at",params![aid,version(),before.size,before.mtime,before.orientation,before.key,ai_vectors::pack(&vector)?,now()])?;
    db::bump(&tx)?;
    tx.commit()?;
    Ok((vector, before.key))
}
pub fn run(cat: &Catalog, jid: &str, r: &Value) -> Result<()> {
    let mut c = cat.connection()?;
    ai_models::ensure_enabled(cat, &c)?;
    if r["modelVersion"] != version() {
        return Err(AppError::new(
            "AI_MODEL_INVALID",
            "此任务的模型版本已变化，请重新开始",
        ));
    }
    let ids = query::target_ids(&c, &r["target"])?;
    let cluster = r["operation"] == "aiCluster";
    if ids.is_empty() || ids.len() > if cluster { 10_000 } else { 100_000 } {
        return Err(AppError::new("INVALID_SELECTION", "本地 AI 任务范围无效"));
    }
    let total = ids.len() + usize::from(cluster);
    let mut failures = 0;
    let mut valid_ids = vec![];
    let mut vectors = vec![];
    let mut keys = vec![];
    tasks::progress(&c, jid, 0, Some(total), 0)?;
    for (i, aid) in ids.iter().enumerate() {
        control(cat, Some(jid), true)?;
        match index_one(cat, &mut c, jid, aid) {
            Ok((vector, key)) => {
                if cluster {
                    valid_ids.push(aid.clone());
                    vectors.push(vector);
                    keys.push(key);
                }
            }
            Err(error) => {
                if [
                    "CANCELLED",
                    "AI_DISABLED",
                    "AI_MODEL_MISSING",
                    "AI_MODEL_INVALID",
                    "AI_RUNTIME_FAILED",
                ]
                .contains(&error.code.as_str())
                {
                    return Err(error);
                }
                failures += 1;
                tasks::record_error(&c, jid, Some(aid), &error.message)?;
            }
        }
        tasks::progress(&c, jid, i + 1, Some(total), failures)?;
    }
    if cluster {
        if vectors.len() < 2 {
            return Err(AppError::new(
                "INVALID_SELECTION",
                "有效内容向量少于 2 项，未发布分组",
            ));
        }
        let k = r["groups"]
            .as_u64()
            .ok_or_else(|| AppError::new("INVALID_FILTER", "分组数量无效"))?
            as usize;
        let (assigned, scores) = ai_vectors::group(&vectors, k.min(vectors.len()), || {
            control(cat, Some(jid), true)
        })?;
        control(cat, Some(jid), true)?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !ai_models::settings(&tx)?.enabled {
            return Err(AppError::new("CANCELLED", "本地 AI 已关闭"));
        }
        let mut group_ids = std::collections::BTreeMap::new();
        for (index, ci) in assigned.iter().enumerate() {
            if input(&tx, &valid_ids[index])?.key != keys[index] {
                return Err(AppError::new(
                    "DB_CONFLICT",
                    "分组期间素材已变化，请重新生成",
                ));
            }
            if !group_ids.contains_key(ci) {
                let gid = id();
                tx.execute(
                    "INSERT INTO ai_clusters VALUES(?,?,?,?,?,?)",
                    params![
                        gid,
                        jid,
                        format!("内容组 {}", group_ids.len() + 1),
                        version(),
                        ai_vectors::CLUSTER_VERSION,
                        now()
                    ],
                )?;
                group_ids.insert(*ci, gid);
            }
            tx.execute(
                "INSERT INTO ai_cluster_members VALUES(?,?,?,?)",
                params![group_ids[ci], valid_ids[index], keys[index], scores[index]],
            )?;
        }
        let summary = json!({"scopeCount":ids.len(),"clusteredCount":vectors.len(),"failedCount":failures,"desiredGroups":k,"actualGroups":group_ids.len()});
        tx.execute("UPDATE jobs SET checkpoint_json=json_set(checkpoint_json,'$.summary',json(?)) WHERE id=?",params![summary.to_string(),jid])?;
        db::bump(&tx)?;
        tx.commit()?;
    }
    tasks::progress(&c, jid, total, Some(total), failures)?;
    Ok(())
}
#[derive(Clone)]
struct Candidate {
    score: f32,
    id: String,
}
impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score && self.id == other.id
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then_with(|| self.id.cmp(&other.id))
    }
}
pub fn search(cat: &Catalog, c: &mut Connection, r: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Request {
        filter: query::Filter,
        text: String,
        limit: usize,
    }
    let r: Request = serde_json::from_value(r.clone())?;
    if !(1..=200).contains(&r.limit) || r.text.trim().is_empty() || r.text.chars().count() > 200 {
        return Err(AppError::new(
            "INVALID_FILTER",
            "请输入 1–200 字的内容描述，结果上限为 1–200",
        ));
    }
    ai_models::ensure_enabled(cat, c)?;
    let (total, indexed) = counts(c, &r.filter)?;
    if total > 100_000 {
        return Err(AppError::new(
            "INVALID_SELECTION",
            "语义搜索范围超过 100000 项，请缩小范围",
        ));
    }
    if indexed == 0 {
        return Ok(
            json!({"items":[],"total":total,"indexedCount":0,"invalidCount":0,"modelVersion":version(),"limit":r.limit}),
        );
    }
    let text = cat
        .inner
        .ai
        .request(cat, json!({"op":"text","text":r.text}), |wait| {
            control(cat, None, wait)
        })?;
    let tx = c.transaction()?;
    let (total, _) = counts(&tx, &r.filter)?;
    if total > 100_000 {
        return Err(AppError::new(
            "INVALID_SELECTION",
            "语义搜索范围已超过 100000 项，请缩小范围",
        ));
    }
    let (condition, mut args) = query::scoped_conditions(&tx, &r.filter)?;
    args.push(Sql::Text(version().into()));
    let mut st=tx.prepare(&format!("SELECT a.id,e.vector {} JOIN ai_embeddings e ON e.asset_id=a.id WHERE ({condition}) AND e.model_version=? AND {CURRENT}",query::FROM))?;
    let mut rows = st.query(params_from_iter(args))?;
    let mut heap = BinaryHeap::new();
    let mut invalid = 0;
    let mut used = 0usize;
    while let Some(row) = rows.next()? {
        if used % 128 == 0 {
            control(cat, None, false)?;
        }
        let bytes: Vec<u8> = row.get(1)?;
        let vector = match ai_vectors::unpack(&bytes) {
            Ok(v) => v,
            Err(_) => {
                invalid += 1;
                continue;
            }
        };
        used += 1;
        let item = Candidate {
            id: row.get(0)?,
            score: ai_vectors::cosine(&text, &vector),
        };
        heap.push(item);
        if heap.len() > r.limit {
            heap.pop();
        }
    }
    drop(rows);
    drop(st);
    let mut items = heap.into_vec();
    items.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    let items = items
        .into_iter()
        .map(|item| Ok(json!({"asset":query::asset(&tx,&item.id)?,"score":item.score})))
        .collect::<Result<Vec<_>>>()?;
    Ok(
        json!({"items":items,"total":total,"indexedCount":used,"invalidCount":invalid,"modelVersion":version(),"limit":r.limit}),
    )
}
pub fn groups(c: &Connection, r: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Request {
        #[serde(default)]
        filter: query::Filter,
        job_id: Option<String>,
    }
    let r: Request = serde_json::from_value(r.clone())?;
    let record: Option<(String, String, i64)> = if let Some(job_id) = r.job_id {
        c.query_row("SELECT id,checkpoint_json,created_at FROM jobs WHERE id=? AND status='completed' AND json_extract(checkpoint_json,'$.operation')='aiCluster'",[job_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?
    } else {
        c.query_row("SELECT id,checkpoint_json,created_at FROM jobs WHERE status='completed' AND json_extract(checkpoint_json,'$.operation')='aiCluster' AND json_extract(checkpoint_json,'$.filter')=? ORDER BY created_at DESC,id DESC LIMIT 1",[serde_json::to_string(&filter_json(&r.filter))?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?
    };
    let Some((job_id, plan, created)) = record else {
        return Ok(
            json!({"jobId":null,"groups":[],"summary":null,"createdAt":null,"filter":filter_json(&r.filter)}),
        );
    };
    let plan: Value = serde_json::from_str(&plan)?;
    let mut st=c.prepare("SELECT id,name,(SELECT count(*) FROM ai_cluster_members m WHERE m.cluster_id=g.id) FROM ai_clusters g WHERE job_id=? ORDER BY created_at,length(name),name,id")?;
    let records = st
        .query_map([&job_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, usize>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(st);
    let mut groups = vec![];
    for (gid, name, count) in records {
        let filter = query::Filter {
            ai_cluster_id: Some(gid.clone()),
            combine_linked: Some(false),
            ..Default::default()
        };
        let (condition, args) = query::scoped_conditions(c, &filter)?;
        let valid: usize = c.query_row(
            &format!("SELECT count(*) {} WHERE {condition}", query::FROM),
            params_from_iter(&args),
            |r| r.get(0),
        )?;
        let mut preview = c.prepare(&format!(
            "SELECT a.id {} WHERE {condition} ORDER BY a.id LIMIT 4",
            query::FROM
        ))?;
        let previews = preview
            .query_map(params_from_iter(&args), |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
            .iter()
            .map(|id| query::asset(c, id))
            .collect::<Result<Vec<_>>>()?;
        groups.push(json!({"id":gid,"name":name,"count":count,"validCount":valid,"staleCount":count-valid,"previews":previews}));
    }
    Ok(
        json!({"jobId":job_id,"groups":groups,"summary":plan["summary"],"createdAt":created,"filter":plan["filter"],"modelVersion":plan["modelVersion"],"algorithmVersion":ai_vectors::CLUSTER_VERSION}),
    )
}
