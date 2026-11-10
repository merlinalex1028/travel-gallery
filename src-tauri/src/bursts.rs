//! Time-only shooting sequences. These are never duplicate or quality judgments.
use crate::{
    db,
    error::{AppError, Result},
    id, now, query,
};
use chrono::NaiveDateTime;
use rusqlite::{params, params_from_iter, Connection};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MAX_CANDIDATES: usize = 100_000;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    #[serde(default)]
    filter: query::Filter,
    #[serde(default = "gap_default")]
    gap_seconds: i64,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default = "page_default")]
    page_size: usize,
    #[serde(default)]
    group_id: Option<String>,
}
fn gap_default() -> i64 {
    2
}
fn page_default() -> usize {
    20
}
struct Group {
    key: String,
    source: String,
    folder: String,
    camera: String,
    camera_key: (String, String),
    tz: Option<i64>,
    start: NaiveDateTime,
    end: NaiveDateTime,
    ids: Vec<String>,
}
impl Group {
    fn finish(&mut self) {
        self.key = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&self.ids).unwrap())
        );
    }
}
struct Sequences {
    groups: Vec<Group>,
    total: i64,
    examined: usize,
    skipped: usize,
    truncated: bool,
}
fn collect(c: &Connection, r: &Request) -> Result<Sequences> {
    if !(1..=10).contains(&r.gap_seconds) || !(1..=50).contains(&r.page_size) {
        return Err(AppError::new(
            "INVALID_FILTER",
            "拍摄间隔须为 1–10 秒，每页须为 1–50 组",
        ));
    }
    if let Some(snapshot) = &r.filter.selection_snapshot_id {
        query::validate_snapshot(c, snapshot)?;
    }
    let (condition, args) = query::conditions(&r.filter)?;
    let condition = format!("{condition} AND a.media_kind='image' AND a.file_state<>'missing' AND a.capture_time_source IN ('exif','quicktime','manual') AND a.capture_local IS NOT NULL AND length(trim(COALESCE(a.camera_model,'')))>0");
    let total = c.query_row(
        &format!("SELECT count(*){} WHERE {condition}", query::FROM),
        params_from_iter(args.iter()),
        |row| row.get::<_, i64>(0),
    )?;
    // Partition by source, exact parent, camera make/model, timezone and original day.
    let folder = "substr(a.relative_path,1,length(a.relative_path)-length(a.filename))";
    let mut statement = c.prepare(&format!("SELECT a.id,a.source_id,{folder},COALESCE(a.camera_make,''),a.camera_model,a.capture_tz_min,a.capture_local {} WHERE {condition} ORDER BY a.source_id,{folder},COALESCE(a.camera_make,''),a.camera_model,a.capture_tz_min,a.capture_local,a.id LIMIT {}",query::FROM, MAX_CANDIDATES+1))?;
    let rows = statement.query_map(params_from_iter(args.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    let mut groups = vec![];
    let mut current: Option<Group> = None;
    let mut examined = 0;
    let mut skipped = 0;
    let mut truncated = false;
    for row in rows {
        let (asset, source, folder, make, model, tz, local) = row?;
        let camera = format!("{make} {model}").trim().to_owned();
        let camera_key = (make, model);
        if examined == MAX_CANDIDATES {
            truncated = true;
            break;
        }
        examined += 1;
        let Ok(time) = NaiveDateTime::parse_from_str(&local, "%Y-%m-%d %H:%M:%S") else {
            skipped += 1;
            continue;
        };
        let same = current.as_ref().is_some_and(|g| {
            g.source == source
                && g.folder == folder
                && g.camera_key == camera_key
                && g.tz == tz
                && g.start.date() == time.date()
                && time.signed_duration_since(g.end).num_seconds() <= r.gap_seconds
                && time.signed_duration_since(g.start).num_seconds() <= 30
        });
        if same {
            let g = current.as_mut().unwrap();
            g.end = time;
            g.ids.push(asset);
        } else {
            if let Some(mut g) = current.take().filter(|g| g.ids.len() > 1) {
                g.finish();
                groups.push(g);
            }
            current = Some(Group {
                key: String::new(),
                source,
                folder,
                camera: camera.trim().into(),
                camera_key,
                tz,
                start: time,
                end: time,
                ids: vec![asset],
            });
        }
    }
    // At the candidate cap the final sequence may be incomplete; do not expose it.
    if !truncated {
        if let Some(mut g) = current.filter(|g| g.ids.len() > 1) {
            g.finish();
            groups.push(g);
        }
    }
    groups.sort_by(|a, b| b.start.cmp(&a.start).then(a.key.cmp(&b.key)));
    Ok(Sequences {
        groups,
        total,
        examined,
        skipped,
        truncated,
    })
}
pub fn query(c: &mut Connection, request: &Value) -> Result<Value> {
    let r: Request = serde_json::from_value(request.clone())?;
    let tx = c.transaction()?;
    let result = collect(&tx, &r)?;
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            &r.filter,
            r.gap_seconds,
            db::epoch(&tx)?
        ))?)
    );
    let offset = if let Some(cursor) = r.cursor {
        let cursor: Value = serde_json::from_str(&cursor)
            .map_err(|_| AppError::new("INVALID_FILTER", "拍摄组游标无效"))?;
        if cursor["fingerprint"] != fingerprint {
            return Err(AppError::new(
                "DB_CONFLICT",
                "资料库或分组条件已改变，请刷新拍摄组",
            ));
        }
        cursor["offset"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| AppError::new("INVALID_FILTER", "拍摄组游标无效"))?
    } else {
        0
    };
    if offset > result.groups.len() {
        return Err(AppError::new("INVALID_FILTER", "拍摄组游标越界"));
    }
    let mut items = vec![];
    let mut summaries = tx.prepare(&format!(
        "SELECT {}{}WHERE a.id=?",
        query::SELECT,
        query::FROM
    ))?;
    for g in result.groups.iter().skip(offset).take(r.page_size) {
        let mut members = vec![];
        for asset in g.ids.iter().take(8) {
            members.push(summaries.query_row([asset], query::summary)?);
        }
        items.push(json!({"id":g.key,"sourceId":g.source,"folder":g.folder,"cameraModel":g.camera,"captureTzMin":g.tz,"startLocal":g.start.format("%Y-%m-%d %H:%M:%S").to_string(),"endLocal":g.end.format("%Y-%m-%d %H:%M:%S").to_string(),"memberCount":g.ids.len(),"members":members}));
    }
    let next = offset + items.len();
    Ok(
        json!({"items":items,"total":result.groups.len(),"nextCursor":if next<result.groups.len(){Some(json!({"fingerprint":fingerprint,"offset":next}).to_string())}else{None},"queryFingerprint":fingerprint,"candidateCount":result.total,"examinedCount":result.examined,"skippedInvalidTimes":result.skipped,"truncated":result.truncated}),
    )
}
pub fn snapshot(c: &mut Connection, request: &Value) -> Result<Value> {
    let r: Request = serde_json::from_value(request.clone())?;
    let group_id = r
        .group_id
        .as_deref()
        .ok_or_else(|| AppError::new("INVALID_SELECTION", "请选择拍摄组"))?;
    let tx = c.transaction()?;
    let result = collect(&tx, &r)?;
    let group = result
        .groups
        .iter()
        .find(|g| g.key == group_id)
        .ok_or_else(|| AppError::new("DB_CONFLICT", "拍摄组已变化，请刷新后重新选择"))?;
    let snap = id();
    let at = now();
    let filter = query::Filter::default();
    let sort = query::Sort {
        field: "captureUtcMs".into(),
        direction: "asc".into(),
    };
    let fingerprint = query::fingerprint(&filter, &sort)?;
    tx.execute(
        "INSERT INTO selection_snapshots VALUES(?,?,?,?,?,?,?)",
        params![
            snap,
            serde_json::to_string(&filter)?,
            serde_json::to_string(&sort)?,
            fingerprint,
            group.ids.len(),
            at,
            at + 86_400_000
        ],
    )?;
    {
        let mut insert = tx.prepare("INSERT INTO selection_snapshot_items VALUES(?,?)")?;
        for asset in &group.ids {
            insert.execute(params![snap, asset])?;
        }
    }
    tx.commit()?;
    Ok(
        json!({"id":snap,"count":group.ids.len(),"createdAt":at,"expiresAt":at+86_400_000,"queryFingerprint":fingerprint}),
    )
}
