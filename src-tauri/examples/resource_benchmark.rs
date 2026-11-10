//! Explicit, persistent, source-safe resource benchmark. Not part of the app UI.
//! No network, no deletion, no toy records presented as decoded real media.
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::io::Write;
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};
use travel_gallery::{catalog::Catalog, files};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const INPUTS: [(&str, &str); 4] = [
    (
        "portrait.jpg",
        "a6f11efaa834706db23f275b6115058fa87fc7f14362681e6abe14e82749de3e",
    ),
    (
        "closed-glasses.jpg",
        "c1d8eb51261c5abc75b53c38ae657ae84546cbfb4359554f011fb678093c0292",
    ),
    (
        "partial-face.jpg",
        "cb0b1f0b64ae348790d18db74c953a389f9b0422ebea53dfd12efd1bbfe10d5f",
    ),
    (
        "closed-coffee.jpg",
        "752fa5472a3d27d6c9b59968f49bb9107ca5fea13180f165b686c851059ccb06",
    ),
];
fn stamp(path: &Path) -> Result<(u64, u128)> {
    let m = path.metadata()?;
    Ok((
        m.len(),
        m.modified()?.duration_since(UNIX_EPOCH)?.as_nanos(),
    ))
}
fn write(root: &Path, name: &str, data: &Value) -> Result<()> {
    let staging = root.join(format!(".{name}.part"));
    fs::write(&staging, serde_json::to_vec_pretty(data)?)?;
    fs::rename(staging, root.join(name))?;
    Ok(())
}
fn phase(root: &Path, name: &str, more: Value) -> Result<()> {
    write(
        root,
        "phase.json",
        &json!({"pid":std::process::id(),"at":travel_gallery::now(),"phase":name,"detail":more}),
    )
}
fn stats(root: &Path, cat: &Catalog) -> Result<Value> {
    let mut bytes = 0u64;
    for e in fs::read_dir(root.join("data"))? {
        let e = e?;
        if e.file_type()?.is_file() {
            bytes += e.metadata()?.len();
        }
    }
    let cache = cat.call("cache_stats", json!(null))?;
    Ok(json!({"databaseDirectoryBytes":bytes,"cache":cache}))
}
fn percentile(samples: &[f64]) -> Value {
    let mut values = samples.to_vec();
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return json!({"samples":0});
    }
    let at = |q: f64| {
        values[((values.len() as f64 * q).ceil() as usize)
            .saturating_sub(1)
            .min(values.len() - 1)]
    };
    json!({"samples":values.len(),"p50Ms":at(0.5),"p95Ms":at(0.95),"maxMs":values.last()})
}
fn wait(cat: &Catalog, root: &Path, jid: &str, name: &str, interactive: bool) -> Result<Value> {
    let start = Instant::now();
    let mut browse = vec![];
    let mut preview = vec![];
    let mut preview_cold = vec![];
    let mut preview_cached = vec![];
    let mut last_probe = Instant::now();
    let mut last_print = Instant::now();
    let mut probes = 0;
    let query = json!({"filter":{"combineLinked":false},"sort":{"field":"filename","direction":"asc"},"cursor":null,"pageSize":120});
    loop {
        let row = cat.call("get_job", json!({"jobId":jid}))?;
        phase(
            root,
            name,
            json!({"job":row,"elapsedMs":start.elapsed().as_secs_f64()*1000.,"querySamples":browse.len(),"previewSamples":preview.len()}),
        )?;
        if last_print.elapsed() > Duration::from_secs(5) {
            println!(
                "{name}: {} / {} ({})",
                row["progressDone"], row["progressTotal"], row["status"]
            );
            last_print = Instant::now();
        }
        if ["completed", "failed", "cancelled"].contains(&row["status"].as_str().unwrap_or("")) {
            if row["status"] != "completed" || row["failureCount"] != 0 {
                return Err(format!("{name} did not complete cleanly: {row}").into());
            }
            return Ok(
                json!({"job":row,"elapsedMs":start.elapsed().as_secs_f64()*1000.,"jobDurationWallMs":row["updatedAt"].as_i64().zip(row["createdAt"].as_i64()).map(|(a,b)|a-b),"foregroundQuery":percentile(&browse),"foregroundPreviewResolve":percentile(&preview),"foregroundPreviewColdResolve":percentile(&preview_cold),"foregroundPreviewCachedResolve":percentile(&preview_cached),"storageAfter":stats(root,cat)?}),
            );
        }
        if interactive && last_probe.elapsed() > Duration::from_secs(1) {
            let t = Instant::now();
            let page = cat.call("query_assets", query.clone())?;
            browse.push(t.elapsed().as_secs_f64() * 1000.);
            if let Some(items) = page["items"].as_array().filter(|v| !v.is_empty()) {
                let aid = items[(probes * 79) % items.len()]["id"]
                    .as_str()
                    .ok_or("missing asset id")?;
                let connection = cat.connection()?;
                let entry:Option<String>=connection.query_row("SELECT t.cache_rel_path FROM thumbnail_cache t JOIN media_assets a ON a.id=t.asset_id WHERE t.asset_id=? AND t.variant='medium' AND t.source_byte_size=a.byte_size AND t.source_mtime_ms IS a.mtime_ms AND t.format_version=1",[aid],|r|r.get(0)).optional()?;
                let cached = entry.is_some_and(|p| root.join("cache").join(p).is_file());
                let t = Instant::now();
                let response =
                    cat.call("resolve_preview", json!({"assetId":aid,"variant":"medium"}))?;
                if response["ready"] != true
                    || !response["resourceUrl"]
                        .as_str()
                        .is_some_and(|u| u.starts_with("media://localhost/"))
                {
                    return Err(format!("foreground preview failed: {response}").into());
                }
                let elapsed = t.elapsed().as_secs_f64() * 1000.;
                preview.push(elapsed);
                if cached {
                    preview_cached.push(elapsed);
                } else {
                    preview_cold.push(elapsed);
                }
                let mut log = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(root.join(format!("{name}-foreground.jsonl")))?;
                writeln!(
                    log,
                    "{}",
                    json!({"assetId":aid,"cacheEntryValidBefore":cached,"resolveMs":elapsed,"queryMs":browse.last(),"at":travel_gallery::now()})
                )?;
                probes += 1;
            }
            last_probe = Instant::now();
        }
        thread::sleep(Duration::from_millis(120));
    }
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 6 || args[0] != "--root" || args[2] != "--samples" || args[4] != "--count" {
        return Err("Usage: resource_benchmark --root <NEW absolute directory> --samples <four pinned public JPEGs> --count <1000..100000>".into());
    }
    let root = PathBuf::from(&args[1]);
    let inputs = PathBuf::from(&args[3]).canonicalize()?;
    let count: usize = args[5].parse()?;
    if !root.is_absolute() || root.exists() || !(1000..=100000).contains(&count) {
        return Err("Root must be a NEW absolute directory; count must be 1000..100000. Existing data will not be overwritten.".into());
    }
    let mut expected = vec![];
    for (name, hash) in INPUTS {
        let path = inputs.join(name);
        if files::sha256(&path, None)? != hash {
            return Err(format!("Public input hash mismatch: {name}").into());
        }
        expected.push((path, hash.to_string(), stamp(&inputs.join(name))?));
    }
    let logical: u64 = (0..count).map(|i| expected[i % 4].2 .0).sum();
    if fs2::available_space(root.parent().ok_or("missing parent")?)?
        < logical + 2 * 1024 * 1024 * 1024
    {
        return Err("Insufficient free space for the independent fixture files and cache".into());
    }
    fs::create_dir(&root)?;
    write(
        &root,
        "OWNED_BENCHMARK.json",
        &json!({"formatVersion":1,"count":count,"inputs":inputs,"inputSha256":expected.iter().map(|e|&e.1).collect::<Vec<_>>(),"sourceLogicalBytes":logical,"note":"Isolated generated benchmark catalog, not user's application data"}),
    )?;
    phase(&root, "preparing", json!({"files":count}))?;
    let source = root.join("来源 📷");
    fs::create_dir(&source)?;
    let prepare = Instant::now();
    let mut manifest = Vec::with_capacity(count);
    for n in 0..count {
        let folder = source.join(format!("day-{:02}", n % 40));
        fs::create_dir_all(&folder)?;
        let path = folder.join(format!("旅行-{n:06}.jpg"));
        let input = &expected[n % 4];
        if fs::copy(&input.0, &path)? != input.2 .0 {
            return Err("Unexpected copy length".into());
        }
        let before = stamp(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if path.metadata()?.nlink() != 1 {
                return Err("Benchmark fixtures must be independent files, not hard links".into());
            }
        }
        manifest.push(json!({"relative":path.strip_prefix(&source)?.to_string_lossy(),"sha256":input.1,"bytes":before.0,"mtimeNs":before.1}));
        if n % 100 == 0 {
            phase(&root, "preparing", json!({"done":n,"total":count}))?;
        }
    }
    write(&root, "source-before.json", &json!(manifest))?;
    let prepare_ms = prepare.elapsed().as_secs_f64() * 1000.;
    let cat = Catalog::new(&root.join("data"), &root.join("cache"))?;
    cat.authorize(&source, "source")?;
    phase(&root, "initialScan", json!({"files":count}))?;
    let t = Instant::now();
    let added=cat.call("add_source",json!({"selectedPath":source,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}}))?;
    let mut initial = wait(
        &cat,
        &root,
        added["jobId"].as_str().ok_or("missing job")?,
        "initialScan",
        true,
    )?;
    initial["elapsedMs"] = json!(t.elapsed().as_secs_f64() * 1000.);
    write(&root, "initial-scan.json", &initial)?;
    let thumbnail_id=cat.connection()?.query_row("SELECT id FROM jobs WHERE type='THUMBNAIL' AND json_extract(checkpoint_json,'$.sourceId')=? ORDER BY created_at DESC LIMIT 1",[added["sourceId"].as_str().ok_or("missing source")?],|r|r.get::<_,String>(0))?;
    let thumbs = wait(&cat, &root, &thumbnail_id, "fullThumbnails", true)?;
    write(&root, "full-thumbnails.json", &thumbs)?;
    let c = cat.connection()?;
    let cached: usize = c.query_row(
        "SELECT count(*) FROM thumbnail_cache WHERE variant='small'",
        [],
        |r| r.get(0),
    )?;
    if cached != count {
        return Err(format!("Only {cached} / {count} small thumbnails cached").into());
    }
    let snapshot = cat.call(
        "create_selection_snapshot",
        json!({"filter":{"combineLinked":false},"sort":{"field":"filename","direction":"asc"}}),
    )?;
    if snapshot["count"] != count {
        return Err("Indexed count mismatch".into());
    }
    let job = cat.call(
        "start_duplicate_scan",
        json!({"kind":"exact","scope":{"kind":"snapshot","snapshotId":snapshot["id"]}}),
    )?;
    let hashes = wait(
        &cat,
        &root,
        job["jobId"].as_str().ok_or("missing hash job")?,
        "sha256Candidates",
        true,
    )?;
    write(&root, "sha256.json", &hashes)?;
    let hashed: usize = c.query_row(
        "SELECT count(*) FROM media_assets WHERE sha256 IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    if hashed != count {
        return Err(format!("Only {hashed} / {count} SHA-256 candidates processed").into());
    }
    let (groups,members):(usize,usize)=c.query_row("SELECT (SELECT count(*) FROM similar_groups WHERE group_type='exact'),(SELECT count(*) FROM similar_group_items)",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
    if groups != 4 || members != count {
        return Err("Expected four source-content families in exact groups".into());
    }
    let job = cat.call("rescan_source", json!({"sourceId":added["sourceId"]}))?;
    let incremental = wait(
        &cat,
        &root,
        job["jobId"].as_str().ok_or("missing incremental job")?,
        "incrementalScan",
        true,
    )?;
    write(&root, "incremental-scan.json", &incremental)?;
    // Drain the cache-hit thumbnail pass before taking the final resource totals.
    let jid:String=c.query_row("SELECT id FROM jobs WHERE type='THUMBNAIL' AND json_extract(checkpoint_json,'$.sourceId')=? ORDER BY created_at DESC LIMIT 1",[added["sourceId"].as_str().unwrap()],|r|r.get(0))?;
    let warm_thumbs = wait(&cat, &root, &jid, "cachedThumbnails", true)?;
    phase(&root, "sourceVerification", json!({"files":count}))?;
    for (n, item) in manifest.iter().enumerate() {
        let path = source.join(item["relative"].as_str().ok_or("invalid manifest")?);
        let actual = stamp(&path)?;
        if actual.0 != item["bytes"]
            || json!(actual.1) != item["mtimeNs"]
            || files::sha256(&path, None)? != item["sha256"].as_str().ok_or("invalid hash")?
        {
            return Err(format!("Source invariant changed: {path:?}").into());
        }
        if n % 100 == 0 {
            phase(&root, "sourceVerification", json!({"done":n,"total":count}))?;
        }
    }
    for (path, hash, before) in &expected {
        if files::sha256(path, None)? != *hash || stamp(path)? != *before {
            return Err("Public reference source changed".into());
        }
    }
    let human_changes:usize=c.query_row("SELECT count(*) FROM asset_reviews WHERE revision<>0 OR rating<>0 OR pick_state<>'unreviewed' OR is_favorite<>0 OR note<>''",[],|r|r.get(0))?;
    if human_changes != 0 {
        return Err("Read-only workload changed human reviews".into());
    }
    let report = json!({"dateUtc":chrono::Utc::now().to_rfc3339(),"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"profile":if cfg!(debug_assertions){"debug"}else{"release"},"files":count,"sourceLogicalBytes":logical,"prepareMs":prepare_ms,"dataset":"Four pinned public full-resolution JPEGs, 820x1024 to 5387x3591; independently copied files in 40 Chinese/emoji directories; four exact-content families","cacheConditions":"App preview cache initially absent. OS file cache is uncontrolled; fixture copies may use APFS copy-on-write extents. Results do not describe cold HDD I/O.","initialScan":initial,"fullThumbnails":thumbs,"sha256Candidates":hashes,"incrementalScan":incremental,"cachedThumbnailPass":warm_thumbs,"smallThumbnails":cached,"hashedCandidates":hashed,"exactGroups":groups,"exactMembers":members,"sourceSha256AndSizeAndNanosecondMtimeUnchanged":true,"humanReviewChanges":human_changes,"storage":stats(&root,&cat)?,"limits":["Headless production Catalog code, not native WebKit/compositor; preview timings end at resolve_preview, not visual paint","JPEG-only workload with repeated content; mixed-format device corpus and other machine classes remain separate","This is not the 30k native interactive startup, 50k 60-second grid, or keyboard visual-feedback acceptance"]});
    write(&root, "report.json", &report)?;
    phase(
        &root,
        "completed",
        json!({"report":root.join("report.json")}),
    )?;
    println!("Completed: {}", root.join("report.json").display());
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("RESOURCE_BENCHMARK_FAILED: {e}");
        std::process::exit(1);
    }
}
