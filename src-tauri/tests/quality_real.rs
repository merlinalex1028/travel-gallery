//! Explicit real-model regression, isolated public inputs only; never downloads.
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
fn done(cat: &Catalog, r: Value) -> Value {
    let job = wait(cat, r["jobId"].as_str().unwrap());
    assert_eq!(job["status"], "completed", "{job}");
    assert_eq!(job["failureCount"], 0, "{job}");
    job
}
fn add(cat: &Catalog, path: &Path) -> Value {
    cat.authorize(path, "source").unwrap();
    let r=cat.call("add_source",json!({"selectedPath":path,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}})).unwrap();
    done(cat, r.clone());
    r
}
fn source_hashes(p: &Path) -> Vec<(String, String, u64, std::time::SystemTime)> {
    let mut rows = fs::read_dir(p)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            let m = p.metadata().unwrap();
            (
                p.file_name().unwrap().to_string_lossy().into(),
                files::sha256(&p, None).unwrap(),
                m.len(),
                m.modified().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}
fn progress(cat: &Catalog, jid: &str, minimum: u64) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let j = cat.call("get_job", json!({"jobId":jid})).unwrap();
        if j["progressDone"].as_u64().unwrap_or(0) >= minimum {
            return;
        }
        assert!(
            !["failed", "cancelled", "completed"].contains(&j["status"].as_str().unwrap()),
            "{j}"
        );
        assert!(Instant::now() < deadline, "{j}");
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
#[ignore = "Requires pinned offline face package and four public images; no network"]
fn real_eyes_import_analysis_reuse_control_and_model_removal_preserve_reviews_and_sources() {
    let package = PathBuf::from(
        std::env::var_os("TRAVEL_GALLERY_EYES_MODEL_PACKAGE")
            .expect("set pinned face model package"),
    );
    let samples = PathBuf::from(
        std::env::var_os("TRAVEL_GALLERY_EYES_SAMPLES")
            .expect("set isolated public sample directory"),
    );
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    fs::create_dir(&source).unwrap();
    for n in [
        "portrait.jpg",
        "closed-glasses.jpg",
        "partial-face.jpg",
        "closed-coffee.jpg",
    ] {
        fs::copy(samples.join(n), source.join(n)).unwrap();
    }
    let original = source_hashes(&source);
    let cat = Catalog::new(&temp.path().join("app"), &temp.path().join("cache")).unwrap();
    let added = add(&cat, &source);
    let c = cat.connection().unwrap();
    let human = || {
        c.prepare("SELECT asset_id,pick_state,rating,is_favorite,note,revision FROM asset_reviews ORDER BY asset_id").unwrap().query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,bool>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?))).unwrap().map(Result::unwrap).collect::<Vec<_>>()
    };
    let before = human();
    cat.authorize(&package, "model").unwrap();
    let import = json!({"selectedPath":package,"clientMutationId":"real-eyes-import"});
    let job = cat.call("import_eyes_model", import.clone()).unwrap();
    assert_eq!(cat.call("import_eyes_model", import).unwrap(), job);
    done(&cat, job);
    let filter = json!({"combineLinked":false,"sourceIds":[added["sourceId"]]});
    let status = cat
        .call("get_quality_status", json!({"filter":filter}))
        .unwrap();
    assert_eq!(status["modelAvailable"], true);
    assert_eq!(status["eyesEnabled"], false);
    assert_eq!(status["runtime"], "idle");
    cat.call("set_eyes_enabled", json!({"enabled":true}))
        .unwrap();
    cat.call("set_detail_enabled", json!({"enabled":true}))
        .unwrap();
    let job=done(&cat,cat.call("start_quality_analysis",json!({"filter":filter,"useEyes":true,"useDetail":true,"clientMutationId":"real-eyes-analysis"})).unwrap());
    assert_eq!(job["progressDone"], 8);
    let status = cat
        .call("get_quality_status", json!({"filter":filter}))
        .unwrap();
    assert_eq!(status["eyesCount"], 4);
    assert_eq!(status["detailCount"], 4);
    assert_eq!(status["runtime"], "loaded");
    let results = cat
        .call(
            "get_quality_results",
            json!({"filter":filter,"onlyHints":false,"includeDismissed":false,"offset":0}),
        )
        .unwrap();
    assert_eq!(results["total"], 4);
    for item in results["items"].as_array().unwrap() {
        let name = item["asset"]["filename"].as_str().unwrap();
        let a = item["analysis"]["observations"].as_array().unwrap();
        assert_eq!(a.len(), 2);
        let eye = a.iter().find(|v| v["kind"] == "eyes").unwrap();
        let serialized = eye.to_string();
        assert!(!serialized.contains("landmarks"));
        assert!(!serialized.contains("blendshapes"));
        let hints: travel_gallery::quality_types::EyeHints =
            serde_json::from_value(eye["value"].clone()).unwrap();
        assert!(hints.validate());
        if name == "partial-face.jpg" {
            assert!(hints.faces.is_empty());
        }
        if name == "portrait.jpg" {
            assert!(!hints.has_hint());
            assert_eq!(hints.faces.len(), 1);
        }
        if name == "closed-glasses.jpg" {
            assert!(hints.has_hint(), "{hints:?}");
        }
    }
    let reused=done(&cat,cat.call("start_quality_analysis",json!({"filter":filter,"useEyes":true,"useDetail":true,"clientMutationId":"real-eyes-reuse"})).unwrap());
    assert_eq!(reused["progressDone"], 8);
    assert_eq!(
        results,
        cat.call(
            "get_quality_results",
            json!({"filter":filter,"onlyHints":false,"includeDismissed":false,"offset":0})
        )
        .unwrap()
    );
    let batch = temp.path().join("controls");
    fs::create_dir(&batch).unwrap();
    for i in 0..96 {
        fs::copy(
            source.join("portrait.jpg"),
            batch.join(format!("control-{i:03}.jpg")),
        )
        .unwrap();
    }
    let batch_before = source_hashes(&batch);
    let b = add(&cat, &batch);
    let bf = json!({"combineLinked":false,"sourceIds":[b["sourceId"]]});
    let run = |key: &str| {
        cat.call(
            "start_quality_analysis",
            json!({"filter":bf,"useEyes":true,"useDetail":false,"clientMutationId":key}),
        )
        .unwrap()
    };
    let j = run("eyes-control-start");
    let jid = j["jobId"].as_str().unwrap();
    progress(&cat, jid, 1);
    cat.call("pause_job", json!({"jobId":jid})).unwrap();
    thread::sleep(Duration::from_millis(150));
    let paused = cat.call("get_job", json!({"jobId":jid})).unwrap();
    assert_eq!(paused["status"], "paused");
    let n = paused["progressDone"].as_u64().unwrap();
    assert!(n < 96);
    thread::sleep(Duration::from_millis(150));
    assert_eq!(
        cat.call("get_job", json!({"jobId":jid})).unwrap()["progressDone"],
        n
    );
    cat.call("resume_job", json!({"jobId":jid})).unwrap();
    progress(&cat, jid, n + 1);
    cat.call("cancel_job", json!({"jobId":jid})).unwrap();
    assert_eq!(wait(&cat, jid)["status"], "cancelled");
    let partial = cat
        .call("get_quality_status", json!({"filter":bf}))
        .unwrap()["eyesCount"]
        .as_u64()
        .unwrap();
    assert!((1..96).contains(&partial));
    let j = run("eyes-control-disable");
    progress(&cat, j["jobId"].as_str().unwrap(), partial + 1);
    cat.call("set_eyes_enabled", json!({"enabled":false}))
        .unwrap();
    assert_eq!(
        wait(&cat, j["jobId"].as_str().unwrap())["status"],
        "cancelled"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if cat
            .call("get_quality_status", json!({"filter":bf}))
            .unwrap()["runtime"]
            == "idle"
        {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        before,
        human()
            .into_iter()
            .filter(|r| before.iter().any(|old| old.0 == r.0))
            .collect::<Vec<_>>()
    );
    assert_eq!(original, source_hashes(&source));
    assert_eq!(batch_before, source_hashes(&batch));
    done(
        &cat,
        cat.call(
            "remove_eyes_model",
            json!({"confirmed":true,"clientMutationId":"real-eyes-remove"}),
        )
        .unwrap(),
    );
    assert!(
        !travel_gallery::model_resources::eye_installed_path(&temp.path().join("app")).exists()
    );
    assert_eq!(
        cat.call("get_quality_status", json!({"filter":filter}))
            .unwrap()["eyesCount"],
        0
    );
    assert_eq!(
        cat.call("get_quality_status", json!({"filter":filter}))
            .unwrap()["detailCount"],
        4
    );
    assert_eq!(original, source_hashes(&source));
}
