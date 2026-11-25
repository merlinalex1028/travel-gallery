//! Reproducible D-02 dataset and diagnostic calibration. Never alters input media.
use image::{codecs::jpeg::JpegEncoder, DynamicImage};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use travel_gallery::{catalog::Catalog, files, media};
fn jpeg(path: &Path, image: &DynamicImage, quality: u8) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .unwrap();
    JpegEncoder::new_with_quality(file, quality)
        .encode_image(&image.to_rgb8())
        .unwrap();
}
fn record(rows: &mut Vec<Value>, root: &Path, file: &str, origin: &str, category: &str) {
    let path = root.join(file);
    rows.push(json!({"file":file,"origin":origin,"category":category,"sha256":files::sha256(&path,None).unwrap(),"byteSize":fs::metadata(path).unwrap().len()}));
}
fn wait(cat: &Catalog, job: Option<&str>) {
    let start = Instant::now();
    loop {
        let list = cat
            .call("list_jobs", json!({"cursor":null,"pageSize":500}))
            .unwrap();
        let pending = list["items"].as_array().unwrap().iter().any(|j| {
            job.is_none_or(|id| j["id"] == id)
                && ["queued", "running", "paused"].contains(&j["status"].as_str().unwrap())
        });
        if !pending {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(180), "jobs timed out");
        thread::sleep(Duration::from_millis(50));
    }
}
fn scan(cat: &Catalog, kind: &str, snapshot: &Value) -> Value {
    let result=cat.call("start_duplicate_scan",json!({"kind":kind,"scope":{"kind":"snapshot","snapshotId":snapshot["id"]},"similarity":{"maxHammingDistance":8,"timeWindowSeconds":0,"algorithmVersion":"1"}})).unwrap();
    wait(cat, result["jobId"].as_str());
    let job = cat
        .call("get_job", json!({"jobId":result["jobId"]}))
        .unwrap();
    assert_eq!(job["status"], "completed", "{job}");
    assert_eq!(job["failureCount"], 0, "{job}");
    job
}
fn main() {
    let input = PathBuf::from(
        std::env::var("TRAVEL_GALLERY_MEDIA_FIXTURES").expect("public input samples"),
    );
    let root = PathBuf::from(
        std::env::var("TRAVEL_GALLERY_DUPLICATE_FIXTURES").expect("new external fixture directory"),
    );
    assert!(root.is_absolute());
    let project = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    let parent = root.parent().unwrap().canonicalize().unwrap();
    assert!(
        !parent.starts_with(&project),
        "put media outside the repository"
    );
    fs::create_dir(&root).expect("fixture directory must be new; never overwrite samples");
    let manifest: Vec<Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/media-samples-manifest.json")).unwrap();
    let originals: Vec<_> = manifest
        .iter()
        .filter(|r| r["file"].as_str().unwrap().to_lowercase().ends_with(".jpg"))
        .take(16)
        .collect();
    let mut rows = vec![];
    let started = Instant::now();
    for (index, entry) in originals.iter().enumerate() {
        let name = entry["file"].as_str().unwrap();
        let source = input.join(name);
        assert_eq!(
            files::sha256(&source, None).unwrap(),
            entry["sha256"].as_str().unwrap()
        );
        let origin = format!("photo-{index:02}");
        let folder = root.join(&origin);
        fs::create_dir(&folder).unwrap();
        for variant in ["original", "renamed-copy"] {
            let file = format!("{origin}/{variant}.jpg");
            fs::copy(&source, root.join(&file)).unwrap();
            record(&mut rows, &root, &file, &origin, variant);
        }
        let metadata = media::metadata(&source, "jpg", "image", files::stamp(&source).unwrap().1);
        let image = media::decode(
            &source,
            metadata.values["orientation"].as_u64().unwrap_or(1) as u32,
        )
        .unwrap()
        .thumbnail(1024, 1024);
        let (w, h) = (image.width(), image.height());
        let variants = [
            ("recompressed", image.clone(), 55),
            ("brightened", image.brighten(20), 90),
            (
                "cropped",
                image.crop_imm(w / 10, h / 10, (w * 8 / 10).max(1), (h * 8 / 10).max(1)),
                90,
            ),
            ("rotated90", image.rotate90(), 90),
            (
                "resized",
                image.thumbnail((w / 2).max(1), (h / 2).max(1)),
                90,
            ),
            (
                "simulated-frame-shift",
                image.crop_imm(w / 50, h / 50, (w * 97 / 100).max(1), (h * 97 / 100).max(1)),
                90,
            ),
            ("grayscale", image.grayscale(), 90),
            ("darkened", image.brighten(-30), 90),
        ];
        for (category, image, quality) in variants {
            let file = format!("{origin}/{category}.jpg");
            jpeg(&root.join(&file), &image, quality);
            record(&mut rows, &root, &file, &origin, category);
        }
    }
    // Same filename, demonstrably different generated pixels and bytes.
    for i in 0..10 {
        for side in 0..2 {
            let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(192, 128, |x, y| {
                let seed = 17 + i * 31 + side * 97;
                image::Rgb([
                    ((x * (1 + side) + seed) % 256) as u8,
                    ((y * 3 + seed) % 256) as u8,
                    ((x * y + seed * 7) % 256) as u8,
                ])
            }));
            let file = format!("same-name/{i:02}/{side}/IMG_{i:04}.jpg");
            jpeg(&root.join(&file), &image, 90);
            record(
                &mut rows,
                &root,
                &file,
                &format!("negative-{i}-{side}"),
                "same-name-different-content",
            );
        }
    }
    let raws: Vec<_> = manifest
        .iter()
        .filter(|r| {
            Path::new(r["file"].as_str().unwrap())
                .extension()
                .is_some_and(|e| media::is_raw(&e.to_string_lossy().to_lowercase()))
        })
        .collect();
    assert!(raws.len() >= 2);
    for i in 0..10 {
        let entry = raws[i % raws.len()];
        let source = input.join(entry["file"].as_str().unwrap());
        assert_eq!(
            files::sha256(&source, None).unwrap(),
            entry["sha256"].as_str().unwrap()
        );
        let origin = format!("raw-origin-{}", i % raws.len());
        let folder = format!("raw-jpeg/{i:02}");
        fs::create_dir_all(root.join(&folder)).unwrap();
        let ext = source.extension().unwrap().to_string_lossy().to_lowercase();
        let file = format!("{folder}/PAIR_{i:02}.{ext}");
        fs::copy(&source, root.join(&file)).unwrap();
        record(&mut rows, &root, &file, &origin, "raw-companion");
        let decoded = media::decode(&source, 1).unwrap();
        let file = format!("{folder}/PAIR_{i:02}.jpg");
        jpeg(&root.join(&file), &decoded, 90);
        record(&mut rows, &root, &file, &origin, "jpeg-from-raw-preview");
    }
    assert_eq!(rows.len(), 200);
    let before: Vec<_> = rows
        .iter()
        .map(|r| files::stamp(&root.join(r["file"].as_str().unwrap())).unwrap())
        .collect();
    let temp = tempfile::tempdir().unwrap();
    let cat = Catalog::new(&temp.path().join("data"), &temp.path().join("cache")).unwrap();
    cat.authorize(&root, "source").unwrap();
    cat.call("add_source",json!({"selectedPath":root,"options":{"recursive":true,"skipHidden":true,"excludeGlobs":[],"followSymlinks":false}})).unwrap();
    wait(&cat, None);
    let snapshot = cat
        .call(
            "create_selection_snapshot",
            json!({"filter":{"combineLinked":false},"sort":{"field":"filename","direction":"asc"}}),
        )
        .unwrap();
    assert_eq!(snapshot["count"], 200);
    let exact_job = scan(&cat, "exact", &snapshot);
    let similar_job = scan(&cat, "similar", &snapshot);
    let c = cat.connection().unwrap();
    let mut by_path = HashMap::new();
    let mut statement = c
        .prepare("SELECT id,relative_path,sha256,perceptual_hash,width,height FROM media_assets")
        .unwrap();
    for row in statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
            ))
        })
        .unwrap()
    {
        let (id, path, sha, hash, w, h) = row.unwrap();
        by_path.insert(
            path,
            (id, sha, u64::from_str_radix(&hash, 16).unwrap(), w, h),
        );
    }
    let exact_members:Vec<(String,String)>=c.prepare("SELECT group_id,asset_id FROM similar_group_items JOIN similar_groups ON id=group_id WHERE group_type='exact'").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().map(|r|r.unwrap()).collect();
    let mut expected_exact = 0;
    let mut observed_exact = 0;
    let mut false_exact = 0;
    let mut diagnostics = vec![];
    let mut thresholds = BTreeMap::new();
    for threshold in [4u32, 8, 12, 16] {
        thresholds.insert(threshold, (0usize, 0usize, 0usize, 0usize));
    }
    for i in 0..rows.len() {
        for j in i + 1..rows.len() {
            let a = &rows[i];
            let b = &rows[j];
            let aa = &by_path[a["file"].as_str().unwrap()];
            let bb = &by_path[b["file"].as_str().unwrap()];
            let same_bytes = a["sha256"] == b["sha256"];
            let same_group = exact_members.iter().any(|(g, id)| {
                id == &aa.0
                    && exact_members
                        .iter()
                        .any(|(gg, iid)| gg == g && iid == &bb.0)
            });
            if same_bytes {
                expected_exact += 1;
            }
            if same_group {
                observed_exact += 1;
                if !same_bytes {
                    false_exact += 1;
                }
            }
            assert_eq!(
                same_group, same_bytes,
                "exact grouping pair mismatch: {} / {}",
                a["file"], b["file"]
            );
            let distance = (aa.2 ^ bb.2).count_ones();
            let same_origin = a["origin"] == b["origin"];
            for (&threshold, counts) in &mut thresholds {
                if same_origin {
                    counts.2 += 1;
                    if distance <= threshold {
                        counts.0 += 1;
                    }
                } else {
                    counts.3 += 1;
                    if distance <= threshold {
                        counts.1 += 1;
                    }
                }
            }
            if a["category"] == "original" && same_origin && b["category"] != "renamed-copy" {
                diagnostics.push(json!({"base":a["file"],"variant":b["file"],"category":b["category"],"hammingDistance":distance}));
            }
        }
    }
    let mut unchanged = true;
    for (i, row) in rows.iter_mut().enumerate() {
        let path = root.join(row["file"].as_str().unwrap());
        unchanged &= files::sha256(&path, None).unwrap() == row["sha256"].as_str().unwrap()
            && files::stamp(&path).unwrap() == before[i];
        let a = &by_path[row["file"].as_str().unwrap()];
        row["dhash64"] = json!(format!("{:016x}", a.2));
    }
    let changed:i64=c.query_row("SELECT count(*) FROM asset_reviews WHERE pick_state<>'unreviewed' OR rating<>0 OR is_favorite<>0",[],|r|r.get(0)).unwrap();
    assert!(unchanged);
    assert_eq!(changed, 0);
    let similar_groups: i64 = c
        .query_row(
            "SELECT count(*) FROM similar_groups WHERE group_type='similar'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let calibration:Vec<_>=thresholds.into_iter().map(|(threshold,(same,cross,all_same,all_cross))|json!({"threshold":threshold,"sameOriginWithinDistance":same,"sameOriginPairs":all_same,"crossOriginWithinDistance":cross,"crossOriginPairs":all_cross,"ratioAndTimeGatesApplied":false})).collect();
    fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(&rows).unwrap(),
    )
    .unwrap();
    let report = json!({"clientDate":"2026-10-17","dataset":"D-02 controlled transformations v1","assetCount":200,"publicJpegOrigins":16,"publicRawOrigins":raws.len(),"realBurstSequenceCount":0,"simulatedFrameShiftCount":16,"exactExpectedPairs":expected_exact,"exactObservedPairs":observed_exact,"falseExactPairs":false_exact,"productionSimilarGroupsAtDistance8":similar_groups,"exactJob":exact_job,"similarJob":similar_job,"sourcesUnchanged":unchanged,"reviewRowsChanged":changed,"elapsedSeconds":started.elapsed().as_secs_f64(),"diagnosticPairThresholds":calibration,"transformDistances":diagnostics,"items":rows,"limitations":["Known transformation lineage is not a human visual-similarity label","Pairwise Hamming diagnostics omit production aspect/time gates and center-based clustering","RAW/JPEG companions are related files; repeated RAW seeds also create intentional byte duplicates","No real burst-motion dataset or held-out human calibration; thresholds remain candidates only"]});
    let output = std::env::var("TRAVEL_GALLERY_DUPLICATE_OUTPUT")
        .unwrap_or_else(|_| "docs/duplicate-calibration.json".into());
    fs::write(
        &output,
        serde_json::to_string_pretty(&report).unwrap() + "\n",
    )
    .unwrap();
    println!("D-02: 200 assets; exact {observed_exact}/{expected_exact} pairs; false exact {false_exact}; sources unchanged={unchanged}; report={output}");
}
