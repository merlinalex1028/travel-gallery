//! Opt-in real model test. Never downloads anything or substitutes toy vectors.
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use travel_gallery::{catalog::Catalog, files};
fn wait(cat: &Catalog, jid: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let job = cat.call("get_job", json!({"jobId":jid})).unwrap();
        if ["completed", "failed", "cancelled"].contains(&job["status"].as_str().unwrap()) {
            return job;
        }
        assert!(Instant::now() < deadline, "{job}");
        thread::sleep(Duration::from_millis(20));
    }
}
fn done(cat: &Catalog, result: Value) -> Value {
    let job = wait(cat, result["jobId"].as_str().unwrap());
    assert_eq!(job["status"], "completed", "{job}");
    job
}
#[test]
#[ignore = "Requires the pinned offline model package and isolated public-image cache; no network"]
fn real_model_import_index_chinese_search_groups_and_cleanup_keep_sources_and_reviews_intact() {
    let package = PathBuf::from(
        std::env::var_os("TRAVEL_GALLERY_AI_MODEL_PACKAGE")
            .expect("set the offline model package directory"),
    );
    let cache = PathBuf::from(
        std::env::var_os("TRAVEL_GALLERY_AI_IMAGE_CACHE")
            .expect("set the isolated three-image probe cache"),
    );
    let manifest: Value =
        serde_json::from_str(include_str!("../../docs/local-ai-model-probe.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    fs::create_dir(&source).unwrap();
    for input in manifest["inputs"].as_array().unwrap() {
        let name = match input["filename"].as_str().unwrap() {
            "DSCN0040.jpg" => "wrong-beach.jpg",
            "DSCN0042.jpg" => "street.jpg",
            _ => "church.jpg",
        };
        fs::copy(
            cache.join(input["cacheFile"].as_str().unwrap()),
            source.join(name),
        )
        .unwrap();
    }
    let before = source_hashes(&source);
    let cat = Catalog::new(&temp.path().join("app"), &temp.path().join("cache")).unwrap();
    cat.authorize(&source, "source").unwrap();
    let added=cat.call("add_source",json!({"selectedPath":source,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}})).unwrap();
    done(&cat, added.clone());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let jobs = cat.call("list_jobs", json!({"pageSize":100})).unwrap();
        let items = jobs["items"].as_array().expect("job page items");
        if items
            .iter()
            .all(|job| !["queued", "running", "paused"].contains(&job["status"].as_str().unwrap()))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "thumbnail queue did not drain: {jobs}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let c = cat.connection().unwrap();
    let review_before:Vec<_>=c.prepare("SELECT asset_id,pick_state,rating,is_favorite,note,revision FROM asset_reviews ORDER BY asset_id").unwrap().query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,bool>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(
        cat.call("get_ai_status", json!({"filter":{}})).unwrap()["enabled"],
        false
    );
    cat.authorize(&package, "model").unwrap();
    done(
        &cat,
        cat.call(
            "import_ai_model",
            json!({"selectedPath":package,"clientMutationId":"real-import"}),
        )
        .unwrap(),
    );
    assert_eq!(
        cat.call("get_ai_status", json!({"filter":{}})).unwrap()["enabled"],
        false
    );
    cat.call("set_ai_enabled", json!({"enabled":true})).unwrap();
    let filter = json!({"combineLinked":false,"sourceIds":[added["sourceId"]]});
    done(
        &cat,
        cat.call(
            "start_ai_index",
            json!({"filter":filter,"clientMutationId":"real-index"}),
        )
        .unwrap(),
    );
    let status = cat.call("get_ai_status", json!({"filter":filter})).unwrap();
    assert_eq!(status["indexedCount"], 3);
    assert_eq!(status["runtime"], "loaded");
    let too_many_tokens = cat
        .call(
            "semantic_search",
            json!({"filter":filter,"text":"a ".repeat(80),"limit":3}),
        )
        .unwrap_err();
    assert_eq!(too_many_tokens.code, "INVALID_FILTER");
    assert!(too_many_tokens.message.contains("缩短"));
    assert_eq!(
        cat.call("get_ai_status", json!({"filter":filter})).unwrap()["runtime"],
        "loaded"
    );
    for (query, expected) in [
        ("一座教堂", "wrong-beach.jpg"),
        ("彩色条纹测试卡", "church.jpg"),
    ] {
        let results = cat
            .call(
                "semantic_search",
                json!({"filter":filter,"text":query,"limit":3}),
            )
            .unwrap();
        assert_eq!(
            results["items"][0]["asset"]["filename"], expected,
            "real pixels must outrank misleading source filenames: {results}"
        );
    }
    let result = cat
        .call(
            "start_ai_cluster",
            json!({"filter":filter,"groups":2,"clientMutationId":"real-clusters"}),
        )
        .unwrap();
    done(&cat, result.clone());
    let groups = cat
        .call("get_ai_clusters", json!({"filter":filter,"jobId":null}))
        .unwrap();
    assert_eq!(groups["jobId"], result["jobId"]);
    assert_eq!(groups["summary"]["clusteredCount"], 3);
    assert_eq!(
        groups["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["validCount"].as_u64().unwrap())
            .sum::<u64>(),
        3
    );
    cat.call("set_ai_enabled", json!({"enabled":false}))
        .unwrap();
    assert_eq!(
        cat.call("get_ai_status", json!({"filter":{}})).unwrap()["runtime"],
        "idle"
    );
    let review_after:Vec<_>=c.prepare("SELECT asset_id,pick_state,rating,is_favorite,note,revision FROM asset_reviews ORDER BY asset_id").unwrap().query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,bool>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(review_before, review_after);
    assert_eq!(before, source_hashes(&source));
    // Exercise real in-flight pause/resume/cancel and disable with a bounded,
    // separate public-image source. Search must still acquire the worker while
    // a background index is paused, and cancelled tasks must stop writing.
    let batch = temp.path().join("control-photos");
    fs::create_dir(&batch).unwrap();
    for i in 0..32 {
        fs::copy(
            source.join("wrong-beach.jpg"),
            batch.join(format!("control-{i:02}.jpg")),
        )
        .unwrap();
    }
    let batch_before = source_hashes(&batch);
    cat.authorize(&batch, "source").unwrap();
    let batch_source = cat.call("add_source", json!({"selectedPath":batch,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}})).unwrap();
    done(&cat, batch_source.clone());
    cat.call("set_ai_enabled", json!({"enabled":true})).unwrap();
    let batch_filter = json!({"combineLinked":false,"sourceIds":[batch_source["sourceId"]]});
    let batch_job = cat
        .call(
            "start_ai_index",
            json!({"filter":batch_filter,"clientMutationId":"real-controls-index"}),
        )
        .unwrap();
    let jid = batch_job["jobId"].as_str().unwrap();
    wait_progress(&cat, jid, 1);
    cat.call("pause_job", json!({"jobId":jid})).unwrap();
    let search = || {
        cat.call(
            "semantic_search",
            json!({"filter":filter,"text":"一座教堂","limit":3}),
        )
        .unwrap()
    };
    assert_eq!(search()["items"][0]["asset"]["filename"], "wrong-beach.jpg");
    let paused = cat.call("get_job", json!({"jobId":jid})).unwrap();
    assert_eq!(paused["status"], "paused");
    let paused_done = paused["progressDone"].as_u64().unwrap();
    assert!(paused_done < 32);
    search();
    assert_eq!(
        cat.call("get_job", json!({"jobId":jid})).unwrap()["progressDone"],
        paused_done
    );
    cat.call("resume_job", json!({"jobId":jid})).unwrap();
    wait_progress(&cat, jid, paused_done + 1);
    cat.call("cancel_job", json!({"jobId":jid})).unwrap();
    assert_eq!(wait(&cat, jid)["status"], "cancelled");
    search(); // Fence any in-flight request after cancellation.
    let partial = cat
        .call("get_ai_status", json!({"filter":batch_filter}))
        .unwrap()["indexedCount"]
        .as_u64()
        .unwrap();
    assert!((1..32).contains(&partial));
    let retry = cat
        .call(
            "start_ai_index",
            json!({"filter":batch_filter,"clientMutationId":"real-controls-disable"}),
        )
        .unwrap();
    wait_progress(&cat, retry["jobId"].as_str().unwrap(), partial + 1);
    cat.call("set_ai_enabled", json!({"enabled":false}))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = cat
            .call("get_ai_status", json!({"filter":batch_filter}))
            .unwrap();
        if status["runtime"] == "idle" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "disabling did not reap the worker: {status}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        wait(&cat, retry["jobId"].as_str().unwrap())["status"],
        "cancelled"
    );
    assert_eq!(batch_before, source_hashes(&batch));
    assert_eq!(before, source_hashes(&source));
    done(
        &cat,
        cat.call(
            "remove_ai_model",
            json!({"confirmed":true,"clientMutationId":"real-remove"}),
        )
        .unwrap(),
    );
    assert!(!temp
        .path()
        .join("app/models")
        .join(&travel_gallery::model_resources::profile().id)
        .exists());
    assert_eq!(
        c.query_row("SELECT count(*) FROM ai_embeddings", [], |r| r
            .get::<_, usize>(0))
            .unwrap(),
        0
    );
    assert_eq!(before, source_hashes(&source));
    for file in &travel_gallery::model_resources::profile().files {
        assert_eq!(
            files::sha256(&package.join(&file.path), None).unwrap(),
            file.sha256
        );
    }
}
fn source_hashes(path: &Path) -> Vec<(String, String, u64, u128)> {
    let mut rows: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                files::sha256(&entry.path(), None).unwrap(),
                fs::metadata(entry.path()).unwrap().len(),
                fs::metadata(entry.path())
                    .unwrap()
                    .modified()
                    .unwrap()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            )
        })
        .collect();
    rows.sort();
    rows
}

fn wait_progress(cat: &Catalog, jid: &str, minimum: u64) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let job = cat.call("get_job", json!({"jobId":jid})).unwrap();
        assert!(
            !["failed", "cancelled", "completed"].contains(&job["status"].as_str().unwrap()),
            "expected an in-flight index: {job}"
        );
        if job["progressDone"].as_u64().unwrap() >= minimum {
            return;
        }
        assert!(Instant::now() < deadline, "index did not advance: {job}");
        thread::sleep(Duration::from_millis(10));
    }
}
