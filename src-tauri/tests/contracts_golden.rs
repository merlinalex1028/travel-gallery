//! Stable serialization fixtures shared with the TypeScript contract test.
use serde_json::{json, Value};
use travel_gallery::{catalog::Catalog, error::AppError};
#[test]
fn public_dtos_match_shared_golden_json() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("IMG_0001.JPG"), b"read-only naming fixture").unwrap();
    let cat = Catalog::new(&temp.path().join("data"), &temp.path().join("cache")).unwrap();
    let c = cat.connection().unwrap();
    c.execute("INSERT INTO source_roots(id,display_name,absolute_path,created_at,updated_at) VALUES('golden-source','fixture',?,0,0)",[source.to_string_lossy().as_ref()]).unwrap();
    c.execute("INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,mtime_ms,capture_local,capture_utc_ms,capture_tz_min,capture_time_source,width,height,orientation,camera_make,camera_model,gps_lat,gps_lon,metadata_json,indexed_at,updated_at) VALUES('golden-photo','golden-source','IMG_0001.JPG','IMG_0001.JPG','jpg','image',12345,0,'2026-06-15 09:20:00',1781486400000,480,'exif',6000,4000,1,'Example','Camera',30.04,101.52,'{}',0,0)",[]).unwrap();
    let query = json!({"filter":{},"sort":{"field":"filename","direction":"asc"},"cursor":null,"pageSize":120});
    let mut page = cat.call("query_assets", query).unwrap();
    let summary = page["items"][0].clone();
    let mut detail = cat
        .call("get_asset", json!({"assetId":"golden-photo"}))
        .unwrap();
    // Only this platform-dependent path and generated opaque identifiers are normalized.
    detail["absolutePath"] = json!("<source>/IMG_0001.JPG");
    let neighbors = cat
        .call(
            "get_asset_neighbors",
            json!({"assetId":"golden-photo","queryFingerprint":page["queryFingerprint"],"limit":5}),
        )
        .unwrap();
    page["queryFingerprint"] = json!("<fingerprint>");
    let progress = cat
        .call(
            "get_cull_progress",
            json!({"assetId":"golden-photo","filter":{}}),
        )
        .unwrap();
    let mut receipt=cat.call("apply_review_batch",json!({"target":{"kind":"explicit","assetIds":["golden-photo"]},"patch":{"pickState":"keep","rating":4},"clientMutationId":"golden-write"})).unwrap();
    receipt["changeBatchId"] = json!("<batch>");
    let time=cat.call("save_capture_time",json!({"assetId":"golden-photo","captureLocal":"2026-06-15 10:30:00","captureTzMin":null,"expectedRevision":0,"clientMutationId":"golden-time"})).unwrap();
    c.execute("INSERT INTO jobs(id,type,status,created_at,updated_at) VALUES('golden-export','EXPORT','completed',0,0)",[]).unwrap();
    c.execute("INSERT INTO export_run_items VALUES('golden-export','golden-photo','IMG_0001.JPG','copied',12345,NULL,'<sha256>',0)",[]).unwrap();
    c.execute("INSERT INTO export_sidecar_items VALUES('golden-export','golden-photo','IMG_0001.xmp','failed',0,'EXPORT_CONFLICT',NULL,0)",[]).unwrap();
    let export = cat
        .call("get_export_report", json!({"jobId":"golden-export"}))
        .unwrap();
    let mut preset=cat.call("save_export_preset",json!({"name":"旅行精选","options":{"layout":"captureMonth","conflictPolicy":"skip","includeLinked":true,"includeXmp":false,"naming":{"pattern":"{date}_{seq}","start":7,"padding":4}}})).unwrap();
    preset["id"] = json!("<preset>");
    let naming_job=cat.call("preview_rename",json!({"target":{"kind":"explicit","assetIds":["golden-photo"]},"rule":{"pattern":"{date}_{seq}","start":7,"padding":4}})).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let job = cat
            .call("get_job", json!({"jobId":naming_job["jobId"]}))
            .unwrap();
        if job["status"] == "completed" {
            break;
        }
        assert!(
            job["status"] != "failed" && std::time::Instant::now() < deadline,
            "{job}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut naming = cat
        .call(
            "get_rename_preview",
            json!({"jobId":naming_job["jobId"],"offset":0,"pageSize":100}),
        )
        .unwrap();
    naming["job"]["id"] = json!("<job>");
    naming["job"]["createdAt"] = json!(0);
    naming["job"]["updatedAt"] = json!(0);
    naming["expiresAt"] = json!(0);
    naming["target"]["snapshotId"] = json!("<snapshot>");
    let timeline = cat.call("timeline_stats", json!({"filter":{}})).unwrap();
    cat.call(
        "save_trip",
        json!({"id":"golden-trip","name":"旅行故事","description":"原始日期分章"}),
    )
    .unwrap();
    cat.call(
        "add_to_trip",
        json!({"tripId":"golden-trip","target":{"kind":"explicit","assetIds":["golden-photo"]}}),
    )
    .unwrap();
    let mut story = cat
        .call(
            "get_trip_story",
            json!({"tripId":"golden-trip","range":"all","offset":0,"pageSize":100,"epoch":null}),
        )
        .unwrap();
    story["epoch"] = json!(0);
    let ai_status = cat.call("get_ai_status", json!({"filter":{}})).unwrap();
    let ai_clusters = cat.call("get_ai_clusters", json!({"filter":{}})).unwrap();
    let quality_status = cat
        .call("get_quality_status", json!({"filter":{}}))
        .unwrap();
    let quality_results = cat
        .call(
            "get_quality_results",
            json!({"filter":{},"offset":0,"onlyHints":false,"includeDismissed":false}),
        )
        .unwrap();
    let asset_quality = cat
        .call("get_asset_quality", json!({"assetId":"golden-photo"}))
        .unwrap();
    let actual = json!({"qualityStatus":quality_status,"qualityResults":quality_results,"assetQuality":asset_quality,"aiStatus":ai_status,"aiClusters":ai_clusters,"story":story,"timeline":timeline,"exportPreset":preset,"renamePreview":naming,"assetSummary":summary,"assetDetail":detail,"page":page,"neighbors":neighbors,"cullProgress":progress,"reviewReceipt":receipt,"captureTimeReceipt":time,"exportReport":export,"error":AppError::new("INVALID_FILTER","筛选或输入参数无效")});
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/contracts.json");
    if std::env::var_os("TRAVEL_GALLERY_UPDATE_GOLDENS").is_some() {
        std::fs::write(&path, serde_json::to_string_pretty(&actual).unwrap() + "\n").unwrap();
    }
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(&path).expect("shared contract fixture is missing"),
    )
    .unwrap();
    assert_eq!(actual,expected,"DTO serialization changed; review both Rust and TypeScript contracts before updating the golden fixture");
}
