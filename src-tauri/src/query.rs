use crate::{
    catalog::Catalog,
    db,
    error::{required, AppError, Result},
    id, now,
};
use rusqlite::{params, params_from_iter, types::Value as Sql, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Filter {
    pub source_ids: Option<Vec<String>>,
    pub media_kinds: Option<Vec<String>>,
    pub extensions: Option<Vec<String>>,
    pub pick_states: Option<Vec<String>>,
    pub rating_min: Option<i64>,
    pub rating_max: Option<i64>,
    pub favorite: Option<bool>,
    pub file_states: Option<Vec<String>>,
    pub capture_from_ms: Option<i64>,
    pub capture_to_ms: Option<i64>,
    pub capture_local_from: Option<String>,
    pub capture_local_to: Option<String>,
    pub capture_date_known: Option<bool>,
    pub gps_cell: Option<[f64; 2]>,
    pub combine_linked: Option<bool>,
    pub min_bytes: Option<i64>,
    pub max_bytes: Option<i64>,
    pub min_width: Option<i64>,
    pub min_height: Option<i64>,
    pub camera_models: Option<Vec<String>>,
    pub tag_rule: Option<TagRule>,
    pub duplicate_kind: Option<String>,
    pub keyword: Option<String>,
    pub folder: Option<String>,
    pub indexed_from_ms: Option<i64>,
    pub indexed_to_ms: Option<i64>,
    pub duration_min_ms: Option<i64>,
    pub duration_max_ms: Option<i64>,
    pub collection_id: Option<String>,
    pub ai_cluster_id: Option<String>,
    pub selection_snapshot_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TagRule {
    pub mode: String,
    pub tag_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Sort {
    pub field: String,
    pub direction: String,
}
impl Default for Sort {
    fn default() -> Self {
        Self {
            field: "captureUtcMs".into(),
            direction: "desc".into(),
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    pub filter: Filter,
    pub sort: Sort,
    pub cursor: Option<String>,
    pub page_size: usize,
    pub selection_snapshot_id: Option<String>,
}
pub const SELECT: &str = "a.id,a.source_id,a.relative_path,a.filename,a.extension,a.media_kind,a.file_state,a.byte_size,a.capture_utc_ms,a.capture_local,a.width,a.height,a.duration_ms,a.camera_model,r.pick_state,r.rating,r.is_favorite,r.note,r.revision,r.updated_at,COALESCE((SELECT 'ready' FROM thumbnail_cache t WHERE t.asset_id=a.id AND t.variant='small'),'queued'),a.indexed_at,a.mtime_ms,(SELECT count(DISTINCT secondary_asset_id) FROM linked_media lm WHERE lm.primary_asset_id=a.id AND lm.confirmed=1),(SELECT min(link_type) FROM linked_media lm WHERE lm.primary_asset_id=a.id AND lm.confirmed=1)";
pub const FROM: &str = " FROM media_assets a JOIN asset_reviews r ON a.id=r.asset_id JOIN source_roots s ON a.source_id=s.id ";
/// Keep original wall dates even without a timezone. File modification dates are not capture dates.
pub const CAPTURE_DATE: &str = "CASE WHEN a.capture_time_source IN ('exif','quicktime','manual') AND date(a.capture_local)=substr(a.capture_local,1,10) THEN substr(a.capture_local,1,10) END";
pub fn cull_progress(c: &Connection, request: &Value) -> Result<Value> {
    let mut filter: Filter = serde_json::from_value(request["filter"].clone())?;
    filter.pick_states = None;
    let (condition, values) = scoped_conditions(c, &filter)?;
    let (source, path, filename, date): (String,String,String,String) = c.query_row(
        "SELECT source_id,relative_path,filename,substr(COALESCE(capture_local,datetime(mtime_ms/1000,'unixepoch','localtime'),'未知日期'),1,10) FROM media_assets WHERE id=?",
        [required(request,"assetId")?], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional()?.ok_or_else(|| AppError::new("INVALID_SELECTION","素材不存在"))?;
    let parent = path.strip_suffix(&filename).unwrap_or("");
    let day = "substr(COALESCE(a.capture_local,datetime(a.mtime_ms/1000,'unixepoch','localtime'),'未知日期'),1,10)=?";
    let folder =
        "a.source_id=? AND substr(a.relative_path,1,length(a.relative_path)-length(a.filename))=?";
    let mut parameters = vec![
        Sql::Text(date.clone()),
        Sql::Text(date.clone()),
        Sql::Text(source.clone()),
        Sql::Text(parent.into()),
        Sql::Text(source),
        Sql::Text(parent.into()),
    ];
    parameters.extend(values);
    let sql = format!("SELECT count(*),COALESCE(sum(r.pick_state<>'unreviewed'),0),COALESCE(sum({day}),0),COALESCE(sum(({day}) AND r.pick_state<>'unreviewed'),0),COALESCE(sum({folder}),0),COALESCE(sum(({folder}) AND r.pick_state<>'unreviewed'),0){FROM}WHERE {condition}");
    Ok(c.query_row(&sql,params_from_iter(parameters),|r| Ok(json!({
        "scope":{"total":r.get::<_,i64>(0)?,"reviewed":r.get::<_,i64>(1)?},
        "day":{"label":date,"total":r.get::<_,i64>(2)?,"reviewed":r.get::<_,i64>(3)?},
        "folder":{"label":if parent.is_empty(){"来源根目录"}else{parent.trim_end_matches('/')},"total":r.get::<_,i64>(4)?,"reviewed":r.get::<_,i64>(5)?}
    })))?)
}
pub fn summary(r: &Row<'_>) -> rusqlite::Result<Value> {
    let state: String = r.get(6)?;
    let thumb: String = r.get(20)?;
    Ok(
        json!({"id":r.get::<_,String>(0)?,"sourceId":r.get::<_,String>(1)?,"relativePath":r.get::<_,String>(2)?,"filename":r.get::<_,String>(3)?,"extension":r.get::<_,String>(4)?,"mediaKind":r.get::<_,String>(5)?,"fileState":state,"byteSize":r.get::<_,i64>(7)?,"captureUtcMs":r.get::<_,Option<i64>>(8)?,"captureLocal":r.get::<_,Option<String>>(9)?,"width":r.get::<_,Option<i64>>(10)?,"height":r.get::<_,Option<i64>>(11)?,"durationMs":r.get::<_,Option<i64>>(12)?,"cameraModel":r.get::<_,Option<String>>(13)?,"thumbnailState":if thumb=="ready" {thumb} else if state=="unsupported" {"unsupported".into()} else if state=="corrupt" {"failed".into()} else {thumb},"review":{"pickState":r.get::<_,String>(14)?,"rating":r.get::<_,i64>(15)?,"isFavorite":r.get::<_,bool>(16)?,"note":r.get::<_,String>(17)?,"revision":r.get::<_,i64>(18)?,"updatedAt":r.get::<_,i64>(19)?},"indexedAt":r.get::<_,i64>(21)?,"mtimeMs":r.get::<_,Option<i64>>(22)?,"linkedMediaCount":r.get::<_,i64>(23)?,"linkedMediaType":r.get::<_,Option<String>>(24)?}),
    )
}
fn list(
    w: &mut Vec<String>,
    p: &mut Vec<Sql>,
    col: &str,
    values: &Option<Vec<String>>,
    allowed: Option<&[&str]>,
) -> Result<()> {
    if let Some(v) = values {
        if v.len() > 200 || allowed.is_some_and(|a| v.iter().any(|x| !a.contains(&x.as_str()))) {
            return Err(AppError::new("INVALID_FILTER", "筛选选项无效"));
        }
        if !v.is_empty() {
            w.push(format!("{col} IN ({})", vec!["?"; v.len()].join(",")));
            p.extend(v.iter().cloned().map(Sql::Text));
        }
    }
    Ok(())
}
pub fn conditions(f: &Filter) -> Result<(String, Vec<Sql>)> {
    conditions_at(f, now())
}

pub fn scoped_conditions(c: &Connection, f: &Filter) -> Result<(String, Vec<Sql>)> {
    scoped_conditions_at(c, f, now())
}

fn scoped_conditions_at(c: &Connection, f: &Filter, at: i64) -> Result<(String, Vec<Sql>)> {
    if let Some(id) = &f.selection_snapshot_id {
        validate_snapshot_with_clock(c, id, || at)?;
    }
    // Use the same expiry cutoff for validation and every SQL predicate in this request.
    conditions_at(f, at)
}

fn conditions_at(f: &Filter, at: i64) -> Result<(String, Vec<Sql>)> {
    let mut w = vec!["s.is_enabled=1".to_string()];
    let mut p = vec![];
    if let Some(group) = &f.ai_cluster_id {
        w.push(format!("a.id IN(SELECT m.asset_id FROM ai_cluster_members m JOIN ai_clusters cc ON cc.id=m.cluster_id JOIN jobs j ON j.id=cc.job_id JOIN ai_embeddings e ON e.asset_id=m.asset_id WHERE m.cluster_id=? AND j.status='completed' AND e.input_key=m.input_key AND e.model_version=? AND cc.model_version=e.model_version AND {})",crate::local_ai::CURRENT));
        p.push(Sql::Text(group.clone()));
        p.push(Sql::Text(crate::local_ai::version().into()));
    }
    if let Some(known) = f.capture_date_known {
        w.push(format!(
            "({CAPTURE_DATE}) IS {}NULL",
            if known { "NOT " } else { "" }
        ));
    }
    for date in [&f.capture_local_from, &f.capture_local_to]
        .into_iter()
        .flatten()
    {
        if date.len() != 10 || chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
            return Err(AppError::new("INVALID_FILTER", "拍摄日期必须为 YYYY-MM-DD"));
        }
    }
    if f.capture_local_from
        .as_ref()
        .zip(f.capture_local_to.as_ref())
        .is_some_and(|(a, b)| a > b)
    {
        return Err(AppError::new("INVALID_FILTER", "范围起点不能大于终点"));
    }
    for (date, op) in [(&f.capture_local_from, ">="), (&f.capture_local_to, "<=")] {
        if let Some(date) = date {
            w.push(format!("substr(COALESCE(a.capture_local,datetime(a.mtime_ms/1000,'unixepoch','localtime')),1,10){op}?"));
            p.push(Sql::Text(date.clone()));
        }
    }
    if let Some([lat, lon]) = f.gps_cell {
        if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90. || lon.abs() > 180. {
            return Err(AppError::new("INVALID_FILTER", "GPS 坐标无效"));
        }
        w.push("round(a.gps_lat,2)=round(?,2) AND round(a.gps_lon,2)=round(?,2)".into());
        p.extend([Sql::Real(lat), Sql::Real(lon)]);
    }
    if let Some(snapshot) = &f.selection_snapshot_id {
        w.push("EXISTS(SELECT 1 FROM selection_snapshot_items si JOIN selection_snapshots ss ON ss.id=si.snapshot_id WHERE si.asset_id=a.id AND si.snapshot_id=? AND ss.expires_at>?)".into());
        p.extend([Sql::Text(snapshot.clone()), Sql::Integer(at)]);
    }
    list(&mut w, &mut p, "a.source_id", &f.source_ids, None)?;
    list(
        &mut w,
        &mut p,
        "a.media_kind",
        &f.media_kinds,
        Some(&["image", "video"]),
    )?;
    list(&mut w, &mut p, "a.extension", &f.extensions, None)?;
    list(
        &mut w,
        &mut p,
        "r.pick_state",
        &f.pick_states,
        Some(&["keep", "maybe", "reject", "unreviewed"]),
    )?;
    list(
        &mut w,
        &mut p,
        "a.file_state",
        &f.file_states,
        Some(&["available", "offline", "missing", "unsupported", "corrupt"]),
    )?;
    list(&mut w, &mut p, "a.camera_model", &f.camera_models, None)?;
    for (a, b) in [
        (f.rating_min, f.rating_max),
        (f.min_bytes, f.max_bytes),
        (f.capture_from_ms, f.capture_to_ms),
        (f.indexed_from_ms, f.indexed_to_ms),
        (f.duration_min_ms, f.duration_max_ms),
    ] {
        if a.zip(b).is_some_and(|(a, b)| a > b) {
            return Err(AppError::new("INVALID_FILTER", "范围起点不能大于终点"));
        }
    }
    if [f.rating_min, f.rating_max]
        .iter()
        .flatten()
        .any(|n| !(0..=5).contains(n))
    {
        return Err(AppError::new("INVALID_FILTER", "星级必须为 0–5"));
    }
    for (col, op, val) in [
        ("r.rating", ">=", f.rating_min),
        ("r.rating", "<=", f.rating_max),
        (
            "COALESCE(a.capture_utc_ms,CASE WHEN a.capture_local IS NULL THEN a.mtime_ms END)",
            ">=",
            f.capture_from_ms,
        ),
        (
            "COALESCE(a.capture_utc_ms,CASE WHEN a.capture_local IS NULL THEN a.mtime_ms END)",
            "<=",
            f.capture_to_ms,
        ),
        ("a.byte_size", ">=", f.min_bytes),
        ("a.byte_size", "<=", f.max_bytes),
        ("a.width", ">=", f.min_width),
        ("a.height", ">=", f.min_height),
        ("a.indexed_at", ">=", f.indexed_from_ms),
        ("a.indexed_at", "<=", f.indexed_to_ms),
        ("a.duration_ms", ">=", f.duration_min_ms),
        ("a.duration_ms", "<=", f.duration_max_ms),
    ] {
        if let Some(n) = val {
            if n < 0
                && [
                    "r.rating",
                    "a.byte_size",
                    "a.width",
                    "a.height",
                    "a.duration_ms",
                ]
                .contains(&col)
            {
                return Err(AppError::new("INVALID_FILTER", "筛选值不能为负数"));
            }
            w.push(format!("{col}{op}?"));
            p.push(Sql::Integer(n));
        }
    }
    if let Some(v) = f.favorite {
        w.push("r.is_favorite=?".into());
        p.push(Sql::Integer(v as i64));
    }
    if let Some(k) = &f.keyword {
        if k.len() > 2000 {
            return Err(AppError::new("INVALID_FILTER", "搜索过长"));
        }
        w.push("(instr(lower(a.filename),lower(?))>0 OR instr(lower(a.relative_path),lower(?))>0 OR EXISTS(SELECT 1 FROM asset_tags at JOIN tags t ON t.id=at.tag_id WHERE at.asset_id=a.id AND instr(lower(t.name),lower(?))>0))".into());
        p.extend([k, k, k].map(|s| Sql::Text(s.clone())));
    }
    if let Some(k) = &f.folder {
        w.push("instr(a.relative_path,?)=1".into());
        p.push(Sql::Text(k.clone()));
    }
    if let Some(c) = &f.collection_id {
        w.push("EXISTS(SELECT 1 FROM collection_assets ca WHERE ca.asset_id=a.id AND ca.collection_id=?)".into());
        p.push(Sql::Text(c.clone()));
    }
    if let Some(t) = &f.tag_rule {
        if !["any", "all", "exclude"].contains(&t.mode.as_str()) || t.tag_ids.len() > 200 {
            return Err(AppError::new("INVALID_FILTER", "标签规则无效"));
        }
        if !t.tag_ids.is_empty() {
            let base=format!("SELECT count(DISTINCT tag_id) FROM asset_tags WHERE asset_id=a.id AND tag_id IN ({})",vec!["?";t.tag_ids.len()].join(","));
            w.push(format!(
                "({base}){}",
                match t.mode.as_str() {
                    "all" => format!("={}", t.tag_ids.len()),
                    "exclude" => "=0".into(),
                    _ => ">0".into(),
                }
            ));
            p.extend(t.tag_ids.iter().cloned().map(Sql::Text));
        }
    }
    if let Some(k) = &f.duplicate_kind {
        if !["exact", "similar", "any", "none"].contains(&k.as_str()) {
            return Err(AppError::new("INVALID_FILTER", "重复类型无效"));
        }
        let mut cond="EXISTS(SELECT 1 FROM similar_group_items gi JOIN similar_groups g ON g.id=gi.group_id WHERE gi.asset_id=a.id AND gi.manually_excluded=0".to_string();
        if k == "exact" || k == "similar" {
            cond.push_str(" AND g.group_type=?");
            p.push(Sql::Text(k.clone()));
        }
        cond.push_str(
            " AND (g.group_type<>'exact' OR g.id NOT IN (
                SELECT sibling.group_id FROM similar_group_items sibling JOIN media_assets m ON m.id=sibling.asset_id
                WHERE m.sha256 IS NULL OR m.sha256_byte_size IS NOT m.byte_size
                    OR m.sha256_mtime_ms IS NOT m.mtime_ms OR m.sha256_file_version IS NULL
                    OR m.sha256_file_version IS NOT m.file_version))",
        );
        cond.push(')');
        if k == "none" {
            cond = format!("NOT {cond}");
        }
        w.push(cond);
    }
    if f.combine_linked.unwrap_or(true) {
        let mut expanded = f.clone();
        expanded.combine_linked = Some(false);
        let (primary_where, args) = conditions_at(&expanded, at)?;
        w.push(format!("NOT EXISTS(SELECT 1 FROM linked_media lm WHERE lm.secondary_asset_id=a.id AND lm.confirmed=1 AND EXISTS(SELECT 1{FROM}WHERE a.id=lm.primary_asset_id AND {primary_where}))"));
        p.extend(args);
    }
    Ok((w.join(" AND "), p))
}
pub fn sort_column(s: &Sort) -> Result<&'static str> {
    if !["asc", "desc"].contains(&s.direction.as_str()) {
        return Err(AppError::new("INVALID_FILTER", "排序方向无效"));
    }
    match s.field.as_str() {
        "captureUtcMs" => Ok("COALESCE(a.capture_local,datetime(a.mtime_ms/1000,'unixepoch'),'' )"),
        "indexedAt" => Ok("a.indexed_at"),
        "filename" => Ok("a.filename COLLATE NOCASE"),
        "byteSize" => Ok("a.byte_size"),
        "rating" => Ok("r.rating"),
        _ => Err(AppError::new("INVALID_FILTER", "排序字段无效")),
    }
}
pub fn fingerprint(f: &Filter, s: &Sort) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(f, s))?)
    ))
}
fn key_value(v: &Value) -> Result<Sql> {
    match v {
        Value::String(s) => Ok(Sql::Text(s.clone())),
        Value::Number(n) => {
            Ok(Sql::Integer(n.as_i64().ok_or_else(|| {
                AppError::new("INVALID_FILTER", "游标数值无效")
            })?))
        }
        _ => Err(AppError::new("INVALID_FILTER", "游标无效")),
    }
}
pub fn query(c: &Connection, q: Query) -> Result<Value> {
    let fp = fingerprint(&q.filter, &q.sort)?;
    let tx = c.unchecked_transaction()?;
    let result = query_page(&tx, &q, &fp, now())?;
    tx.commit()?;
    // Save the query definition after releasing the read snapshot. The fingerprint
    // identifies the definition independently of the version of the returned rows.
    c.execute(
        "INSERT OR REPLACE INTO query_contexts VALUES(?,?,?)",
        params![
            fp,
            json!({"filter":q.filter,"sort":q.sort}).to_string(),
            now() + 86_400_000
        ],
    )?;
    Ok(result)
}
fn query_page(c: &Connection, q: &Query, fp: &str, at: i64) -> Result<Value> {
    if let Some(id) = &q.selection_snapshot_id {
        validate_snapshot_with_clock(c, id, || at)?;
    }
    let (w, mut p) = scoped_conditions_at(c, &q.filter, at)?;
    if !(1..=500).contains(&q.page_size) {
        return Err(AppError::new("INVALID_FILTER", "每页数量须为 1–500"));
    }
    let col = sort_column(&q.sort)?;
    let epoch = db::epoch(c)?;
    // A stale cursor can be rejected without counting the current matches.
    let cursor = q
        .cursor
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()?;
    if cursor.as_ref().is_some_and(|v| v["fingerprint"] != fp || v["epoch"] != epoch) {
        return Err(AppError::new(
            "INVALID_FILTER",
            "资料库或筛选已变化，请刷新结果",
        ));
    }
    let total: i64 = c.query_row(
        &format!("SELECT count(*){FROM}WHERE {w}"),
        params_from_iter(p.iter()),
        |r| r.get(0),
    )?;
    let mut cursor_cond = String::new();
    if let Some(v) = &cursor {
        let op = if q.sort.direction == "asc" { ">" } else { "<" };
        cursor_cond = format!(" AND (({col}) {op} ? OR (({col})=? AND a.id {op} ?))");
        p.push(key_value(&v["key"])?);
        p.push(key_value(&v["key"])?);
        p.push(Sql::Text(required(v, "id")?.into()));
    }
    let sql=format!("SELECT {SELECT},({col}) AS cursor_key{FROM}WHERE {w}{cursor_cond} ORDER BY {col} {},a.id {} LIMIT ?",q.sort.direction,q.sort.direction);
    p.push(Sql::Integer(q.page_size as i64 + 1));
    let mut st = c.prepare(&sql)?;
    let rows = st.query_map(params_from_iter(p), |r| {
        let v = summary(r)?;
        let key = match r.get_ref("cursor_key")? {
            rusqlite::types::ValueRef::Integer(n) => json!(n),
            rusqlite::types::ValueRef::Text(s) => json!(String::from_utf8_lossy(s)),
            _ => Value::Null,
        };
        Ok((v, key))
    })?;
    let mut entries = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    let more = entries.len() > q.page_size;
    entries.truncate(q.page_size);
    let next = if more {
        entries
            .last()
            .map(|(a, k)| json!({"fingerprint":fp,"epoch":epoch,"id":a["id"],"key":k}).to_string())
    } else {
        None
    };
    let mut result = json!({"items":entries.into_iter().map(|(a,_)|a).collect::<Vec<_>>(),"total":total,"nextCursor":next,"queryFingerprint":fp,"catalogEpoch":epoch});
    if let Some(id) = &q.selection_snapshot_id {
        let ids: Vec<&Value> = result["items"].as_array().unwrap().iter().map(|a| &a["id"]).collect();
        let mut statement = c.prepare("SELECT asset_id FROM selection_snapshot_items WHERE snapshot_id=? AND asset_id IN(SELECT value FROM json_each(?)) ORDER BY asset_id")?;
        let members = statement.query_map(params![id, serde_json::to_string(&ids)?], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        result["selectionMembership"] = json!({"snapshotId":id,"assetIds":members});
    }
    Ok(result)
}

#[cfg(test)]
mod query_transaction_tests {
    use super::*;

    fn ordering_fixture() -> (tempfile::TempDir, Connection) {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("catalog.sqlite");
        db::initialize(&database).unwrap();
        let c = db::connect(&database).unwrap();
        c.execute("INSERT INTO source_roots(id,display_name,absolute_path,created_at,updated_at)
            VALUES('source','Synthetic ordering fixture','/synthetic-only',0,0)", []).unwrap();
        for (aid, name, local, mtime, indexed, size, rating) in [
            ("a", "IMG.jpg", None, None, 10, 10, 0),
            ("b", "img.JPG", None, Some(0), 10, 10, 5),
            ("c", "IMG.JPG", Some("2024-01-01 00:00:00"), Some(0), 5, 30, 5),
            ("d", "alpha.jpg", None, Some(0), 0, 0, 2),
            ("e", "Zulu.jpg", Some("2024-01-01 00:00:00"), None, 5, 30, 0),
            ("f", "zulu.JPG", None, None, 5, 20, 2),
            ("g", "Beta.jpg", Some("2023-01-01 00:00:00"), None, 10, 20, 4),
            ("h", "beta.JPG", None, Some(-1000), 0, 10, 4),
        ] {
            c.execute("INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,
                byte_size,mtime_ms,capture_local,capture_time_source,indexed_at,updated_at)
                VALUES(?,'source',?,?,'jpg','image',?,?,?,?,?,0)",
                params![aid,format!("{aid}/{name}"),name,size,mtime,local,local.map(|_| "exif"),indexed]).unwrap();
            c.execute("UPDATE asset_reviews SET rating=? WHERE asset_id=?", params![rating,aid]).unwrap();
        }
        (temp, c)
    }

    #[test]
    fn every_sort_pages_null_dates_and_equal_keys_in_both_directions() {
        let (_temp, c) = ordering_fixture();
        let epoch = db::epoch(&c).unwrap();
        for known in [None, Some(true), Some(false)] {
            for (field, ascending) in [
                ("captureUtcMs", "afhbdgce"),
                ("indexedAt", "dhcefabg"),
                ("filename", "dghabcef"),
                ("byteSize", "dabhfgce"),
                ("rating", "aedfghbc"),
            ] {
                for direction in ["asc", "desc"] {
                    // Explicit expected orders are independent of the SQL keyset comparison.
                    let mut expected: Vec<String> = ascending.chars()
                        .filter(|aid| known.is_none_or(|known| "ceg".contains(*aid) == known))
                        .map(|aid| aid.to_string()).collect();
                    if direction == "desc" {
                        expected.reverse();
                    }
                    let mut seen = Vec::new();
                    let mut cursor = None;
                    let mut pages = 0;
                    loop {
                        let result = query(&c, Query {
                            filter: Filter { capture_date_known: known, ..Filter::default() },
                            sort: Sort { field: field.into(), direction: direction.into() },
                            cursor,
                            page_size: 3,
                            selection_snapshot_id: None,
                        }).unwrap();
                        assert_eq!(result["catalogEpoch"], epoch);
                        assert_eq!(result["total"], expected.len());
                        seen.extend(result["items"].as_array().unwrap().iter()
                            .map(|asset| asset["id"].as_str().unwrap().to_owned()));
                        pages += 1;
                        cursor = result["nextCursor"].as_str().map(str::to_owned);
                        if cursor.is_none() {
                            break;
                        }
                        assert!(pages < 3, "Cursor did not terminate: {known:?} {field} {direction}");
                    }
                    assert_eq!(seen, expected, "{known:?} {field} {direction}");
                    assert_eq!(pages, expected.len().div_ceil(3));
                }
            }
        }
    }

    #[test]
    fn changed_query_and_epoch_reject_old_cursors_without_replacing_the_context() {
        let (_temp, c) = ordering_fixture();
        let request = |filter, direction: &str, cursor| Query {
            filter,
            sort: Sort { field: "filename".into(), direction: direction.into() },
            cursor,
            page_size: 3,
            selection_snapshot_id: None,
        };
        let first = query(&c, request(Filter::default(), "asc", None)).unwrap();
        let cursor = first["nextCursor"].as_str().unwrap().to_owned();
        let context = || c.query_row("SELECT fingerprint,request_json,expires_at FROM query_contexts",
            [], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, i64>(2)?))).unwrap();
        let saved = context();
        for (filter, direction) in [
            (Filter { keyword: Some("IMG".into()), ..Filter::default() }, "asc"),
            (Filter::default(), "desc"),
        ] {
            assert_eq!(query(&c, request(filter, direction, Some(cursor.clone()))).unwrap_err().code, "INVALID_FILTER");
            assert_eq!(context(), saved);
        }
        db::bump(&c).unwrap();
        assert_eq!(query(&c, request(Filter::default(), "asc", Some(cursor))).unwrap_err().code, "INVALID_FILTER");
        assert_eq!(context(), saved);
        let refreshed = query(&c, request(Filter::default(), "asc", None)).unwrap();
        assert_eq!(refreshed["catalogEpoch"], first["catalogEpoch"].as_i64().unwrap() + 1);
        assert_eq!(refreshed["items"], first["items"]);
        assert!(query(&c, request(Filter::default(), "asc", refreshed["nextCursor"].as_str().map(str::to_owned))).is_ok());
    }

    #[test]
    fn snapshot_epoch_rejection_keeps_old_members_and_accepts_a_refreshed_query() {
        let (_temp, mut c) = ordering_fixture();
        c.execute("UPDATE asset_reviews SET is_favorite=1 WHERE asset_id='b'", []).unwrap();
        let read = |c: &Connection| query(c, Query {
            filter: Filter { favorite: Some(true), ..Filter::default() },
            sort: Sort::default(),
            cursor: None,
            page_size: 3,
            selection_snapshot_id: None,
        }).unwrap();
        let freeze = |c: &mut Connection, epoch: Value| snapshot(c, &json!({
            "filter":{"favorite":true},"sort":Sort::default(),"expectedCatalogEpoch":epoch
        }));
        let displayed = read(&c);
        let original = freeze(&mut c, displayed["catalogEpoch"].clone()).unwrap();
        let tx = c.transaction().unwrap();
        tx.execute("UPDATE asset_reviews SET is_favorite=(asset_id='h') WHERE asset_id IN ('b','h')", []).unwrap();
        db::bump(&tx).unwrap();
        tx.commit().unwrap();
        for stale in [displayed["catalogEpoch"].clone(), Value::Null, json!(-1)] {
            assert_eq!(freeze(&mut c, stale).unwrap_err().code, "INVALID_FILTER");
        }
        assert_eq!(c.query_row("SELECT count(*) FROM selection_snapshots", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        let refreshed = read(&c);
        assert_eq!(refreshed["total"], displayed["total"]);
        assert_eq!(refreshed["queryFingerprint"], displayed["queryFingerprint"]);
        let updated = freeze(&mut c, refreshed["catalogEpoch"].clone()).unwrap();
        for (frozen, member) in [(original, "b"), (updated, "h")] {
            assert_eq!(frozen["count"], 1);
            let actual: String = c.query_row("SELECT asset_id FROM selection_snapshot_items WHERE snapshot_id=?",
                [frozen["id"].as_str().unwrap()], |row| row.get(0)).unwrap();
            assert_eq!(actual, member);
        }
    }

    #[test]
    fn snapshot_consumers_share_an_exclusive_expiry_boundary() {
        let (_temp, mut c) = ordering_fixture();
        let frozen = snapshot(&mut c, &json!({"filter":{},"sort":Sort::default()})).unwrap();
        let deadline = frozen["expiresAt"].as_i64().unwrap();
        assert_eq!(deadline - frozen["createdAt"].as_i64().unwrap(), 86_400_000);
        let target = json!({"kind":"snapshot","snapshotId":frozen["id"],"excludeAssetIds":["a"]});
        assert_eq!(target_ids_with_clock(&c, &target, || deadline - 1).unwrap(),
                   ["b", "c", "d", "e", "f", "g", "h"]);
        validate_snapshot_with_clock(&c, frozen["id"].as_str().unwrap(), || deadline - 1).unwrap();
        for at in [deadline, deadline + 1] {
            assert_eq!(target_ids_with_clock(&c, &target, || at).unwrap_err().code, "SNAPSHOT_EXPIRED");
            assert_eq!(validate_snapshot_with_clock(&c, frozen["id"].as_str().unwrap(), || at).unwrap_err().code,
                       "SNAPSHOT_EXPIRED");
        }
    }

    #[test]
    fn snapshot_scoped_page_uses_the_same_deadline_for_validation_and_filtering() {
        let (_temp, mut c) = ordering_fixture();
        let frozen = snapshot(&mut c, &json!({"filter":{},"sort":Sort::default()})).unwrap();
        let id = frozen["id"].as_str().unwrap();
        // Model expiry during the query without changing the system clock or sleeping.
        let deadline = now() - 1;
        c.execute("UPDATE selection_snapshots SET expires_at=? WHERE id=?", params![deadline, id]).unwrap();
        let request = Query {
            filter: Filter { selection_snapshot_id: Some(id.into()), ..Filter::default() },
            sort: Sort { field: "filename".into(), direction: "asc".into() },
            cursor: None,
            page_size: 3,
            selection_snapshot_id: Some(id.into()),
        };
        let fp = fingerprint(&request.filter, &request.sort).unwrap();
        let read = c.unchecked_transaction().unwrap();
        let page = query_page(&read, &request, &fp, deadline - 1).unwrap();
        assert_eq!(page["total"], 8);
        assert_eq!(page["items"].as_array().unwrap().len(), 3);
        assert_eq!(page["selectionMembership"], json!({"snapshotId":id,"assetIds":["d","g","h"]}));
        assert!(page["nextCursor"].is_string());
        assert_eq!(query_page(&read, &request, &fp, deadline).unwrap_err().code, "SNAPSHOT_EXPIRED");
    }

    #[test]
    fn page_identifies_which_snapshot_expired() {
        let (_temp, mut c) = ordering_fixture();
        let request = json!({"filter":{},"sort":Sort::default()});
        let scope = snapshot(&mut c, &request).unwrap();
        let selection = snapshot(&mut c, &request).unwrap();
        let scope_id = scope["id"].as_str().unwrap();
        let selection_id = selection["id"].as_str().unwrap();
        for expired_id in [scope_id, selection_id] {
            c.execute("UPDATE selection_snapshots SET expires_at=? WHERE id IN (?,?)",
                params![now() + 86_400_000, scope_id, selection_id]).unwrap();
            c.execute("UPDATE selection_snapshots SET expires_at=0 WHERE id=?", [expired_id]).unwrap();
            let error = query(&c, Query {
                filter: Filter { selection_snapshot_id: Some(scope_id.into()), ..Filter::default() },
                sort: Sort::default(),
                cursor: None,
                page_size: 3,
                selection_snapshot_id: Some(selection_id.into()),
            }).unwrap_err();
            assert_eq!(error.code, "SNAPSHOT_EXPIRED");
            assert_eq!(error.details, json!({"snapshotId": expired_id}));
        }
    }

    #[test]
    fn page_keeps_its_epoch_and_rows_while_a_catalog_update_commits() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("catalog.sqlite");
        db::initialize(&database).unwrap();
        let mut writer = db::connect(&database).unwrap();
        writer.busy_timeout(std::time::Duration::ZERO).unwrap();
        writer.execute_batch("INSERT INTO source_roots(id,display_name,absolute_path,created_at,updated_at)
            VALUES('source','Fixture','/fixture',0,0);
            INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,mtime_ms,indexed_at,updated_at)
            VALUES('asset','source','before.jpg','before.jpg','jpg','image',1,0,0,0);").unwrap();

        let reader = db::connect(&database).unwrap();
        let page = || Query {
            filter: Filter::default(),
            sort: Sort::default(),
            cursor: None,
            page_size: 120,
            selection_snapshot_id: None,
        };
        let request = page();
        let fp = fingerprint(&request.filter, &request.sort).unwrap();
        let read = reader.unchecked_transaction().unwrap();
        let epoch = db::epoch(&read).unwrap();

        let tx = writer.transaction().unwrap();
        tx.execute("UPDATE media_assets SET filename='after.jpg' WHERE id='asset'", []).unwrap();
        db::bump(&tx).unwrap();
        tx.commit().unwrap();

        let before = query_page(&read, &request, &fp, now()).unwrap();
        assert_eq!(before["catalogEpoch"], epoch);
        assert_eq!(before["items"][0]["filename"], "before.jpg");
        assert_eq!(before["total"], 1);
        read.commit().unwrap();
        let after = query(&reader, page()).unwrap();
        assert_eq!(after["catalogEpoch"], epoch + 1);
        assert_eq!(after["items"][0]["filename"], "after.jpg");
        let saved: String = reader.query_row(
            "SELECT request_json FROM query_contexts WHERE fingerprint=?",
            [&fp],
            |row| row.get(0),
        ).unwrap();
        let saved: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(saved, json!({"filter":request.filter,"sort":request.sort}));
    }
}

