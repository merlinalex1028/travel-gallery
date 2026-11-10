use crate::{
    db,
    error::{required, AppError, Result},
    id, now, query,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub fn receipt(c: &Connection, key: &str, r: &Value) -> Result<Option<Value>> {
    let hash = format!("{:x}", Sha256::digest(r.to_string().as_bytes()));
    let old: Option<(String, String)> = c
        .query_row(
            "SELECT payload_sha256,result_json FROM mutation_receipts WHERE client_mutation_id=?",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((h, res)) = old {
        if h != hash {
            return Err(AppError::new("DB_CONFLICT", "操作编号已用于另一项变更"));
        }
        return Ok(Some(serde_json::from_str(&res)?));
    }
    Ok(None)
}
pub fn save_receipt(c: &Connection, key: &str, r: &Value, result: &Value) -> Result<()> {
    c.execute(
        "INSERT INTO mutation_receipts VALUES(?,?,?,?)",
        params![
            key,
            format!("{:x}", Sha256::digest(r.to_string().as_bytes())),
            result.to_string(),
            now()
        ],
    )?;
    Ok(())
}
fn state(c: &Connection, aid: &str) -> Result<Value> {
    let a = query::asset(c, aid)?;
    let mut s = c.prepare("SELECT tag_id FROM asset_tags WHERE asset_id=? ORDER BY tag_id")?;
    let tags = s
        .query_map([aid], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(json!({"review":a["review"],"tagIds":tags}))
}
fn write_state(c: &Connection, aid: &str, v: &Value, revision: i64) -> Result<()> {
    let r = &v["review"];
    c.execute("UPDATE asset_reviews SET pick_state=?,rating=?,is_favorite=?,note=?,revision=?,updated_at=? WHERE asset_id=?",params![r["pickState"].as_str(),r["rating"].as_i64(),r["isFavorite"].as_bool(),r["note"].as_str(),revision,now(),aid])?;
    c.execute("DELETE FROM asset_tags WHERE asset_id=?", [aid])?;
    for tag in v["tagIds"].as_array().unwrap() {
        c.execute(
            "INSERT INTO asset_tags VALUES(?,?)",
            params![aid, tag.as_str()],
        )?;
    }
    Ok(())
}
pub fn apply(c: &mut Connection, r: &Value, is_tags: bool) -> Result<Value> {
    let key = required(r, "clientMutationId")?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(v) = receipt(&tx, key, r)? {
        return Ok(v);
    }
    let ids = query::target_ids(&tx, &r["target"])?;
    let patch = &r["patch"];
    if !is_tags {
        let obj = patch
            .as_object()
            .ok_or_else(|| AppError::new("INVALID_FILTER", "标记修改无效"))?;
        if obj
            .keys()
            .any(|k| !["pickState", "rating", "isFavorite", "note"].contains(&k.as_str()))
        {
            return Err(AppError::new("INVALID_FILTER", "标记字段无效"));
        }
        if let Some(p) = obj.get("pickState") {
            if !p
                .as_str()
                .is_some_and(|p| ["keep", "reject", "maybe", "unreviewed"].contains(&p))
            {
                return Err(AppError::new("INVALID_FILTER", "挑片状态无效"));
            }
        }
        if let Some(v) = obj.get("rating") {
            if !v.as_i64().is_some_and(|n| (0..=5).contains(&n)) {
                return Err(AppError::new("INVALID_FILTER", "评分必须为 0–5"));
            }
        }
        if obj.get("isFavorite").is_some_and(|v| !v.is_boolean()) {
            return Err(AppError::new("INVALID_FILTER", "收藏值无效"));
        }
        if obj
            .get("note")
            .is_some_and(|v| v.as_str().is_none_or(|s| s.len() > 20000))
        {
            return Err(AppError::new("INVALID_FILTER", "备注过长或无效"));
        }
    }
    let add: Vec<String> = if is_tags {
        serde_json::from_value(r["addTagIds"].clone())?
    } else {
        vec![]
    };
    let remove: Vec<String> = if is_tags {
        serde_json::from_value(r["removeTagIds"].clone())?
    } else {
        vec![]
    };
    for t in add.iter().chain(remove.iter()) {
        if !tx.query_row("SELECT EXISTS(SELECT 1 FROM tags WHERE id=?)", [t], |r| {
            r.get::<_, bool>(0)
        })? {
            return Err(AppError::new("INVALID_SELECTION", "标签已不存在"));
        }
    }
    let mut changes = vec![];
    let mut conflicts = vec![];
    for aid in &ids {
        let before = state(&tx, aid)?;
        if let Some(expected) = r["expectedRevisions"].get(aid) {
            if *expected != before["review"]["revision"] {
                conflicts
                    .push(json!({"assetId":aid,"actualRevision":before["review"]["revision"]}));
            }
        }
        let mut after = before.clone();
        if is_tags {
            let mut tags: Vec<String> = serde_json::from_value(before["tagIds"].clone())?;
            tags.retain(|t| !remove.contains(t));
            tags.extend(add.clone());
            tags.sort();
            tags.dedup();
            after["tagIds"] = json!(tags);
        } else {
            for (k, v) in patch.as_object().unwrap() {
                after["review"][k] = v.clone();
            }
        }
        if before != after {
            after["review"]["revision"] = json!(before["review"]["revision"].as_i64().unwrap() + 1);
            after["review"]["updatedAt"] = json!(now());
            changes.push((aid, before, after));
        }
    }
    if !conflicts.is_empty() {
        return Err(
            AppError::new("DB_CONFLICT", "照片已被其他操作更新，本批未写入")
                .details(json!({"conflicts":conflicts})),
        );
    }
    let batch = id();
    tx.execute(
        "INSERT INTO change_batches VALUES(?,?,?,?,NULL)",
        params![
            batch,
            if is_tags { "tags" } else { "review" },
            r.to_string(),
            now()
        ],
    )?;
    for (aid, before, after) in &changes {
        write_state(
            &tx,
            aid,
            after,
            after["review"]["revision"].as_i64().unwrap(),
        )?;
        tx.execute(
            "INSERT INTO change_batch_items VALUES(?,?,?,?)",
            params![batch, aid, before.to_string(), after.to_string()],
        )?;
    }
    let res = json!({"changeBatchId":batch,"affectedCount":changes.len(),"unchangedCount":ids.len()-changes.len(),"conflicts":[]});
    save_receipt(&tx, key, r, &res)?;
    db::bump(&tx)?;
    tx.commit()?;
    Ok(res)
}
pub fn undo(c: &mut Connection, r: &Value) -> Result<Value> {
    let bid = required(r, "changeBatchId")?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let undone: Option<Option<i64>> = tx
        .query_row(
            "SELECT undone_at FROM change_batches WHERE id=?",
            [bid],
            |r| r.get(0),
        )
        .optional()?;
    if undone.is_none() {
        return Err(AppError::new("INVALID_SELECTION", "撤销记录不存在"));
    }
    if undone.flatten().is_some() {
        return Ok(json!({"changeBatchId":bid,"restoredCount":0,"skippedCount":0}));
    }
    let mut s = tx.prepare(
        "SELECT asset_id,before_json,after_json FROM change_batch_items WHERE batch_id=?",
    )?;
    let rows = s
        .query_map([bid], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(s);
    let (mut restored, mut skipped) = (0, 0);
    for (aid, before, after) in rows {
        let before: Value = serde_json::from_str(&before)?;
        let after: Value = serde_json::from_str(&after)?;
        let current = state(&tx, &aid)?;
        let current_revision = current["review"]["revision"].as_i64().unwrap();
        let expected_revision = after["review"]["revision"].as_i64().unwrap();
        // Only an explicit undo lineage may bridge revisions. Equal values after a
        // new edit (ABA), tag maintenance, and unrelated writes remain conflicts.
        let matches = if current_revision == expected_revision {
            true
        } else {
            tx.query_row("WITH RECURSIVE lineage(revision) AS (SELECT ? UNION ALL SELECT u.restored_revision FROM review_undo_lineage u JOIN lineage l ON u.undo_revision=l.revision WHERE u.asset_id=? AND l.revision>?) SELECT EXISTS(SELECT 1 FROM lineage WHERE revision=?)",params![current_revision,aid,expected_revision,expected_revision],|r|r.get::<_,bool>(0))?
        };
        if !matches {
            skipped += 1;
            continue;
        }
        let mut missing_tag = false;
        for tag in before["tagIds"].as_array().unwrap() {
            if !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM tags WHERE id=?)",
                [tag.as_str()],
                |r| r.get::<_, bool>(0),
            )? {
                missing_tag = true;
                break;
            }
        }
        if missing_tag {
            skipped += 1;
            continue;
        }
        let revision = current_revision + 1;
        write_state(&tx, &aid, &before, revision)?;
        tx.execute(
            "INSERT INTO review_undo_lineage VALUES(?,?,?,?)",
            params![
                aid,
                revision,
                before["review"]["revision"].as_i64().unwrap(),
                bid
            ],
        )?;
        restored += 1;
    }
    tx.execute(
        "UPDATE change_batches SET undone_at=? WHERE id=?",
        params![now(), bid],
    )?;
    db::bump(&tx)?;
    tx.commit()?;
    Ok(json!({"changeBatchId":bid,"restoredCount":restored,"skippedCount":skipped}))
}
