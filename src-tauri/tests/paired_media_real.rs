//! Opt-in integration check against unmodified, pinned public Live Photo files.
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use travel_gallery::{catalog::Catalog, files};

#[test]
#[ignore = "Requires pinned TRAVEL_GALLERY_RAW_JPEG_FIXTURES and a new TRAVEL_GALLERY_RAW_JPEG_OUTPUT"]
fn public_raw_jpeg_candidates_preview_and_confirm_without_changing_sources() {
    let input = PathBuf::from(std::env::var("TRAVEL_GALLERY_RAW_JPEG_FIXTURES").unwrap());
    let root = PathBuf::from(std::env::var("TRAVEL_GALLERY_RAW_JPEG_OUTPUT").unwrap());
    assert!(root.is_absolute() && !root.exists());
    let fixtures = [
        ("IMG_1994.cr2", "1b84266100b391267c4f75cad47dfdc0c4913308652d26eadc4fb429c9e72e31"),
        ("IMG_1994.JPG", "72d97acc591b95ad3f118ef02c103a140050e2462bd679fa846ec27beb1e63c1"),
        ("IMG_1997.cr2", "4bcb6beb9dc03ab9a60591785cea3914b6b591041c59ab54a3b598503130ca39"),
        ("IMG_1997.JPG", "4cecedf2353ebeffb05cda0d499a4669efab3bf012acf1e2a18b188541762e71"),
    ];
    let before: Vec<_> = fixtures.iter().map(|(name,hash)| {
        let path=input.join(name); let meta=path.metadata().unwrap();
        assert_eq!(files::sha256(&path,None).unwrap(),*hash);
        (meta.len(),meta.modified().unwrap())
    }).collect();
    fs::create_dir(&root).unwrap();
    let source=root.join("RAW 与 JPEG 📷"); fs::create_dir(&source).unwrap();
    for (name,_) in fixtures { fs::copy(input.join(name),source.join(name)).unwrap(); }
    let copied: Vec<_> = fixtures.iter().map(|(name,_)|source.join(name).metadata().unwrap().modified().unwrap()).collect();
    let cat=Catalog::new(&root.join("data"),&root.join("cache")).unwrap();
    cat.authorize(&source,"source").unwrap();
    cat.call("add_source",json!({"selectedPath":source,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}})).unwrap();
    let start=Instant::now();
    loop {
        let jobs=cat.call("list_jobs",json!({"pageSize":100})).unwrap();
        for job in jobs["items"].as_array().unwrap() {
            assert!(!["failed","cancelled"].contains(&job["status"].as_str().unwrap()),"{job}");
            assert_eq!(job["failureCount"],0,"{job}");
        }
        if jobs["items"].as_array().unwrap().iter().all(|j|j["status"]=="completed") { break; }
        assert!(start.elapsed()<Duration::from_secs(90));
        thread::sleep(Duration::from_millis(50));
    }
    let query=||cat.call("query_assets",json!({"filter":{"combineLinked":true},"sort":{"field":"filename","direction":"asc"},"pageSize":120})).unwrap();
    let all=query(); assert_eq!(all["total"],4);
    let c=cat.connection().unwrap();
    // A separate consistent snapshot preserves the unconfirmed candidates for
    // the later native manual-confirmation check; it is never a user catalog.
    travel_gallery::db::backup(&c,&root.join("native-unconfirmed.sqlite")).unwrap();
    let mut results=vec![];
    for asset in all["items"].as_array().unwrap() {
        let detail=cat.call("get_asset",json!({"assetId":asset["id"]})).unwrap();
        let preview=cat.call("resolve_preview",json!({"assetId":asset["id"],"variant":"screen"})).unwrap();
        assert_eq!(preview["ready"],true,"{detail} {preview}");
        assert_eq!(detail["review"]["revision"],0);
        if asset["extension"]=="cr2" {
            assert_eq!(detail["preview"]["canOriginalPreview"],false);
            let links=cat.call("get_media_links",json!({"assetId":asset["id"]})).unwrap();
            assert_eq!(links.as_array().unwrap().len(),1);
            assert_eq!(links[0]["linkType"],"raw_jpeg");
            assert_eq!(links[0]["matchMethod"],"same_stem");
            assert_eq!(links[0]["confirmed"],false);
            cat.call("link_media",json!({"assetId":asset["id"],"secondaryAssetId":links[0]["secondaryAssetId"],"linkType":"raw_jpeg"})).unwrap();
            results.push(json!({"raw":detail,"candidateBeforeConfirmation":links[0],"screenPreviewReady":true}));
        }
    }
    assert_eq!(query()["total"],2);
    assert_eq!(c.query_row("SELECT count(*) FROM linked_media WHERE link_type='raw_jpeg' AND confirmed=1",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert_eq!(c.query_row("SELECT sum(revision) FROM asset_reviews",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    for (i,(name,hash)) in fixtures.iter().enumerate() {
        let meta=input.join(name).metadata().unwrap(); assert_eq!((meta.len(),meta.modified().unwrap()),before[i]);
        assert_eq!(source.join(name).metadata().unwrap().modified().unwrap(),copied[i]);
        assert_eq!(files::sha256(&input.join(name),None).unwrap(),*hash);
        assert_eq!(files::sha256(&source.join(name),None).unwrap(),*hash);
    }
    fs::write(root.join("report.json"),serde_json::to_vec_pretty(&json!({"dateUtc":chrono::Utc::now().to_rfc3339(),"physicalAssets":4,"unconfirmedLogicalAssets":4,"confirmedLogicalAssets":2,"humanReviewChanges":0,"sourcesHashSizeAndNanosecondMtimeUnchanged":true,"nativeUiTested":false,"scope":"Real CR2 embedded-JPEG screen previews; not full RAW development","results":results})).unwrap()).unwrap();
}

#[test]
#[ignore = "Requires TRAVEL_GALLERY_PAIRED_FIXTURES and a new TRAVEL_GALLERY_PAIRED_OUTPUT directory"]
fn public_heic_mov_identifiers_pair_without_filename_hints() {
    let input = PathBuf::from(
        std::env::var("TRAVEL_GALLERY_PAIRED_FIXTURES").expect("public fixture root"),
    );
    let root =
        PathBuf::from(std::env::var("TRAVEL_GALLERY_PAIRED_OUTPUT").expect("new output directory"));
    assert!(root.is_absolute() && !root.exists());
    let fixtures = [
        (
            "IMG_1853.HEIC",
            "still-a.HEIC",
            "8e6fac35b135ca6756ce8fb3368ec4dc3613ea5c0db34aa1eeefd3988e26c625",
            "dc121c25-1a7c-4fec-bb42-99fb89a4719d",
        ),
        (
            "IMG_1853.mov",
            "motion-one.mov",
            "5d24de6ae010d9bdaff559ea8f736923967cbc8e3e382cc0489a5673adec436d",
            "dc121c25-1a7c-4fec-bb42-99fb89a4719d",
        ),
        (
            "IMG_6161.HEIC",
            "still-b.HEIC",
            "87c3ae4280f1ab7f8dd5d7d3d21a8a23ead8a27f9d4ebb9cbb3dbc42b744839d",
            "eb3decca-5151-4fb6-9da6-cbdee3f0290c",
        ),
        (
            "IMG_6161.mov",
            "motion-two.mov",
            "0dd8a48f8451716e9034a6a4829876d40380836cd66f7493da4abb8a75a09bea",
            "eb3decca-5151-4fb6-9da6-cbdee3f0290c",
        ),
    ];
    let mut before = vec![];
    for (name, _, hash, _) in fixtures {
        let path = input.join(name);
        assert_eq!(files::sha256(&path, None).unwrap(), hash);
        let metadata = path.metadata().unwrap();
        before.push((metadata.len(), metadata.modified().unwrap()));
    }
    fs::create_dir(&root).unwrap();
    let source = root.join("配套原件 📷");
    fs::create_dir(&source).unwrap();
    for (name, renamed, _, _) in fixtures {
        fs::copy(input.join(name), source.join(renamed)).unwrap();
    }
    let cat = Catalog::new(&root.join("data"), &root.join("cache")).unwrap();
    cat.authorize(&source, "source").unwrap();
    cat.call("add_source", json!({"selectedPath":source,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}})).unwrap();
    let start = Instant::now();
    loop {
        let jobs = cat
            .call("list_jobs", json!({"cursor":null,"pageSize":100}))
            .unwrap();
        for job in jobs["items"].as_array().unwrap() {
            assert!(
                !["failed", "cancelled"].contains(&job["status"].as_str().unwrap()),
                "Pair job failed: {job}"
            );
            assert_eq!(job["failureCount"], 0, "Pair job had file failures: {job}");
        }
        if jobs["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|job| job["status"] == "completed")
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(150),
            "Pair scan/preview did not finish: {jobs}"
        );
        thread::sleep(Duration::from_millis(100));
    }
    let query = |combined| {
        cat.call("query_assets", json!({"filter":{"combineLinked":combined},"sort":{"field":"filename","direction":"asc"},"cursor":null,"pageSize":120})).unwrap()
    };
    let physical = query(false);
    assert_eq!(physical["total"], 4);
    assert_eq!(query(true)["total"], 2);
    // Reproduce a version-2 HEIF catalog's unswapped EXIF dimensions, then
    // rescan the unchanged bytes. Its metadata must be refreshed safely.
    let legacy = physical["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["filename"] == "still-a.HEIC")
        .unwrap();
    let c = cat.connection().unwrap();
    c.execute("UPDATE media_assets SET width=4032,height=3024,metadata_json=json_set(metadata_json,'$._version',2) WHERE id=?", [legacy["id"].as_str().unwrap()]).unwrap();
    let source_id: String = c
        .query_row("SELECT id FROM source_roots", [], |r| r.get(0))
        .unwrap();
    cat.call("rescan_source", json!({"sourceId":source_id}))
        .unwrap();
    loop {
        let jobs = cat
            .call("list_jobs", json!({"cursor":null,"pageSize":100}))
            .unwrap();
        for job in jobs["items"].as_array().unwrap() {
            assert!(
                !["failed", "cancelled"].contains(&job["status"].as_str().unwrap()),
                "Legacy rescan failed: {job}"
            );
            assert_eq!(job["failureCount"], 0);
        }
        if jobs["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|job| job["status"] == "completed")
        {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(150));
        thread::sleep(Duration::from_millis(100));
    }
    let upgraded = cat
        .call("get_asset", json!({"assetId":legacy["id"]}))
        .unwrap();
    assert_eq!(
        c.query_row(
            "SELECT json_extract(metadata_json,'$._version') FROM media_assets WHERE id=?",
            [legacy["id"].as_str().unwrap()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        3
    );
    assert_eq!(upgraded["width"], 3024);
    assert_eq!(upgraded["height"], 4032);
    let mut results = vec![];
    for (_, renamed, _, expected) in fixtures {
        let asset = physical["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|asset| asset["filename"] == renamed)
            .unwrap();
        let detail = cat
            .call("get_asset", json!({"assetId":asset["id"]}))
            .unwrap();
        assert_eq!(detail["metadata"]["contentIdentifier"], expected);
        let expected_dimensions = if renamed == "still-a.HEIC" {
            Some((3024, 4032))
        } else if renamed == "still-b.HEIC" {
            Some((4032, 3024))
        } else {
            None
        };
        if let Some((width, height)) = expected_dimensions {
            assert_eq!(detail["width"], width);
            assert_eq!(detail["height"], height);
        }
        assert_eq!(detail["review"]["revision"], 0);
        let links = cat
            .call("get_media_links", json!({"assetId":asset["id"]}))
            .unwrap();
        assert_eq!(links.as_array().unwrap().len(), 1);
        assert_eq!(links[0]["linkType"], "live_photo");
        assert_eq!(links[0]["matchMethod"], "apple_content_identifier");
        assert_eq!(links[0]["confirmed"], true);
        let preview = cat
            .call(
                "resolve_preview",
                json!({"assetId":asset["id"],"variant":"small"}),
            )
            .unwrap();
        assert_eq!(
            preview["ready"], true,
            "Preview unavailable: {detail} {preview}"
        );
        if renamed.ends_with(".HEIC") {
            let (relative, format): (String, i64) = c.query_row("SELECT cache_rel_path,format_version FROM thumbnail_cache WHERE asset_id=? AND variant='small'", [asset["id"].as_str().unwrap()], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
            assert_eq!(format, 2);
            let actual = image::image_dimensions(root.join("cache").join(relative)).unwrap();
            assert_eq!(
                actual,
                if renamed == "still-a.HEIC" {
                    (192, 256)
                } else {
                    (256, 192)
                }
            );
        }
        results.push(json!({"asset":detail,"links":links,"smallPreviewReady":preview["ready"]}));
    }
    for (i, (name, renamed, hash, _)) in fixtures.iter().enumerate() {
        let path = input.join(name);
        let metadata = path.metadata().unwrap();
        assert_eq!((metadata.len(), metadata.modified().unwrap()), before[i]);
        assert_eq!(files::sha256(&path, None).unwrap(), *hash);
        assert_eq!(files::sha256(&source.join(renamed), None).unwrap(), *hash);
    }
    let report: Value = json!({"dateUtc":chrono::Utc::now().to_rfc3339(),"physicalAssets":4,"logicalAssets":2,"confirmedPairs":2,"filenameHintsUsed":false,"legacyHeifOrientationUpgraded":true,"sourceHashesAndOriginalNanosecondMtimeUnchanged":true,"humanReviewChanges":0,"inlinePlaybackTested":false,"nativeUiTested":false,"results":results});
    fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
