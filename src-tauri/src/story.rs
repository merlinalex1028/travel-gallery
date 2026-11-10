//! Offline travel stories. Plans freeze membership and captions; workers only read
//! source media and publish re-encoded JPEGs plus local HTML into a fresh folder.
use crate::{
    catalog::Catalog,
    db,
    error::{required, AppError, Result},
    files, id, media, now, query, review, tasks,
};
use cap_std::{ambient_authority, fs::Dir};
use rusqlite::{params, params_from_iter, types::Value as Sql, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const MAX_EXPORT_ITEMS: usize = 5_000;
#[derive(Clone, Copy, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Range {
    #[default]
    All,
    Keep,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PageRequest {
    trip_id: String,
    range: Range,
    offset: usize,
    page_size: usize,
    epoch: Option<i64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreviewRequest {
    trip_id: String,
    range: Range,
    title: String,
    description: String,
    include_notes: bool,
    destination_path: PathBuf,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    asset_id: String,
    filename: String,
    media_kind: String,
    file_state: String,
    date: Option<String>,
    unknown_timezone: bool,
    note: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Frozen {
    #[serde(flatten)]
    item: Item,
    source_path: Option<PathBuf>,
    size: u64,
    mtime: Option<i64>,
    orientation: i64,
    relative: String,
}
fn trip(c: &Connection, tid: &str) -> Result<Value> {
    c.query_row("SELECT id,name,description,start_at,end_at,cover_asset_id,(SELECT count(*) FROM collection_assets WHERE collection_id=t.id) FROM collections t WHERE id=? AND kind='trip'", [tid], |r| Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"description":r.get::<_,Option<String>>(2)?,"startAt":r.get::<_,Option<i64>>(3)?,"endAt":r.get::<_,Option<i64>>(4)?,"coverAssetId":r.get::<_,Option<String>>(5)?,"count":r.get::<_,i64>(6)?}))).optional()?.ok_or_else(||AppError::new("INVALID_SELECTION","旅行相册不存在"))
}
fn scope(tid: &str, range: Range) -> Result<(String, Vec<Sql>)> {
    query::conditions(&query::Filter {
        collection_id: Some(tid.into()),
        combine_linked: Some(false),
        pick_states: matches!(range, Range::Keep).then(|| vec!["keep".into()]),
        ..Default::default()
    })
}
fn count(c: &Connection, condition: &str, args: &[Sql]) -> Result<usize> {
    Ok(c.query_row(
        &format!("SELECT count(*) {} WHERE {condition}", query::FROM),
        params_from_iter(args),
        |r| r.get(0),
    )?)
}
fn rows(
    c: &Connection,
    condition: &str,
    args: &[Sql],
    offset: usize,
    limit: usize,
    include_notes: bool,
) -> Result<Vec<Frozen>> {
    let date = query::CAPTURE_DATE;
    let note = if include_notes { "r.note" } else { "''" };
    let sql=format!("SELECT a.id,a.filename,a.media_kind,a.file_state,({date}),a.capture_tz_min,{note},a.byte_size,a.mtime_ms,COALESCE(a.orientation,1) {} WHERE {condition} ORDER BY ({date}) IS NULL,({date}),CASE WHEN ({date}) IS NOT NULL THEN a.capture_local END,a.id LIMIT ? OFFSET ?",query::FROM);
    let mut args = args.to_vec();
    args.push(Sql::Integer(limit as i64));
    args.push(Sql::Integer(offset as i64));
    let mut st = c.prepare(&sql)?;
    let result = st
        .query_map(params_from_iter(args), |r| {
            let date: Option<String> = r.get(4)?;
            Ok(Frozen {
                item: Item {
                    asset_id: r.get(0)?,
                    filename: r.get(1)?,
                    media_kind: r.get(2)?,
                    file_state: r.get(3)?,
                    unknown_timezone: date.is_some() && r.get::<_, Option<i64>>(5)?.is_none(),
                    date,
                    note: r.get(6)?,
                },
                source_path: None,
                size: r.get(7)?,
                mtime: r.get(8)?,
                orientation: r.get(9)?,
                relative: String::new(),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(result)
}
pub fn page(c: &mut Connection, r: &Value) -> Result<Value> {
    let r: PageRequest = serde_json::from_value(r.clone())?;
    if !(1..=100).contains(&r.page_size) || r.offset > 1_000_000 {
        return Err(AppError::new("INVALID_FILTER", "故事分页范围无效"));
    }
    let tx = c.transaction()?;
    let trip = trip(&tx, &r.trip_id)?;
    let epoch = db::epoch(&tx)?;
    if r.epoch.is_some_and(|v| v != epoch) {
        return Err(AppError::new("DB_CONFLICT", "旅行内容已变化，请刷新故事"));
    }
    let (condition, args) = scope(&r.trip_id, r.range)?;
    let total = count(&tx, &condition, &args)?;
    let items = rows(&tx, &condition, &args, r.offset, r.page_size, true)?
        .into_iter()
        .map(|r| r.item)
        .collect::<Vec<_>>();
    let next = r.offset + items.len();
    Ok(
        json!({"trip":trip,"items":items,"total":total,"epoch":epoch,"nextOffset":(next<total).then_some(next)}),
    )
}
pub fn preview(cat: &Catalog, c: &mut Connection, r: &Value) -> Result<Value> {
    let r: PreviewRequest = serde_json::from_value(r.clone())?;
    let title = r.title.trim();
    if title.is_empty() || title.chars().count() > 100 || r.description.chars().count() > 4_000 {
        return Err(AppError::new(
            "INVALID_FILTER",
            "故事标题须为 1–100 字，简介最多 4000 字",
        ));
    }
    let tx = c.transaction()?;
    let trip = trip(&tx, &r.trip_id)?;
    let destination = files::safe_destination(cat, &tx, &r.destination_path, "export")?;
    let (condition, args) = scope(&r.trip_id, r.range)?;
    let total = count(&tx, &condition, &args)?;
    if total == 0 || total > MAX_EXPORT_ITEMS {
        return Err(AppError::new(
            "INVALID_SELECTION",
            "故事导出须包含 1–5000 项，请调整旅行内容或选择仅保留项",
        ));
    }
    // Check the aggregate text budget before materializing notes. Opted-out
    // notes are never loaded into an export plan, even from imported catalogs.
    let note_bytes = if r.include_notes {
        "length(CAST(r.note AS BLOB))"
    } else {
        "0"
    };
    let text_bytes: i64 = tx.query_row(&format!("SELECT COALESCE(sum(length(CAST(a.filename AS BLOB))+{note_bytes}),0) {} WHERE {condition}",query::FROM),params_from_iter(&args),|row|row.get(0))?;
    if text_bytes > 8 * 1024 * 1024 {
        return Err(AppError::new(
            "INVALID_SELECTION",
            "故事文字超过 8 MiB，请关闭整理备注或减少内容",
        ));
    }
    let mut items = rows(&tx, &condition, &args, 0, MAX_EXPORT_ITEMS, r.include_notes)?;
    let mut offline = 0;
    for (index, item) in items.iter_mut().enumerate() {
        item.source_path = files::source_path(&tx, &item.item.asset_id).ok();
        offline += usize::from(item.source_path.is_none());
        item.relative = format!("{:05}.jpg", index + 1);
    }
    let token = id();
    let folder = format!("travel-story-{token}");
    let ttl = now() + 900_000;
    let plan = json!({"kind":"storyExport","operation":"storyExport","destination":destination,"folder":folder,"title":title,"description":r.description,"coverAssetId":trip["coverAssetId"],"includeNotes":r.include_notes,"items":items});
    tx.execute(
        "INSERT INTO export_plans VALUES(?,NULL,?,0,?,?,?,NULL)",
        params![token, plan.to_string(), total, now(), ttl],
    )?;
    tx.commit()?;
    Ok(
        json!({"planToken":token,"expiresAt":ttl,"itemCount":total,"offlineCount":offline,"folder":folder,"includeNotes":r.include_notes}),
    )
}
pub fn start(cat: &Catalog, c: &mut Connection, r: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Request {
        plan_token: String,
        client_mutation_id: String,
    }
    let request: Request = serde_json::from_value(r.clone())?;
    // Include the operation in the receipt so a token cannot be replayed through another command.
    let receipt = json!({"operation":"storyExport","request":r});
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(result) = review::receipt(&tx, &request.client_mutation_id, &receipt)? {
        return Ok(result);
    }
    let (plan, ttl, used): (String, i64, Option<String>) = tx
        .query_row(
            "SELECT plan_json,expires_at,consumed_by_job_id FROM export_plans WHERE token=?",
            [&request.plan_token],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::new("SNAPSHOT_EXPIRED", "故事预演不存在"))?;
    if ttl < now() || used.is_some() {
        return Err(AppError::new(
            "SNAPSHOT_EXPIRED",
            "故事预演已过期或已执行，请重新预演",
        ));
    }
    let plan: Value = serde_json::from_str(&plan)?;
    if plan["kind"] != "storyExport" {
        return Err(AppError::new("INVALID_FILTER", "此计划不是故事导出"));
    }
    files::safe_destination(
        cat,
        &tx,
        Path::new(required(&plan, "destination")?),
        "export",
    )?;
    let jid = id();
    tasks::insert(&tx, &jid, "EXPORT", &plan)?;
    for item in plan["items"]
        .as_array()
        .ok_or_else(|| AppError::new("INTERNAL", "故事计划损坏"))?
    {
        tx.execute(
            "INSERT INTO export_run_items VALUES(?,?,?,'queued',0,NULL,NULL,?)",
            params![
                jid,
                required(item, "assetId")?,
                format!(
                    "{}/{}",
                    required(&plan, "folder")?,
                    required(item, "relative")?
                ),
                now()
            ],
        )?;
    }
    tx.execute(
        "UPDATE export_plans SET consumed_by_job_id=? WHERE token=?",
        params![jid, request.plan_token],
    )?;
    let result = json!({"jobId":jid});
    review::save_receipt(&tx, &request.client_mutation_id, &receipt, &result)?;
    tx.commit()?;
    cat.spawn_job(jid, "EXPORT".into(), plan);
    Ok(result)
}
fn text(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}
fn render(plan: &Value, items: &[Frozen], ready: &[bool]) -> String {
    let title = text(plan["title"].as_str().unwrap_or("旅行故事"));
    let description = text(plan["description"].as_str().unwrap_or(""));
    let mut html=format!("<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src 'self' file:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><title>{title}</title><style>body{{margin:0;background:#f8f6f0;color:#28352e;font:17px/1.7 system-ui,sans-serif}}main{{max-width:960px;margin:auto;padding:8vw 24px}}header{{padding:24px 0 64px}}h1{{font-size:clamp(36px,7vw,72px);line-height:1.15}}h2{{margin-top:64px;border-top:1px solid #ccd3c7;padding-top:24px}}p{{white-space:pre-wrap}}figure{{margin:24px 0 48px}}img{{display:block;width:100%;max-height:85vh;object-fit:contain;background:#e7e9df;border-radius:12px}}figcaption{{padding:12px 0}}small,footer{{color:#5c685f}}.missing{{padding:64px 24px;background:#e7e9df;text-align:center}}.cover{{margin-bottom:64px}}@media print{{figure{{break-inside:avoid}}}}</style></head><body><main><header><small>旅图 · 离线旅行故事</small><h1>{title}</h1><p>{description}</p><small>共 {} 项 · 视频以静帧呈现 · JPEG 预览图，原文件未包含</small></header>",items.len());
    if let Some(index) = items.iter().enumerate().position(|(i, item)| {
        ready[i] && Some(item.item.asset_id.as_str()) == plan["coverAssetId"].as_str()
    }) {
        html.push_str(&format!(
            "<img class=\"cover\" src=\"{}\" alt=\"故事封面\">",
            items[index].relative
        ));
    }
    let mut previous: Option<&str> = None;
    for (index, item) in items.iter().enumerate() {
        let date = item.item.date.as_deref().unwrap_or("未知拍摄日期");
        if previous != Some(date) {
            html.push_str(&format!("<h2>{}</h2>", text(date)));
            previous = Some(date);
        }
        html.push_str("<figure>");
        if ready[index] {
            html.push_str(&format!(
                "<img loading=\"lazy\" src=\"{}\" alt=\"{}\">",
                item.relative,
                text(&item.item.filename)
            ));
        } else {
            html.push_str("<div class=\"missing\">此项预览未导出，请查看旅图任务明细</div>");
        }
        html.push_str(&format!(
            "<figcaption>{}{}{}{}</figcaption></figure>",
            text(&item.item.filename),
            if item.item.media_kind == "video" {
                " · 视频静帧"
            } else {
                ""
            },
            if item.item.unknown_timezone {
                " · 时区未知，按原始日期分章"
            } else {
                ""
            },
            if item.item.note.is_empty() {
                String::new()
            } else {
                format!("<p>{}</p>", text(&item.item.note))
            }
        ));
    }
    html.push_str("<footer>由旅图在本机生成。此故事不含 GPS 字段或源目录路径；整理备注仅在导出时主动选择后包含。预览图已重新编码，可将整个文件夹离线分享。</footer></main></body></html>");
    html
}
fn save_result(c: &Connection, jid: &str, result: &Value) -> Result<()> {
    c.execute("UPDATE jobs SET checkpoint_json=json_set(checkpoint_json,'$.storyResult',json(?)),updated_at=? WHERE id=?",params![result.to_string(),now(),jid])?;
    Ok(())
}
pub fn run(cat: &Catalog, jid: &str, plan: &Value) -> Result<()> {
    tasks::safe_point(cat, jid)?;
    let mut c = cat.connection()?;
    let destination =
        files::safe_destination(cat, &c, Path::new(required(plan, "destination")?), "export")?;
    let root = Dir::open_ambient_dir(&destination, ambient_authority())?;
    let folder = Path::new(required(plan, "folder")?);
    files::safe_relative(folder)?;
    root.create_dir(folder).map_err(|e| {
        AppError::new(
            "EXPORT_CONFLICT",
            "无法创建新的故事文件夹，请检查空间、权限或同名目录",
        )
        .details(json!({"reason":e.to_string()}))
    })?;
    let dir = root.open_dir(folder)?;
    let items: Vec<Frozen> = serde_json::from_value(plan["items"].clone())?;
    let total = items.len() + 2;
    let mut failures = 0;
    let mut ready = vec![false; items.len()];
    let mut exported = vec![];
    tasks::progress(&c, jid, 0, Some(total), 0)?;
    save_result(
        &c,
        jid,
        &json!({"folder":folder,"html":null,"manifest":null}),
    )?;
    for (index, item) in items.iter().enumerate() {
        tasks::safe_point(cat, jid)?;
        let result = (|| -> Result<(u64, String)> {
            let path = files::source_path(&c, &item.item.asset_id)?;
            if item.source_path.as_ref() != Some(&path)
                || files::stamp(&path)? != (item.size, item.mtime.unwrap_or(-1))
            {
                return Err(AppError::new(
                    "DB_CONFLICT",
                    "素材自预演后已变化，请重新预演",
                ));
            }
            let current: (u64, Option<i64>, i64) = c.query_row(
                "SELECT byte_size,mtime_ms,COALESCE(orientation,1) FROM media_assets WHERE id=?",
                [&item.item.asset_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            if current != (item.size, item.mtime, item.orientation) {
                return Err(AppError::new("DB_CONFLICT", "媒体元数据自预演后已变化"));
            }
            let cache = media::thumbnail(cat, &mut c, &item.item.asset_id, "screen")?;
            tasks::safe_point(cat, jid)?;
            // Re-encode pixels rather than copying cache metadata (EXIF/GPS/XMP).
            let img = image::open(&cache)
                .map_err(|_| AppError::new("ASSET_CORRUPT", "故事预览无法解码"))?
                .to_rgb8();
            let mut bytes = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 86)
                .encode_image(&img)
                .map_err(|_| AppError::new("ASSET_CORRUPT", "故事预览无法生成"))?;
            if files::stamp(&path)? != (item.size, item.mtime.unwrap_or(-1)) {
                return Err(AppError::new("DB_CONFLICT", "素材在生成故事预览期间已变化"));
            }
            files::write_generated(&dir, Path::new(&item.relative), &bytes, || {
                tasks::safe_point(cat, jid)
            })
        })();
        match result {
            Ok((bytes, hash)) => {
                ready[index] = true;
                exported.push(json!({"relative":item.relative,"bytes":bytes,"sha256":hash}));
                c.execute("UPDATE export_run_items SET state='copied',copied_bytes=?,verified_sha256=?,updated_at=? WHERE job_id=? AND asset_id=?",params![bytes,hash,now(),jid,item.item.asset_id])?;
            }
            Err(error) => {
                c.execute("UPDATE export_run_items SET state='failed',error_code=?,updated_at=? WHERE job_id=? AND asset_id=?",params![error.code,now(),jid,item.item.asset_id])?;
                if error.code == "CANCELLED" {
                    return Err(error);
                }
                failures += 1;
                tasks::record_error(&c, jid, Some(&item.item.asset_id), &error.message)?;
            }
        }
        tasks::progress(&c, jid, index + 1, Some(total), failures)?;
    }
    let html = render(plan, &items, &ready);
    let (bytes, hash) =
        files::write_generated(&dir, Path::new("index.html"), html.as_bytes(), || {
            tasks::safe_point(cat, jid)
        })?;
    let mut result = json!({"folder":folder,"html":{"relative":"index.html","bytes":bytes,"sha256":hash},"manifest":null});
    save_result(&c, jid, &result)?;
    tasks::progress(&c, jid, items.len() + 1, Some(total), failures)?;
    exported.push(json!({"relative":"index.html","bytes":bytes,"sha256":hash}));
    let manifest = json!({"formatVersion":1,"kind":"travelStory","title":plan["title"],"itemCount":items.len(),"previewCount":ready.iter().filter(|v|**v).count(),"failedCount":failures,"includesNotes":plan["includeNotes"],"originalsIncluded":false,"videos":"postersOnly","files":exported});
    let (bytes, hash) = files::write_generated(
        &dir,
        Path::new("manifest.json"),
        serde_json::to_string_pretty(&manifest)?.as_bytes(),
        || tasks::safe_point(cat, jid),
    )?;
    result["manifest"] = json!({"relative":"manifest.json","bytes":bytes,"sha256":hash});
    save_result(&c, jid, &result)?;
    tasks::progress(&c, jid, total, Some(total), failures)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_escapes_metadata_and_does_not_embed_private_plan_fields() {
        let item = Frozen {
            item: Item {
                asset_id: "a".into(),
                filename: "\"<img src=x onerror=alert(1)> &.jpg".into(),
                media_kind: "video".into(),
                file_state: "available".into(),
                date: None,
                unknown_timezone: false,
                note: "</p><script>oops()</script>".into(),
            },
            source_path: Some(PathBuf::from("/private/secret.jpg")),
            size: 10,
            mtime: Some(1),
            orientation: 1,
            relative: "00001.jpg".into(),
        };
        let html = render(
            &json!({"title":"<script>title</script>","description":"A & B","destination":"/private/target","coverAssetId":"a"}),
            &[item],
            &[true],
        );
        assert!(!html.contains("<script>"));
        assert!(!html.contains("/private/"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("视频静帧"));
        assert!(html.contains("未知拍摄日期"));
        assert!(!html.contains("https://"));
        assert!(html.contains("src=\"00001.jpg\""));
    }
}
