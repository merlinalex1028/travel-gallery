//! Real ENOSPC acceptance on an explicitly owned, mounted macOS disk image.
#![cfg(target_os = "macos")]
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    os::unix::fs::MetadataExt,
    path::Path,
    thread,
    time::{Duration, Instant},
};
use travel_gallery::{catalog::Catalog, files};

const MIB: u64 = 1024 * 1024;

fn save(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn wait(cat: &Catalog, jid: &str, destination: &Path) -> (Value, u64) {
    let started = Instant::now();
    let mut part_peak = 0;
    loop {
        for entry in fs::read_dir(destination).unwrap().flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .contains("travelgallery-part")
            {
                part_peak = part_peak.max(entry.metadata().map(|m| m.len()).unwrap_or(0));
            }
        }
        let job = cat.call("get_job", json!({"jobId":jid})).unwrap();
        if ["completed", "failed", "cancelled"].contains(&job["status"].as_str().unwrap_or("")) {
            return (job, part_peak);
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "job timed out: {job}"
        );
        thread::sleep(Duration::from_millis(1));
    }
}
fn source_state(source: &Path) -> Value {
    let mut entries: Vec<_> = fs::read_dir(source)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    json!(entries
        .iter()
        .map(|p| {
            let stat = fs::metadata(p).unwrap();
            json!({"filename":p.file_name().unwrap().to_str().unwrap(),"bytes":stat.len(),
               "mtimeSeconds":stat.mtime(),"mtimeNanos":stat.mtime_nsec(),
               "sha256":files::sha256(p,None).unwrap()})
        })
        .collect::<Vec<_>>())
}
fn reviews(cat: &Catalog) -> Value {
    let c = cat.connection().unwrap();
    let mut statement = c.prepare("SELECT asset_id,pick_state,rating,is_favorite,note,updated_at,revision FROM asset_reviews ORDER BY asset_id").unwrap();
    let rows = statement
        .query_map([], |r| {
            Ok(json!([
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?
            ]))
        })
        .unwrap();
    json!(rows.collect::<Result<Vec<_>, _>>().unwrap())
}

