use crate::{
    db,
    error::{required, AppError, Result},
    now, review,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub fn save(c: &mut Connection, request: &Value) -> Result<Value> {
    let aid = required(request, "assetId")?;
    let key = required(request, "clientMutationId")?;
    let expected = request["expectedRevision"]
        .as_i64()
        .filter(|v| *v >= 0)
        .ok_or_else(|| AppError::new("INVALID_FILTER", "缺少拍摄时间版本"))?;
    let local = match request.get("captureLocal") {
        Some(Value::Null) => None,
        Some(Value::String(value)) => {
            if value.len() != 19
                || chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").is_err()
            {
                return Err(AppError::new(
                    "INVALID_FILTER",
                    "拍摄时间格式须为 YYYY-MM-DD HH:MM:SS",
                ));
            }
            Some(value.as_str())
        }
        _ => return Err(AppError::new("INVALID_FILTER", "拍摄时间无效")),
    };
    let tz = match request.get("captureTzMin") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_i64()
                .filter(|v| (-840..=840).contains(v))
                .ok_or_else(|| AppError::new("INVALID_FILTER", "时区偏移须为 -840 至 840 分钟"))?,
        ),
    };
    if local.is_none() && tz.is_some() {
        return Err(AppError::new("INVALID_FILTER", "恢复原时间时不能另设时区"));
    }
    let utc = local.zip(tz).map(|(local, tz)| {
        chrono::NaiveDateTime::parse_from_str(local, "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .and_utc()
            .timestamp_millis()
            - tz * 60_000
    });
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(receipt) = review::receipt(&tx, key, request)? {
        return Ok(receipt);
    }
    let revision: i64 = tx
        .query_row(
            "SELECT revision FROM capture_time_corrections WHERE asset_id=?",
            [aid],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    if revision != expected {
        return Err(AppError::new(
            "DB_CONFLICT",
            "拍摄时间已被其他操作更新，请重新打开详情",
        ));
    }
    let inserted = tx.execute("INSERT INTO capture_time_corrections(asset_id,original_local,original_utc_ms,original_tz_min,original_source,corrected_local,corrected_utc_ms,corrected_tz_min,is_corrected,revision,updated_at) SELECT id,capture_local,capture_utc_ms,capture_tz_min,capture_time_source,?,?,?, ?,1,? FROM media_assets WHERE id=? ON CONFLICT(asset_id) DO UPDATE SET corrected_local=excluded.corrected_local,corrected_utc_ms=excluded.corrected_utc_ms,corrected_tz_min=excluded.corrected_tz_min,is_corrected=excluded.is_corrected,revision=capture_time_corrections.revision+1,updated_at=excluded.updated_at",params![local,utc,tz,local.is_some(),now(),aid])?;
    if inserted == 0 {
        return Err(AppError::new("INVALID_SELECTION", "素材不存在"));
    }
    apply(&tx, aid)?;
    // Time-window similarity must be recalculated; byte-exact groups stay valid.
    tx.execute("DELETE FROM similar_groups WHERE group_type='similar' AND id IN (SELECT group_id FROM similar_group_items WHERE asset_id=?)",[aid])?;
    db::bump(&tx)?;
    let result = json!({"updated":true,"revision":revision+1});
    review::save_receipt(&tx, key, request, &result)?;
    tx.commit()?;
    Ok(result)
}
fn apply(c: &Connection, aid: &str) -> Result<()> {
    c.execute("UPDATE media_assets SET (capture_local,capture_utc_ms,capture_tz_min,capture_time_source)=(SELECT CASE WHEN is_corrected THEN corrected_local ELSE original_local END,CASE WHEN is_corrected THEN corrected_utc_ms ELSE original_utc_ms END,CASE WHEN is_corrected THEN corrected_tz_min ELSE original_tz_min END,CASE WHEN is_corrected THEN 'manual' ELSE original_source END FROM capture_time_corrections WHERE asset_id=?),updated_at=? WHERE id=?",params![aid,now(),aid])?;
    Ok(())
}
/// A rescan refreshes the source metadata but retains the user's catalog override.
pub fn refresh_original(c: &Connection, aid: &str, metadata: &Value) -> Result<()> {
    let changed = c.execute("UPDATE capture_time_corrections SET original_local=?,original_utc_ms=?,original_tz_min=?,original_source=? WHERE asset_id=?",params![metadata["captureLocal"].as_str(),metadata["captureUtcMs"].as_i64(),metadata["captureTzMin"].as_i64(),metadata["captureTimeSource"].as_str(),aid])?;
    if changed > 0 {
        apply(c, aid)?;
    }
    Ok(())
}
pub fn detail(c: &Connection, aid: &str) -> Result<Value> {
    Ok(c.query_row("SELECT original_local,original_utc_ms,original_tz_min,original_source,is_corrected,revision FROM capture_time_corrections WHERE asset_id=?",[aid],|r|Ok(json!({"original":{"captureLocal":r.get::<_,Option<String>>(0)?,"captureUtcMs":r.get::<_,Option<i64>>(1)?,"captureTzMin":r.get::<_,Option<i64>>(2)?,"captureTimeSource":r.get::<_,Option<String>>(3)?},"isCorrected":r.get::<_,bool>(4)?,"revision":r.get::<_,i64>(5)?}))).optional()?.unwrap_or(json!({"original":null,"isCorrected":false,"revision":0})))
}