pub fn asset(c: &Connection, asset_id: &str) -> Result<Value> {
    c.query_row(
        &format!("SELECT {SELECT}{FROM}WHERE a.id=?"),
        [asset_id],
        summary,
    )
    .optional()?
    .ok_or_else(|| AppError::new("ASSET_NOT_FOUND", "照片已不在资料库中"))
}
pub fn snapshot(c: &mut Connection, r: &Value) -> Result<Value> {
    let f: Filter = serde_json::from_value(r["filter"].clone())?;
    let s: Sort = serde_json::from_value(r["sort"].clone())?;
    sort_column(&s)?;
    let key = fingerprint(&f, &s)?;
    let snap = id();
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let at = now();
    if let Some(expected) = r.get("expectedCatalogEpoch") {
        let expected = expected
            .as_i64()
            .filter(|value| *value >= 0)
            .ok_or_else(|| AppError::new("INVALID_FILTER", "资料库版本无效，请刷新结果后重新选择"))?;
        if expected != db::epoch(&tx)? {
            return Err(AppError::new(
                "INVALID_FILTER",
                "资料库已变化，请刷新结果后重新选择",
            ));
        }
    }
    let (mut w, p) = scoped_conditions_at(&tx, &f, at)?;
    if let Some(target) = r.get("target") {
        let ids = target_ids_with_clock(&tx, target, || at)?;
        tx.execute_batch("CREATE TEMP TABLE frozen_target_ids(id TEXT PRIMARY KEY)")?;
        let mut insert = tx.prepare("INSERT INTO frozen_target_ids VALUES(?)")?;
        for id in ids {
            insert.execute([id])?;
        }
        w.push_str(" AND a.id IN(SELECT id FROM frozen_target_ids)");
    }
    tx.execute(
        "INSERT INTO selection_snapshots VALUES(?,?,?,?,?,?,?)",
        params![
            snap,
            serde_json::to_string(&f)?,
            serde_json::to_string(&s)?,
            key,
            0,
            at,
            at + 86_400_000
        ],
    )?;
    let mut args = vec![Sql::Text(snap.clone())];
    args.extend(p);
    let count = tx.execute(
        &format!("INSERT INTO selection_snapshot_items SELECT ?,a.id{FROM}WHERE {w}"),
        params_from_iter(args),
    )?;
    tx.execute(
        "UPDATE selection_snapshots SET initial_count=? WHERE id=?",
        params![count, snap],
    )?;
    if r.get("target").is_some() {
        tx.execute_batch("DROP TABLE frozen_target_ids")?;
    }
    tx.commit()?;
    Ok(
        json!({"id":snap,"count":count,"createdAt":at,"expiresAt":at+86_400_000,"queryFingerprint":key}),
    )
}
pub fn target_ids(c: &Connection, t: &Value) -> Result<Vec<String>> {
    target_ids_with_clock(c, t, now)
}
fn target_ids_with_clock(c: &Connection, t: &Value, clock: impl FnOnce() -> i64) -> Result<Vec<String>> {
    let ids = match required(t, "kind")? {
        "explicit" => {
            let v = t["assetIds"]
                .as_array()
                .ok_or_else(|| AppError::new("INVALID_SELECTION", "选区无效"))?;
            if v.len() > 10000 {
                return Err(AppError::new("INVALID_SELECTION", "大量选择请使用筛选快照"));
            }
            let mut ids = Vec::new();
            for a in v {
                ids.push(
                    a.as_str()
                        .ok_or_else(|| AppError::new("INVALID_SELECTION", "媒体 ID 无效"))?
                        .to_string(),
                );
            }
            ids.sort();
            ids.dedup();
            ids
        }
        "snapshot" => {
            let s = required(t, "snapshotId")?;
            let meta: Option<(i64, i64)> = c
                .query_row(
                    "SELECT initial_count,expires_at FROM selection_snapshots WHERE id=?",
                    [s],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let (count, ttl) = meta
                .ok_or_else(|| AppError::new("SNAPSHOT_EXPIRED", "选择快照不存在，请重新选择"))?;
            let mut st=c.prepare("SELECT asset_id FROM selection_snapshot_items WHERE snapshot_id=? ORDER BY asset_id")?;
            let ids = st
                .query_map([s], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if ttl <= clock() || count != ids.len() as i64 {
                return Err(AppError::new(
                    "SNAPSHOT_EXPIRED",
                    "选择快照已过期或范围发生变化，请重新确认",
                ));
            }
            let excluded: Vec<String> =
                serde_json::from_value(t.get("excludeAssetIds").cloned().unwrap_or(json!([])))?;
            ids.into_iter().filter(|i| !excluded.contains(i)).collect()
        }
        _ => return Err(AppError::new("INVALID_SELECTION", "选区类型无效")),
    };
    for i in &ids {
        if !c.query_row(
            "SELECT EXISTS(SELECT 1 FROM media_assets WHERE id=?)",
            [i],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(AppError::new("INVALID_SELECTION", "选区中的照片已移除"));
        }
    }
    Ok(ids)
}
pub fn validate_snapshot(c: &Connection, id: &str) -> Result<()> {
    validate_snapshot_with_clock(c, id, now)
}
fn validate_snapshot_with_clock(c: &Connection, id: &str, clock: impl FnOnce() -> i64) -> Result<()> {
    let meta:Option<(i64,i64,i64)>=c.query_row("SELECT initial_count,expires_at,(SELECT count(*) FROM selection_snapshot_items WHERE snapshot_id=ss.id) FROM selection_snapshots ss WHERE id=?",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    if !meta.is_some_and(|(initial, ttl, current)| ttl > clock() && initial == current) {
        return Err(AppError::new("SNAPSHOT_EXPIRED", "冻结选区已过期或数量变化，请重新确认")
            .details(json!({"snapshotId": id})));
    }
    Ok(())
}
impl Catalog {
    pub fn neighbors(&self, c: &Connection, r: &Value) -> Result<Value> {
        // Keep the query count, rank and neighbor window on one WAL read
        // snapshot while a background scan or review changes membership.
        let tx = c.unchecked_transaction()?;
        let c = &tx;
        let fp = required(r, "queryFingerprint")?;
        let context: String = c
            .query_row(
                "SELECT request_json FROM query_contexts WHERE fingerprint=? AND expires_at>?",
                params![fp, now()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| AppError::new("INVALID_FILTER", "浏览会话已过期"))?;
        let ctx: Value = serde_json::from_str(&context)?;
        let f: Filter = serde_json::from_value(ctx["filter"].clone())?;
        let (w, p) = scoped_conditions(c, &f)?;
        let s: Sort = serde_json::from_value(ctx["sort"].clone())?;
        let col = sort_column(&s)?;
        let aid = required(r, "assetId")?;
        let raw: Sql = c.query_row(&format!("SELECT {col}{FROM}WHERE a.id=?"), [aid], |r| {
            r.get(0)
        })?;
        let before_op = if s.direction == "asc" { "<" } else { ">" };
        let mut counter_args = vec![Sql::Text(aid.into()), raw.clone(), raw.clone(), Sql::Text(aid.into())];
        counter_args.extend(p.clone());
        let (total, anchor_matches, preceding): (i64, bool, i64) = c.query_row(
            &format!("SELECT count(*),COALESCE(max(a.id=?),0),COALESCE(sum((({col}) {before_op} ?) OR (({col})=? AND a.id {before_op} ?)),0){FROM}WHERE {w}"),
            params_from_iter(counter_args),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut out = json!({"previousId":null,"nextId":null,"anchorMatches":anchor_matches,"total":total,"position":anchor_matches.then_some(preceding + 1)});
        let limit = r["limit"].as_u64().unwrap_or(1).clamp(1, 10);
        let mut before = vec![];
        let mut after = vec![];
        for (key, is_next) in [("previousId", false), ("nextId", true)] {
            let asc = (s.direction == "asc") == is_next;
            let op = if asc { ">" } else { "<" };
            let dir = if asc { "ASC" } else { "DESC" };
            let mut args = p.clone();
            args.extend([raw.clone(), raw.clone(), Sql::Text(aid.into())]);
            let mut st = c.prepare(&format!("SELECT {SELECT}{FROM}WHERE {w} AND (({col}) {op} ? OR (({col})=? AND a.id {op} ?)) ORDER BY {col} {dir},a.id {dir} LIMIT {limit}"))?;
            let rows = st
                .query_map(params_from_iter(args), summary)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            out[key] = rows.first().map(|a| a["id"].clone()).unwrap_or(Value::Null);
            if is_next {
                after = rows;
            } else {
                before = rows;
            }
        }
        before.reverse();
        before.push(c.query_row(
            &format!("SELECT {SELECT}{FROM}WHERE a.id=?"),
            [aid],
            summary,
        )?);
        before.extend(after);
        out["items"] = json!(before);
        tx.commit()?;
        Ok(out)
    }
}
