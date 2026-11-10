//! Explicit, local-only compatibility run against a provenance/hash manifest.
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use travel_gallery::{catalog::Catalog, files};
#[test]
#[ignore = "Requires TRAVEL_GALLERY_MEDIA_FIXTURES with public device samples"]
fn public_device_media_compatibility() {
    let root = PathBuf::from(std::env::var("TRAVEL_GALLERY_MEDIA_FIXTURES").expect("fixture root"));
    let manifest: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
    assert!(manifest.len() >= 60);
    for entry in &manifest {
        let path = root.join(entry["file"].as_str().unwrap());
        assert_eq!(
            files::sha256(&path, None).unwrap(),
            entry["sha256"].as_str().unwrap()
        );
    }
    let temp = tempfile::tempdir().unwrap();
    let cat = Catalog::new(&temp.path().join("data"), &temp.path().join("cache")).unwrap();
    cat.authorize(&root, "source").unwrap();
    let started = Instant::now();
    cat.call("add_source",json!({"selectedPath":root,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}})).unwrap();
    loop {
        let jobs = cat
            .call("list_jobs", json!({"cursor":null,"pageSize":100}))
            .unwrap();
        if jobs["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|j| !["queued", "running", "paused"].contains(&j["status"].as_str().unwrap()))
        {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(180));
        thread::sleep(Duration::from_millis(100));
    }
    let result=cat.call("query_assets",json!({"filter":{"combineLinked":false},"sort":{"field":"filename","direction":"asc"},"cursor":null,"pageSize":500})).unwrap();
    let mut results = vec![];
    for asset in result["items"].as_array().unwrap() {
        let detail = cat
            .call("get_asset", json!({"assetId":asset["id"]}))
            .unwrap();
        let preview = cat
            .call(
                "resolve_preview",
                json!({"assetId":asset["id"],"variant":"small"}),
            )
            .unwrap();
        let relative = asset["relativePath"].as_str().unwrap();
        let expected = manifest.iter().find(|entry| entry["file"] == relative);
        if let Some(expected) = expected {
            assert_eq!(
                files::sha256(&root.join(relative), None).unwrap(),
                expected["sha256"].as_str().unwrap()
            );
        }
        results.push(json!({"file":relative,"extension":asset["extension"],"device":detail["cameraModel"],"fileState":detail["fileState"],"width":detail["width"],"height":detail["height"],"codec":detail["codec"],"durationMs":detail["durationMs"],"previewReady":preview["ready"],"reason":preview["reason"],"captureTimeSource":detail["captureTimeSource"],"timezoneKnown":!detail["captureTzMin"].is_null(),"hasAppleContentIdentifier":detail["metadata"]["contentIdentifier"].is_string(),"sha256Unchanged":expected.is_some()}));
    }
    assert_eq!(
        results.len(),
        manifest.len() + 2,
        "64 device/test-archive media plus two deliberately bad fixtures"
    );
    let output = json!({"date":"2026-10-16","os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"buildProfile":if cfg!(debug_assertions){"debug"}else{"release"},"fixtureCount":manifest.len(),"badFixtures":2,"seconds":started.elapsed().as_secs_f64(),"sourceHashesUnchanged":true,"inlineWebviewPlaybackTested":false,"systemOpenTested":false,"results":results});
    let path = std::env::var("TRAVEL_GALLERY_COMPAT_OUTPUT").expect("report output path");
    fs::write(path, serde_json::to_string_pretty(&output).unwrap()).unwrap();
    println!(
        "COMPAT: {} files; {} previews ready; hashes unchanged",
        results.len(),
        results.iter().filter(|v| v["previewReady"] == true).count()
    );
}
