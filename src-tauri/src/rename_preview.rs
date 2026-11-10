//! Read-only source naming analysis. REPAIR jobs provide the existing bounded,
//! cancellable catalog-work scheduler; the UI exposes this operation by name.
use crate::{
    catalog::Catalog,
    error::{required, AppError, Result},
    export_options::NamingRule,
    files, now, query, tasks,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

pub fn start(cat: &Catalog, c: &mut Connection, r: &Value) -> Result<Value> {
    let rule: NamingRule = serde_json::from_value(r["rule"].clone())?;
    rule.validate()?;
    let snapshot = query::snapshot(
        c,
        &json!({"filter":{},"sort":{"field":"filename","direction":"asc"},"target":r["target"]}),
    )?;
    if snapshot["count"] == 0 {
        return Err(AppError::new("INVALID_SELECTION", "请先选择媒体"));
    }
    cat.queue(c,"REPAIR",json!({"operation":"renamePreview","rule":rule,"target":{"kind":"snapshot","snapshotId":snapshot["id"]}}))
}
struct DirectoryNames {
    exact: HashSet<String>,
    folded: HashSet<String>,
}
fn key(value: &str) -> String {
    value.to_lowercase()
}
pub fn run(cat: &Catalog, jid: &str, r: &Value) -> Result<()> {
    let c = cat.connection()?;
    let rule: NamingRule = serde_json::from_value(r["rule"].clone())?;
    rule.validate()?;
    let sid = required(&r["target"], "snapshotId")?;
    query::validate_snapshot(&c, sid)?;
    let mut statement=c.prepare("SELECT a.id,a.source_id,a.relative_path,a.filename,s.display_name,s.absolute_path,CASE WHEN a.capture_time_source='mtime' THEN NULL ELSE a.capture_local END FROM media_assets a JOIN selection_snapshot_items si ON si.asset_id=a.id JOIN source_roots s ON s.id=a.source_id WHERE si.snapshot_id=? ORDER BY a.filename COLLATE NOCASE,a.relative_path,a.source_id,a.id")?;
    let records = statement
        .query_map([sid], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    tasks::progress(&c, jid, 0, Some(records.len()), 0)?;
    let mut directories: HashMap<PathBuf, Option<DirectoryNames>> = HashMap::new();
    let mut planned: HashMap<(String, String), usize> = HashMap::new();
    let mut items = vec![];
    for (index, (aid, source_id, relative, filename, source_name, root, capture)) in
        records.iter().enumerate()
    {
        tasks::safe_point(cat, jid)?;
        files::safe_relative(Path::new(relative))?;
        let parent = Path::new(root).join(Path::new(relative).parent().unwrap_or(Path::new("")));
        if !directories.contains_key(&parent) {
            let inside = Path::new(root)
                .canonicalize()
                .ok()
                .zip(parent.canonicalize().ok())
                .is_some_and(|(root, parent)| parent.starts_with(root));
            let names = if inside {
                std::fs::read_dir(&parent).ok().and_then(|entries| {
                    let mut names = HashSet::new();
                    for entry in entries {
                        if names.len() % 100 == 0 && tasks::safe_point(cat, jid).is_err() {
                            return None;
                        }
                        let entry = entry.ok()?;
                        names.insert(entry.file_name().to_string_lossy().to_string());
                    }
                    Some(DirectoryNames {
                        folded: names.iter().map(|name| key(name)).collect(),
                        exact: names,
                    })
                })
            } else {
                None
            };
            directories.insert(parent.clone(), names);
        }
        let names = directories.get(&parent).and_then(Option::as_ref);
        let available = names.is_some_and(|names| names.exact.contains(filename));
        let rendered = rule.filename(filename, source_name, capture.as_deref(), index);
        let (new_name, mut status, mut reason) = match rendered {
            Ok(name) => (Some(name), "ready", String::new()),
            Err(error) => (None, "invalid", error.message),
        };
        let new_relative = new_name.as_ref().map(|name| {
            Path::new(relative)
                .with_file_name(name)
                .to_string_lossy()
                .into_owned()
        });
        if let Some(name) = &new_name {
            *planned
                .entry((source_id.clone(), key(new_relative.as_deref().unwrap())))
                .or_default() += 1;
            if status == "ready" {
                if name == filename {
                    status = "unchanged";
                } else if key(name) != key(filename)
                    && names.is_some_and(|names| names.folded.contains(&key(name)))
                {
                    status = "conflict";
                    reason = "来源目录已有同名文件或目录".into();
                }
            }
        }
        if !available {
            status = "unavailable";
            reason = "原文件缺失、来源离线或目录无法读取".into();
        }
        items.push(json!({"assetId":aid,"sourceId":source_id,"sourceName":source_name,"sourceRelativePath":relative,"originalFilename":filename,"proposedFilename":new_name,"proposedRelativePath":new_relative,"status":status,"reason":reason,"dateUnknown":capture.is_none()}));
        if index % 100 == 99 {
            tasks::progress(&c, jid, index + 1, Some(records.len()), 0)?;
        }
    }
    tasks::safe_point(cat, jid)?;
    let mut summary = json!({"total":items.len(),"ready":0,"unchanged":0,"conflict":0,"invalid":0,"unavailable":0});
    for item in &mut items {
        if ["ready", "unchanged"].contains(&item["status"].as_str().unwrap_or(""))
            && item["proposedRelativePath"].as_str().is_some_and(|path| {
                planned
                    .get(&(item["sourceId"].as_str().unwrap().to_string(), key(path)))
                    .copied()
                    .unwrap_or(0)
                    > 1
            })
        {
            item["status"] = json!("conflict");
            item["reason"] = json!("本次选区有多个文件生成相同名称");
        }
        let status = item["status"].as_str().unwrap();
        summary[status] = json!(summary[status].as_u64().unwrap() + 1);
    }
    let data = json!({"kind":"renamePreview","summary":summary,"items":items});
    tasks::safe_point(cat, jid)?;
    c.execute(
        "INSERT INTO export_plans VALUES(?,?,?,?,?,?,?,NULL)",
        params![
            jid,
            sid,
            data.to_string(),
            0,
            records.len(),
            now(),
            now() + 900_000
        ],
    )?;
    tasks::progress(&c, jid, records.len(), Some(records.len()), 0)?;
    Ok(())
}
pub fn report(c: &Connection, r: &Value) -> Result<Value> {
    let jid = required(r, "jobId")?;
    let request:Option<(String,String)>=c.query_row("SELECT json_extract(checkpoint_json,'$.rule'),json_extract(checkpoint_json,'$.target') FROM jobs WHERE id=? AND type='REPAIR' AND json_extract(checkpoint_json,'$.operation')='renamePreview'",[jid],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    let (rule, target) =
        request.ok_or_else(|| AppError::new("INVALID_SELECTION", "此任务不是重命名预演"))?;
    let job = tasks::job(c, jid)?;
    let mut result = json!({"job":job,"rule":serde_json::from_str::<Value>(&rule)?,"target":serde_json::from_str::<Value>(&target)?,"summary":null,"items":[],"nextOffset":null,"expiresAt":null});
    if result["job"]["status"] != "completed" {
        return Ok(result);
    }
    let data:Option<(String,i64)>=c.query_row("SELECT json_extract(plan_json,'$.summary'),expires_at FROM export_plans WHERE token=? AND json_extract(plan_json,'$.kind')='renamePreview'",[jid],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    let (summary, expires) =
        data.ok_or_else(|| AppError::new("SNAPSHOT_EXPIRED", "预演结果已清理，请重新预演"))?;
    if expires < now() {
        return Err(AppError::new(
            "SNAPSHOT_EXPIRED",
            "预演结果已过期，请重新预演",
        ));
    }
    let offset = r["offset"].as_u64().unwrap_or(0).min(i64::MAX as u64) as i64;
    let size = r["pageSize"].as_u64().unwrap_or(100).clamp(1, 100) as i64;
    let mut statement=c.prepare("SELECT j.value FROM export_plans p,json_each(p.plan_json,'$.items') j WHERE p.token=? LIMIT ? OFFSET ?")?;
    let values = statement
        .query_map(params![jid, size, offset], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let values = values
        .into_iter()
        .map(|s| serde_json::from_str::<Value>(&s))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let summary: Value = serde_json::from_str(&summary)?;
    result["nextOffset"] = if offset.saturating_add(size) < summary["total"].as_i64().unwrap_or(0) {
        json!(offset + size)
    } else {
        Value::Null
    };
    result["items"] = json!(values);
    result["summary"] = summary;
    result["expiresAt"] = json!(expires);
    Ok(result)
}
