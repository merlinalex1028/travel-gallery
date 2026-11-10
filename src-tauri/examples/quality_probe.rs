//! Offline research only: no production score, model download or review writes.
use image::{DynamicImage, GrayImage, Luma};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
fn metrics(image: &GrayImage) -> Value {
    let (w, h) = image.dimensions();
    let mut lap = 0.;
    let mut lap2 = 0.;
    let mut gradient = 0.;
    let mut sum = 0.;
    let mut sum2 = 0.;
    let mut edges = 0usize;
    let p = |x: u32, y: u32| f64::from(image.get_pixel(x, y)[0]) / 255.;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let center = p(x, y);
            sum += center;
            sum2 += center * center;
            let l = p(x - 1, y) + p(x + 1, y) + p(x, y - 1) + p(x, y + 1) - 4. * center;
            lap += l;
            lap2 += l * l;
            let gx = (-p(x - 1, y - 1) + p(x + 1, y - 1) - 2. * p(x - 1, y) + 2. * p(x + 1, y)
                - p(x - 1, y + 1)
                + p(x + 1, y + 1))
                / 8.;
            let gy = (-p(x - 1, y - 1) - 2. * p(x, y - 1) - p(x + 1, y - 1)
                + p(x - 1, y + 1)
                + 2. * p(x, y + 1)
                + p(x + 1, y + 1))
                / 8.;
            let g = gx * gx + gy * gy;
            gradient += g;
            if g > 0.01 {
                edges += 1;
            }
        }
    }
    let n = f64::from((w - 2) * (h - 2));
    json!({"laplacianVariance":(lap2/n-(lap/n).powi(2)).max(0.),"tenengradMean":gradient/n,"contrastStdDev":(sum2/n-(sum/n).powi(2)).max(0.).sqrt(),"edgeFractionAtFixedProbeThreshold":edges as f64/n})
}
fn series(name: &str, kind: &str, image: GrayImage) -> Value {
    let start = Instant::now();
    let scores: Vec<Value> = [0., 0.6, 1.2, 2.4, 4.8]
        .into_iter()
        .map(|sigma| {
            let candidate = if sigma == 0. {
                image.clone()
            } else {
                image::imageops::blur(&image, sigma)
            };
            json!({"gaussianSigma":sigma,"metrics":metrics(&candidate)})
        })
        .collect();
    let monotonic = scores.windows(2).all(|pair| {
        pair[1]["metrics"]["laplacianVariance"].as_f64().unwrap()
            <= pair[0]["metrics"]["laplacianVariance"].as_f64().unwrap() + 1e-12
    });
    json!({"name":name,"kind":kind,"width":image.width(),"height":image.height(),"variants":scores,"laplacianMonotonicUnderProbeBlur":monotonic,"elapsedMs":start.elapsed().as_secs_f64()*1000.})
}
fn main() {
    let start = Instant::now();
    let mut rows = vec![];
    for (name, kind) in [
        ("checkerboard", 0),
        ("smooth_gradient", 1),
        ("edge_rectangle", 2),
        ("noisy_gradient", 3),
    ] {
        let image = GrayImage::from_fn(256, 256, |x, y| {
            let value = match kind {
                0 => {
                    if (x / 8 + y / 8) % 2 == 0 {
                        32
                    } else {
                        224
                    }
                }
                1 => x as u8,
                2 => {
                    if (64..192).contains(&x) && (64..192).contains(&y) {
                        224
                    } else {
                        32
                    }
                }
                _ => {
                    let mut seed = x.wrapping_mul(1_664_525) ^ y.wrapping_mul(1_013_904_223) ^ 17;
                    seed ^= seed >> 13;
                    seed = seed.wrapping_mul(1_274_126_177);
                    seed ^= seed >> 16;
                    (i32::from(x as u8) + (seed % 81) as i32 - 40).clamp(0, 255) as u8
                }
            };
            Luma([value])
        });
        rows.push(series(name, "synthetic", image));
    }
    let mut unchanged = true;
    let mut source_count = 0;
    if let Some(root) = std::env::var_os("TRAVEL_GALLERY_MEDIA_FIXTURES") {
        let root = PathBuf::from(root);
        let manifest: Value =
            serde_json::from_str(include_str!("../../docs/media-samples-manifest.json")).unwrap();
        for entry in manifest
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["file"].as_str().unwrap().ends_with(".jpg"))
            .take(12)
        {
            let name = entry["file"].as_str().unwrap();
            let path = root.join(name);
            let before = travel_gallery::files::sha256(&path, None).unwrap();
            let decoded = travel_gallery::media::decode(&path, 1).unwrap();
            let resized = DynamicImage::ImageLuma8(decoded.to_luma8())
                .thumbnail(512, 512)
                .to_luma8();
            rows.push(series(name, "public_jpeg", resized));
            source_count += 1;
            unchanged &= before == travel_gallery::files::sha256(&path, None).unwrap()
                && before == entry["sha256"].as_str().unwrap();
        }
    }
    let public_monotonic = rows
        .iter()
        .filter(|row| {
            row["kind"] == "public_jpeg" && row["laplacianMonotonicUnderProbeBlur"] == true
        })
        .count();
    let report = json!({"clientDate":"2026-10-17","researchOnly":true,"algorithmVersion":"laplacian4-sobel3/v1","normalization":"grayscale [0,1]; public inputs downsampled to max512; borders excluded","syntheticSeries":4,"publicSeries":source_count,"publicMonotonicCount":public_monotonic,"sourceHashesUnchanged":unchanged,"elapsedMs":start.elapsed().as_secs_f64()*1000.,"modelDownloaded":false,"productionReviewWrites":false,"series":rows,"limitations":["Synthetic blur order is not a human quality label","Texture/noise/exposure and resizing change absolute scores","Does not detect closed eyes, subject sharpness or aesthetics","No calibrated threshold, probability or automatic rejection"]});
    let output = std::env::var("TRAVEL_GALLERY_QUALITY_OUTPUT")
        .unwrap_or_else(|_| "docs/quality-probe.json".into());
    if let Some(parent) = Path::new(&output).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(
        &output,
        serde_json::to_string_pretty(&report).unwrap() + "\n",
    )
    .unwrap();
    println!("Quality research: {source_count} public series; {public_monotonic} monotonic; source hashes unchanged={unchanged}; report={output}");
}