#[test]
#[ignore = "requires TRAVEL_GALLERY_DISK_FULL_FIXTURE with an owned 256MB APFS mount"]
fn actual_disk_full_preserves_successes_cleans_parts_and_recovers_with_new_preflight() {
    let root = fs::canonicalize(
        std::env::var("TRAVEL_GALLERY_DISK_FULL_FIXTURE").expect("explicit QA fixture required"),
    )
    .unwrap();
    let marker: Value =
        serde_json::from_slice(&fs::read(root.join("OWNED_DISK_FULL_QA.json")).unwrap()).unwrap();
    assert_eq!(marker["kind"], "travel-gallery-owned-disk-full-qa");
    assert_eq!(marker["hostRoot"], root.to_str().unwrap());
    let mount = fs::canonicalize(root.join("volume")).unwrap();
    assert_eq!(marker["mount"], mount.to_str().unwrap());
    assert_eq!(
        marker,
        serde_json::from_slice::<Value>(&fs::read(mount.join("OWNED_DISK_FULL_QA.json")).unwrap())
            .unwrap()
    );
    assert_ne!(
        fs::metadata(&mount).unwrap().dev(),
        fs::metadata(&root).unwrap().dev()
    );
    let capacity = fs2::total_space(&mount).unwrap();
    assert!(
        (64 * MIB..=512 * MIB).contains(&capacity),
        "Only a small owned test volume is allowed"
    );
    let run_name =
        std::env::var("TRAVEL_GALLERY_DISK_FULL_RUN_NAME").unwrap_or_else(|_| "run-1".into());
    assert!(
        run_name.starts_with("run-")
            && run_name.len() < 32
            && run_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
    );
    let mode = std::env::var("TRAVEL_GALLERY_DISK_FULL_MODE").unwrap_or_else(|_| "partial".into());
    assert!(["partial", "zero"].contains(&mode.as_str()));
    let zero_space = mode == "zero";
    let output = root.join(&run_name);
    assert!(!output.exists());
    fs::create_dir(&output).unwrap();
    let source = output.join("来源 📷");
    let destination = mount.join(format!("精选-{run_name}"));
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    for (name, size) in [
        ("a-small.jpg", 256 * 1024),
        ("b-large.jpg", 64 * MIB),
        ("c-small.jpg", 256 * 1024),
    ] {
        image::RgbImage::from_fn(96, 64, |x, y| {
            image::Rgb([(x * 2) as u8, (y * 3) as u8, (x + y) as u8])
        })
        .save(source.join(name))
        .unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(source.join(name))
            .unwrap()
            .set_len(size)
            .unwrap();
    }
    let cat = Catalog::new(&output.join("app-data"), &output.join("cache")).unwrap();
    cat.authorize(&source, "source").unwrap();
    let scan = cat.call("add_source",json!({"selectedPath":source,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}})).unwrap();
    assert_eq!(
        wait(&cat, scan["jobId"].as_str().unwrap(), &destination).0["status"],
        "completed"
    );
    let started = Instant::now();
    while cat.call("list_jobs", json!({})).unwrap()["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|j| ["queued", "running", "paused"].contains(&j["status"].as_str().unwrap_or("")))
    {
        assert!(started.elapsed() < Duration::from_secs(30));
        thread::sleep(Duration::from_millis(10));
    }
    let assets = cat
        .call(
            "query_assets",
            json!({"filter":{},"sort":{"field":"filename","direction":"asc"},"pageSize":10}),
        )
        .unwrap();
    let assets = assets["items"].as_array().unwrap();
    assert_eq!(assets.len(), 3);
    let ids: Vec<_> = assets.iter().map(|a| a["id"].clone()).collect();
    cat.call("apply_review_batch",json!({"target":{"kind":"explicit","assetIds":ids},"patch":{"pickState":"keep","rating":4,"isFavorite":true,"note":"磁盘满验收：保留人工整理"},"clientMutationId":"full-volume-review"})).unwrap();
    let originals = source_state(&source);
    let human = reviews(&cat);
    save(&output.join("source-before.json"), &originals);
    save(&output.join("reviews-before.json"), &human);
    cat.authorize(&destination, "export").unwrap();
    fs::write(
        destination.join("do-not-touch.txt"),
        b"existing destination content",
    )
    .unwrap();
    let request = json!({"target":{"kind":"explicit","assetIds":ids},"destinationPath":destination,"layout":"flat","conflictPolicy":"rename","includeXmp":true});
    let preview = cat.call("preview_export", request.clone()).unwrap();
    assert!(preview["warnings"].as_array().unwrap().is_empty());
    let free_before = fs2::available_space(&mount).unwrap();
    assert!(free_before > preview["totalBytes"].as_u64().unwrap());
    let filler = mount.join("owned-space-pressure.bin");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&filler)
        .unwrap();
    let buffer = vec![0xa7; MIB as usize];
    let mut written = 0;
    loop {
        let free = fs2::available_space(&mount).unwrap();
        if free <= 8 * MIB {
            break;
        }
        let count = (free - 8 * MIB).min(MIB) as usize;
        file.write_all(&buffer[..count]).unwrap();
        written += count as u64;
        assert!(written <= capacity);
        file.sync_all().unwrap();
    }
    drop(file);
    // Independent OS control proves this volume really returns ENOSPC, rather
    // than simulating a catalog error or using an unwritable directory.
    let control = mount.join("owned-enospc-control.bin");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&control)
        .unwrap();
    let mut attempted = 0;
    let control_error = loop {
        match file.write_all(&buffer) {
            Ok(()) => {
                attempted += MIB;
                assert!(attempted < 64 * MIB);
            }
            Err(error) => break error,
        }
    };
    assert_eq!(control_error.raw_os_error(), Some(28));
    if zero_space {
        // Consume any final sub-MiB allocation that the large-block control
        // could not use, so even the manifest must handle a full destination.
        for size in [4096, 1] {
            let mut bytes = 0;
            loop {
                match file.write_all(&buffer[..size]) {
                    Ok(()) => {
                        bytes += size;
                        assert!(bytes < 16 * MIB as usize);
                    }
                    Err(error) => {
                        assert_eq!(error.raw_os_error(), Some(28));
                        break;
                    }
                }
            }
        }
        if let Err(error) = file.sync_all() {
            assert_eq!(error.raw_os_error(), Some(28));
        }
    }
    let control_partial_bytes = file.metadata().unwrap().len();
    drop(file);
    if !zero_space {
        fs::remove_file(&control).unwrap();
    }
    let low_preview = cat.call("preview_export", request).unwrap();
    assert!(low_preview["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().contains("空间")));
    let free_at_start = fs2::available_space(&mount).unwrap();
    assert!(free_at_start < 64 * MIB && (zero_space || free_at_start > MIB));
    let job = cat
        .call(
            "start_export",
            json!({"planToken":preview["planToken"],"clientMutationId":"enospc-first"}),
        )
        .unwrap();
    let (terminal, part_peak) = wait(&cat, job["jobId"].as_str().unwrap(), &destination);
    let report = cat
        .call("get_export_report", json!({"jobId":job["jobId"]}))
        .unwrap();
    save(
        &output.join("failed-job.json"),
        &json!({"job":terminal,"report":report,"partPeakBytes":part_peak,"freeBefore":free_before,"freeAtStart":free_at_start,"osControlErrno":control_error.raw_os_error(),"osControlPartialBytes":control_partial_bytes}),
    );
    if zero_space {
        let entries: Vec<_> = fs::read_dir(&destination)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        save(
            &output.join("destination-after-failure.json"),
            &json!(entries),
        );
        assert_eq!(source_state(&source), originals);
        assert_eq!(reviews(&cat), human);
        assert_eq!(
            fs::read(destination.join("do-not-touch.txt")).unwrap(),
            b"existing destination content"
        );
        // Restore only this fixture's capacity before assertions, retaining the
        // raw failed job/output inventory even when a product defect is found.
        fs::remove_file(&control).unwrap();
        fs::remove_file(&filler).unwrap();
        assert_eq!(terminal["status"], "failed");
        assert!(
            terminal["errorSummary"].as_str().unwrap().contains("空间"),
            "Space failure must remain actionable: {terminal}"
        );
        assert_eq!(report["manifestPath"], Value::Null);
        assert!(
            entries
                .iter()
                .all(|name| !name.ends_with(".csv") && !name.contains("travelgallery-part")),
            "No partial manifest or temporary output: {entries:?}"
        );
        assert!(report["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["state"] == "failed")
            .all(|i| i["errorCode"] == "EXPORT_NO_SPACE"));
        save(
            &output.join("report.json"),
            &json!({"mode":"fully-exhausted-target","osEnospcErrno":28,"capacityBytes":capacity,"jobFailedWithActionableSpaceMessage":true,"noFinalManifestReported":true,"noPartialCsvOrPartLeft":true,"sourceAndHumanReviewUnchanged":true,"existingDestinationUnchanged":true}),
        );
        return;
    }
    assert_eq!(terminal["status"], "completed");
    assert_eq!(terminal["failureCount"], 1);
    assert_eq!(report["copiedCount"], 2);
    assert_eq!(report["verifiedCount"], 2);
    assert_eq!(report["failedCount"], 1);
    assert_eq!(report["xmpCopiedCount"], 2);
    assert_eq!(report["xmpSkippedCount"], 1);
    let failed = report["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["state"] == "failed")
        .unwrap();
    assert_eq!(failed["errorCode"], "EXPORT_NO_SPACE");
    assert_eq!(failed["destination"], "b-large.jpg");
    assert!(!destination.join("b-large.jpg").exists());
    assert!(fs::read_dir(&destination).unwrap().flatten().all(|e| !e
        .file_name()
        .to_string_lossy()
        .contains("travelgallery-part")));
    assert!(
        part_peak > 0,
        "Observe an actual partial copy before cleanup"
    );
    let manifest = destination.join(report["manifestPath"].as_str().unwrap());
    let manifest_hash = files::sha256(&manifest, None).unwrap();
    for name in ["a-small.jpg", "c-small.jpg"] {
        assert_eq!(
            files::sha256(&destination.join(name), None).unwrap(),
            files::sha256(&source.join(name), None).unwrap()
        );
    }
    assert_eq!(
        cat.call("retry_job", json!({"jobId":job["jobId"]}))
            .unwrap_err()
            .code,
        "EXPORT_CONFLICT"
    );
    fs::remove_file(&filler).unwrap();
    let retry_preview=cat.call("preview_export",json!({"target":{"kind":"explicit","assetIds":[failed["assetId"]]},"destinationPath":destination,"layout":"flat","conflictPolicy":"rename","includeXmp":true})).unwrap();
    assert!(retry_preview["warnings"].as_array().unwrap().is_empty());
    let retry=cat.call("start_export",json!({"planToken":retry_preview["planToken"],"clientMutationId":"enospc-after-release"})).unwrap();
    let retry_job = wait(&cat, retry["jobId"].as_str().unwrap(), &destination).0;
    let recovered = cat
        .call("get_export_report", json!({"jobId":retry["jobId"]}))
        .unwrap();
    save(
        &output.join("recovered-job.json"),
        &json!({"job":retry_job,"report":recovered}),
    );
    assert_eq!(retry_job["status"], "completed");
    assert_eq!(retry_job["failureCount"], 0);
    assert_eq!(recovered["copiedCount"], 1);
    assert_eq!(recovered["xmpCopiedCount"], 1);
    assert_eq!(
        files::sha256(&destination.join("b-large.jpg"), None).unwrap(),
        files::sha256(&source.join("b-large.jpg"), None).unwrap()
    );
    assert_eq!(files::sha256(&manifest, None).unwrap(), manifest_hash);
    assert_eq!(
        cat.call("get_export_report", json!({"jobId":job["jobId"]}))
            .unwrap(),
        report
    );
    assert_eq!(
        fs::read(destination.join("do-not-touch.txt")).unwrap(),
        b"existing destination content"
    );
    assert_eq!(source_state(&source), originals);
    assert_eq!(reviews(&cat), human);
    save(&output.join("source-after.json"), &source_state(&source));
    save(&output.join("reviews-after.json"), &reviews(&cat));
    save(
        &output.join("report.json"),
        &json!({"filesystem":"APFS mounted256MB disk image","capacityBytes":capacity,"preflightHadEnoughSpace":true,"spaceConsumedAfterPreflight":true,"osEnospcErrno":28,"observedPartPeakBytes":part_peak,"failedMedia":1,"preservedVerifiedMedia":2,"preservedSidecars":2,"newPreflightRecoveryVerified":true,"oldManifestAndReportUnchanged":true,"sourceShaSizeNanosecondMtimeUnchanged":true,"humanReviewFieldsUnchanged":true,"existingDestinationSentinelUnchanged":true,"scope":"Actual filesystem ENOSPC on owned virtual disk; not physical power loss, real removable hardware, Windows or GUI acceptance."}),
    );
}
