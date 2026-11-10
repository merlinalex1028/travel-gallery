use serde_json::{json, Value};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
use travel_gallery::{catalog::Catalog, files};
fn terminal(cat: &Catalog, job: &str) -> Value {
    let start = Instant::now();
    loop {
        let row = cat.call("get_job", json!({"jobId":job})).unwrap();
        if ["completed", "failed", "cancelled"].contains(&row["status"].as_str().unwrap()) {
            return row;
        }
        assert!(start.elapsed() < Duration::from_secs(180));
        thread::sleep(Duration::from_millis(30));
    }
}
fn stop_thumbnails(cat: &Catalog) {
    let jobs = cat
        .call("list_jobs", json!({"pageSize":100,"cursor":null}))
        .unwrap();
    for job in jobs["items"].as_array().unwrap() {
        if ["queued", "running", "paused"].contains(&job["status"].as_str().unwrap()) {
            let id = job["id"].as_str().unwrap();
            cat.call("cancel_job", json!({"jobId":id})).unwrap();
            terminal(cat, id);
        }
    }
}
#[test]
#[ignore = "Explicit 10k real-directory enumeration/incremental benchmark"]
fn ten_thousand_file_scan() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("来源 📷");
    fs::create_dir(&source).unwrap();
    let mut encodings = vec![];
    for ext in ["jpg", "png", "webp"] {
        let path = temp.path().join(format!("seed.{ext}"));
        image::RgbImage::from_pixel(24, 16, image::Rgb([60, 100, 140]))
            .save(&path)
            .unwrap();
        encodings.push((ext, fs::read(path).unwrap()));
    }
    for n in 0..10000 {
        let folder = source.join(format!("day-{:02}", n % 40));
        fs::create_dir_all(&folder).unwrap();
        let (ext, bytes) = &encodings[n % 3];
        fs::write(folder.join(format!("旅行-{n:05}.{ext}")), bytes).unwrap();
    }
    let before: Vec<_> = walkdir::WalkDir::new(&source)
        .into_iter()
        .filter_map(|v| v.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            (
                e.path().to_owned(),
                files::sha256(e.path(), None).unwrap(),
                files::stamp(e.path()).unwrap(),
            )
        })
        .collect();
    let cat = Catalog::new(&temp.path().join("data"), &temp.path().join("cache")).unwrap();
    cat.authorize(&source, "source").unwrap();
    let start = Instant::now();
    let first=cat.call("add_source",json!({"selectedPath":source,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}})).unwrap();
    let result = terminal(&cat, first["jobId"].as_str().unwrap());
    let initial_ms = start.elapsed().as_secs_f64() * 1000.;
    assert_eq!(result["status"], "completed");
    stop_thumbnails(&cat);
    let start = Instant::now();
    let second = cat
        .call("rescan_source", json!({"sourceId":first["sourceId"]}))
        .unwrap();
    let result = terminal(&cat, second["jobId"].as_str().unwrap());
    let incremental_ms = start.elapsed().as_secs_f64() * 1000.;
    assert_eq!(result["status"], "completed");
    stop_thumbnails(&cat);
    let page=cat.call("query_assets",json!({"filter":{},"sort":{"field":"filename","direction":"asc"},"pageSize":120,"cursor":null})).unwrap();
    assert_eq!(page["total"], 10000);
    for (path, hash, stamp) in &before {
        assert_eq!(&files::sha256(path, None).unwrap(), hash);
        assert_eq!(&files::stamp(path).unwrap(), stamp);
    }
    let report = json!({"date":"2026-10-16","os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"files":10000,"dataset":"Generated 24x16 JPEG/PNG/WebP in 40 Chinese/emoji directories; real independent files; local SSD","profile":"debug","initialScanMs":initial_ms,"incrementalScanMs":incremental_ms,"initialIndexCount":page["total"],"sourceSha256AndMtimeUnchanged":true,"fullThumbnailPassTested":false});
    fs::write(
        std::env::var("TRAVEL_GALLERY_SCAN_OUTPUT").expect("report path"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("SCAN_JSON={report}");
}
