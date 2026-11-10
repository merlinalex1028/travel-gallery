//! Explicit process-fault acceptance worker. Never included in the desktop app.
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
use travel_gallery::catalog::Catalog;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn save(root: &Path, name: &str, value: Value) -> Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(name))?;
    file.write_all(&serde_json::to_vec_pretty(&value)?)?;
    file.sync_all()?;
    Ok(())
}
fn wait(cat: &Catalog, jid: &str) -> Result<Value> {
    let started = Instant::now();
    loop {
        let job = cat.call("get_job", json!({"jobId":jid}))?;
        if ["completed", "failed", "cancelled"].contains(&job["status"].as_str().unwrap_or("")) {
            return Ok(job);
        }
        if started.elapsed() > Duration::from_secs(180) {
            return Err("Scan timed out".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}
fn stop_background(cat: &Catalog) -> Result<()> {
    let c = cat.connection()?;
    let ids = c
        .prepare("SELECT id FROM jobs WHERE status IN ('queued','running','paused')")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in ids {
        cat.call("cancel_job", json!({"jobId":id}))?;
        wait(cat, &id)?;
    }
    Ok(())
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err(
            "Usage: scan_fault_worker <owned root> prepare|cancel|crash|retry-cancel|retry-crash"
                .into(),
        );
    }
    let root = Path::new(&args[0]).canonicalize()?;
    let marker: Value = serde_json::from_slice(&fs::read(root.join("OWNED_SCAN_FAULTS.json"))?)?;
    if marker["identifier"] != "com.travelgallery.scanfault.qa"
        || marker["count"] != 10000
        || marker["root"].as_str() != root.to_str()
        || root.join("data").is_symlink()
        || root.join("source").is_symlink()
        || root.join("cache").is_symlink()
    {
        return Err("Owned scan-fault fixture marker/path mismatch".into());
    }
    let mode = args[1].as_str();
    if !["prepare", "cancel", "crash", "retry-cancel", "retry-crash"].contains(&mode) {
        return Err("Unknown acceptance action".into());
    }
    let cat = Catalog::new(&root.join("data"), &root.join("cache"))?;
    cat.call(
        "save_settings",
        json!({"scanConcurrency":1,"pauseOnBattery":false}),
    )?;
    let c = cat.connection()?;
    if mode == "prepare" {
        cat.authorize(&root.join("source"), "source")?;
        let start=cat.call("add_source",json!({"selectedPath":root.join("source"),"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}}))?;
        let job = wait(&cat, start["jobId"].as_str().ok_or("Missing job ID")?)?;
        if job["status"] != "completed" || job["failureCount"] != 0 {
            return Err(format!("Initial scan failed: {job}").into());
        }
        stop_background(&cat)?;
        let ids = c
            .prepare("SELECT id FROM media_assets ORDER BY filename,id LIMIT 3")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        cat.call("apply_review_batch",json!({"target":{"kind":"explicit","assetIds":ids},"patch":{"pickState":"keep","rating":4,"isFavorite":true,"note":"保留中断前人工整理"},"clientMutationId":"scan-fault-baseline"}))?;
        let tag = cat.call(
            "upsert_tag",
            json!({"name":"中断恢复验收","color":"#315d49"}),
        )?;
        cat.call("apply_tags_batch",json!({"target":{"kind":"explicit","assetIds":ids},"addTagIds":[tag["tagId"]],"removeTagIds":[],"clientMutationId":"scan-fault-tags"}))?;
        save(
            &root,
            "prepare-result.json",
            json!({"pid":std::process::id(),"job":job,"humanAssetIds":ids}),
        )?;
        return Ok(());
    }
    if let Some(previous) = mode.strip_prefix("retry-") {
        let old: Value =
            serde_json::from_slice(&fs::read(root.join(format!("{previous}-start.json")))?)?;
        let old_job = cat.call("get_job", json!({"jobId":old["jobId"]}))?;
        let start = cat.call("retry_job", json!({"jobId":old["jobId"]}))?;
        let job = wait(&cat, start["newJobId"].as_str().ok_or("Missing retry ID")?)?;
        stop_background(&cat)?;
        save(
            &root,
            &format!("{mode}-result.json"),
            json!({"pid":std::process::id(),"oldJobOnOpen":old_job,"newJob":job}),
        )?;
        if job["status"] != "completed" || job["failureCount"] != 0 {
            return Err("Retry failed".into());
        }
        return Ok(());
    }
    let (source, old_generation): (String, i64) =
        c.query_row("SELECT id,scan_generation FROM source_roots", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    let start = cat.call("rescan_source", json!({"sourceId":source}))?;
    let jid = start["jobId"].as_str().ok_or("Missing job ID")?;
    save(
        &root,
        &format!("{mode}-start.json"),
        json!({"pid":std::process::id(),"jobId":jid,"sourceId":source,"oldGeneration":old_generation}),
    )?;
    let started = Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(120) {
            return Err("Fault controller timed out".into());
        }
        let seen: i64 = c.query_row(
            "SELECT count(*) FROM media_assets WHERE source_id=? AND last_seen_scan_generation>?",
            rusqlite::params![source, old_generation],
            |r| r.get(0),
        )?;
        if mode == "cancel" && seen >= 3000 {
            cat.call("cancel_job", json!({"jobId":jid}))?;
            let job = wait(&cat, jid)?;
            save(
                &root,
                "cancel-result.json",
                json!({"pid":std::process::id(),"seenAtRequest":seen,"job":job}),
            )?;
            if job["status"] != "cancelled" {
                return Err("Cancellation did not reach its terminal state".into());
            }
            return Ok(());
        }
        let job = cat.call("get_job", json!({"jobId":jid}))?;
        if job["status"] == "completed" || job["status"] == "failed" {
            return Err("Scan ended before the fault controller acted".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Scan-fault worker failed: {error}");
        std::process::exit(1);
    }
}
