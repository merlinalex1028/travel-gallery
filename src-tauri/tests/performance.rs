use serde_json::json;
use std::time::Instant;
use travel_gallery::catalog::Catalog;

#[test]
#[ignore = "Run explicitly to record a machine-specific synthetic index baseline"]
fn fifty_thousand_record_baseline() {
    let count = std::env::var("TRAVEL_GALLERY_BENCH_COUNT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50000);
    assert!((10000..=100000).contains(&count));
    let temp = tempfile::tempdir().unwrap();
    let cat = Catalog::new(&temp.path().join("data"), &temp.path().join("cache")).unwrap();
    let c = cat.connection().unwrap();
    c.execute("INSERT INTO source_roots(id,display_name,absolute_path,created_at,updated_at) VALUES('bench','Synthetic index','/synthetic-only',0,0)",[]).unwrap();
    let build = Instant::now();
    c.execute_batch(&format!("BEGIN;
      WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM seq WHERE n<{count})
      INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,mtime_ms,capture_local,capture_utc_ms,indexed_at,updated_at)
      SELECT printf('asset-%06d',n),'bench',printf('day/%06d.jpg',n),printf('%06d.jpg',n),'jpg','image',3000000+n,1700000000000+n*1000,datetime(1700000000+n,'unixepoch'),1700000000000+n*1000,0,0 FROM seq;
      UPDATE asset_reviews SET pick_state='keep',rating=4 WHERE asset_id IN(SELECT id FROM media_assets WHERE byte_size%3=0);
      COMMIT; ANALYZE;")) .unwrap();
    let seed_ms = build.elapsed().as_secs_f64() * 1000.;
    drop(c);
    let request = json!({"filter":{"pickStates":["keep"],"ratingMin":4,"mediaKinds":["image"]},"sort":{"field":"captureUtcMs","direction":"asc"},"cursor":null,"pageSize":120});
    for _ in 0..5 {
        cat.call("query_assets", request.clone()).unwrap();
    }
    let mut samples = vec![];
    for _ in 0..100 {
        let start = Instant::now();
        let result = cat.call("query_assets", request.clone()).unwrap();
        samples.push(start.elapsed().as_secs_f64() * 1000.);
        assert_eq!(result["items"].as_array().unwrap().len(), 120);
        assert!(result["total"].as_u64().unwrap() >= (count / 3) as u64);
    }
    samples.sort_by(f64::total_cmp);
    let cold = Instant::now();
    let result=cat.call("query_assets",json!({"filter":{},"sort":{"field":"captureUtcMs","direction":"desc"},"cursor":null,"pageSize":120})).unwrap();
    let first_page = cold.elapsed().as_secs_f64() * 1000.;
    let mut neighbor_samples = vec![];
    for (index, anchor) in result["items"].as_array().unwrap().iter().take(100).enumerate() {
        let start = Instant::now();
        let window = cat.call("get_asset_neighbors", json!({"assetId":anchor["id"],"queryFingerprint":result["queryFingerprint"],"limit":5})).unwrap();
        neighbor_samples.push(start.elapsed().as_secs_f64() * 1000.);
        assert_eq!(window["total"], count);
        assert_eq!(window["position"], index + 1);
        assert_eq!(window["anchorMatches"], true);
    }
    neighbor_samples.sort_by(f64::total_cmp);
    let start = Instant::now();
    let snap = cat
        .call(
            "create_selection_snapshot",
            json!({"filter":{},"sort":{"field":"captureUtcMs","direction":"asc"}}),
        )
        .unwrap();
    let snapshot_ms = start.elapsed().as_secs_f64() * 1000.;
    assert_eq!(snap["count"], count);
    let data = json!({"dataset":format!("{count} synthetic SQLite records; no media decoding or disk scan"),"recordCount":count,"architecture":std::env::consts::ARCH,"os":std::env::consts::OS,"buildProfile":if cfg!(debug_assertions){"debug"}else{"release"},"queryRuns":100,"querySamplesSortedMs":samples,"neighborRuns":100,"neighborSamplesSortedMs":neighbor_samples,"neighborP50Ms":neighbor_samples[50],"neighborP95Ms":neighbor_samples[95],"percentileMethod":"sorted_samples[floor(n*q)], with zero-based index; p95 uses index95 of100", "dateUtc":chrono::Utc::now().to_rfc3339(),"seedMs":seed_ms,"queryP50Ms":samples[50],"queryP95Ms":samples[95],"firstPageMs":first_page,"firstPageRecords":result["items"].as_array().unwrap().len(),"snapshotMs":snapshot_ms});
    println!("PERF_JSON={data}");
    if let Ok(path) = std::env::var("TRAVEL_GALLERY_PERF_OUTPUT") {
        std::fs::write(path, serde_json::to_string_pretty(&data).unwrap()).unwrap();
    }
}
