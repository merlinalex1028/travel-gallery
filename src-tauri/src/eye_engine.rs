//! CPU-only Face Landmarker V1 adapter. Never loads arbitrary model code or
//! Subset: Copyright 2023 The MediaPipe Authors, Apache-2.0; see third-party/mediapipe-adapter/NOTICE.
use crate::{
    error::{AppError, Result},
    model_resources,
    quality_types::{EyeHints, FaceHint},
    tflite_compat,
};
use cap_std::{ambient_authority, fs::Dir};
use image::RgbImage;
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read},
    path::Path,
    sync::Arc,
};
use tract_tflite::prelude::*;
const SUBSET: [usize; 146] = [
    0, 1, 4, 5, 6, 7, 8, 10, 13, 14, 17, 21, 33, 37, 39, 40, 46, 52, 53, 54, 55, 58, 61, 63, 65,
    66, 67, 70, 78, 80, 81, 82, 84, 87, 88, 91, 93, 95, 103, 105, 107, 109, 127, 132, 133, 136,
    144, 145, 146, 148, 149, 150, 152, 153, 154, 155, 157, 158, 159, 160, 161, 162, 163, 168, 172,
    173, 176, 178, 181, 185, 191, 195, 197, 234, 246, 249, 251, 263, 267, 269, 270, 276, 282, 283,
    284, 285, 288, 291, 293, 295, 296, 297, 300, 308, 310, 311, 312, 314, 317, 318, 321, 323, 324,
    332, 334, 336, 338, 356, 361, 362, 365, 373, 374, 375, 377, 378, 379, 380, 381, 382, 384, 385,
    386, 387, 388, 389, 390, 397, 398, 400, 402, 405, 409, 415, 454, 466, 468, 469, 470, 471, 472,
    473, 474, 475, 476, 477,
];
const INNER: [(&str, u64, &str); 4] = [
    (
        "face_detector.tflite",
        229746,
        "b4578f35940bf5a1a655214a1cce5cab13eba73c1297cd78e1a04c2380b0152f",
    ),
    (
        "face_landmarks_detector.tflite",
        2553590,
        "c7d54204ce0448474c7f3fa9af494787c0965cbdd6f20fc72867e43046bd43d5",
    ),
    (
        "geometry_pipeline_metadata_landmarks.binarypb",
        19376,
        "bdbcda96dfcb7da883da124aaa2c55dee49770d934f0fcc71747f8c21bdc75b4",
    ),
    (
        "face_blendshapes.tflite",
        955312,
        "4f36dded049db18d76048567439b2a7f58f1daabc00d78bfe8f3ad396a2d2082",
    ),
];
fn failure() -> AppError {
    AppError::new(
        "QUALITY_INFERENCE_FAILED",
        "闭眼模型无法处理此项，请重试或重新导入模型",
    )
}
fn convert<T>(r: TractResult<T>) -> Result<T> {
    r.map_err(|_| failure())
}

pub struct Engine {
    detector: Arc<TypedRunnableModel>,
    mesh: Arc<TypedRunnableModel>,
    blend: Arc<TypedRunnableModel>,
}
/// Diagnostics are available only to explicit local research executables. The
/// desktop protocol serializes `EyeHints`, never these temporary landmarks.
pub struct DiagnosticFace {
    pub landmarks: Vec<[f32; 3]>,
    pub blendshapes: Vec<f32>,
    pub bounds: [f32; 4],
}
#[derive(Clone)]
struct Detection {
    score: f32,
    values: [f32; 16],
}
impl Engine {
    pub fn load(path: &Path) -> Result<Self> {
        let dir = Dir::open_ambient_dir(path, ambient_authority())?;
        model_resources::validate_files_for(&dir, model_resources::eye_profile(), || Ok(()))?;
        let resource = &model_resources::eye_profile().files[0];
        let mut bytes = Vec::with_capacity(resource.bytes as usize);
        dir.open(&resource.path)?
            .take(resource.bytes + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 != resource.bytes
            || format!("{:x}", Sha256::digest(&bytes)) != resource.sha256
        {
            return Err(failure());
        }
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| failure())?;
        if zip.len() != 4 {
            return Err(failure());
        }
        let mut models = Vec::new();
        for (name, size, hash) in INNER {
            let mut entry = zip.by_name(name).map_err(|_| failure())?;
            if entry.is_dir() || entry.size() != size {
                return Err(failure());
            }
            let mut bytes = Vec::with_capacity(size as usize);
            (&mut entry).take(size + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 != size || format!("{:x}", Sha256::digest(&bytes)) != hash {
                return Err(failure());
            }
            if name.ends_with(".tflite") {
                let model =
                    convert(tflite_compat::framework().model_for_read(&mut Cursor::new(bytes)))?;
                models.push(convert(convert(model.into_optimized())?.into_runnable())?);
            }
        }
        if models.len() != 3 {
            return Err(failure());
        }
        let blend = models.pop().unwrap();
        let mesh = models.pop().unwrap();
        let detector = models.pop().unwrap();
        Ok(Self {
            detector,
            mesh,
            blend,
        })
    }
    pub fn analyze(&self, image: &RgbImage) -> Result<EyeHints> {
        let (raw, limited) = self.diagnostics(image)?;
        let faces = raw
            .iter()
            .map(|f| hint(f, image.width(), image.height()))
            .collect();
        let result = EyeHints {
            faces,
            face_limit_reached: limited,
            width: image.width(),
            height: image.height(),
        };
        if !result.validate() {
            return Err(failure());
        }
        Ok(result)
    }
    pub fn diagnostics(&self, image: &RgbImage) -> Result<(Vec<DiagnosticFace>, bool)> {
        let (w, h) = (image.width() as f32, image.height() as f32);
        if w < 1. || h < 1. || w > 16384. || h > 16384. {
            return Err(failure());
        }
        let side = w.max(h);
        let input = warp(image, [w / 2., h / 2., side, 0.], 128, true)?;
        let out = convert(self.detector.run(tvec!(input.into())))?;
        if out.len() != 2 {
            return Err(failure());
        }
        let boxes = convert(out[0].to_plain_array_view::<f32>())?;
        let scores = convert(out[1].to_plain_array_view::<f32>())?;
        let boxes = boxes.as_slice().ok_or_else(failure)?;
        let scores = scores.as_slice().ok_or_else(failure)?;
        if boxes.len() != 896 * 16 || scores.len() != 896 {
            return Err(failure());
        }
        let mut detections = Vec::new();
        let mut index = 0;
        for (grid, repeats) in [(16, 2), (8, 6)] {
            for y in 0..grid {
                for x in 0..grid {
                    for _ in 0..repeats {
                        let score = sigmoid(scores[index]);
                        if score >= 0.5 {
                            let b = &boxes[index * 16..(index + 1) * 16];
                            let mut values = [0f32; 16];
                            let ax = (x as f32 + 0.5) / grid as f32;
                            let ay = (y as f32 + 0.5) / grid as f32;
                            for p in 0..8 {
                                if p == 1 {
                                    values[2] = b[2] / 128. * side;
                                    values[3] = b[3] / 128. * side;
                                } else {
                                    values[p * 2] = (b[p * 2] / 128. + ax - 0.5) * side + w / 2.;
                                    values[p * 2 + 1] =
                                        (b[p * 2 + 1] / 128. + ay - 0.5) * side + h / 2.;
                                }
                            }
                            if values.iter().any(|v| !v.is_finite())
                                || values[2] <= 0.
                                || values[3] <= 0.
                            {
                                return Err(failure());
                            }
                            detections.push(Detection { score, values });
                        }
                        index += 1;
                    }
                }
            }
        }
        let merged = weighted_nms(detections);
        let limited = merged.len() >= 10;
        let mut faces = Vec::new();
        for detection in merged.iter().take(10) {
            let b = detection.values;
            let rotation = (b[7] - b[5]).atan2(b[6] - b[4]);
            let crop = [b[0], b[1], b[2].max(b[3]) * 1.5, rotation];
            let input = warp(image, crop, 256, false)?;
            let out = convert(self.mesh.run(tvec!(input.into())))?;
            if out.len() != 3 {
                return Err(failure());
            }
            let points = convert(out[0].to_plain_array_view::<f32>())?;
            let presence = convert(out[1].to_plain_array_view::<f32>())?;
            let points = points.as_slice().ok_or_else(failure)?;
            if points.len() != 1434 || presence.len() != 1 {
                return Err(failure());
            }
            if sigmoid(*presence.iter().next().unwrap()) < 0.5 {
                continue;
            }
            let (sin, cos) = rotation.sin_cos();
            let landmarks: Vec<_> = points
                .chunks_exact(3)
                .map(|p| {
                    let x = (p[0] / 256. - 0.5) * crop[2];
                    let y = (p[1] / 256. - 0.5) * crop[2];
                    [
                        (crop[0] + cos * x - sin * y) / w,
                        (crop[1] + sin * x + cos * y) / h,
                        p[2] / 256. * crop[2] / w,
                    ]
                })
                .collect();
            if landmarks.iter().flatten().any(|v| !v.is_finite()) {
                return Err(failure());
            }
            let input: Vec<f32> = SUBSET
                .iter()
                .flat_map(|&i| [landmarks[i][0] * w, landmarks[i][1] * h])
                .collect();
            let input = convert(Tensor::from_shape(&[1, 146, 2], &input))?;
            let out = convert(self.blend.run(tvec!(input.into())))?;
            if out.len() != 1 {
                return Err(failure());
            }
            let blendshapes: Vec<f32> = convert(out[0].to_plain_array_view::<f32>())?
                .iter()
                .copied()
                .collect();
            if blendshapes.len() != 52
                || blendshapes
                    .iter()
                    .any(|v| !v.is_finite() || *v < -0.0001 || *v > 1.0001)
            {
                return Err(failure());
            }
            let bounds = [
                (b[0] - b[2] / 2.) / w,
                (b[1] - b[3] / 2.) / h,
                (b[0] + b[2] / 2.) / w,
                (b[1] + b[3] / 2.) / h,
            ]
            .map(|v| v.clamp(0., 1.));
            faces.push(DiagnosticFace {
                landmarks,
                blendshapes,
                bounds,
            });
        }
        Ok((faces, limited))
    }
}
fn sigmoid(v: f32) -> f32 {
    1. / (1. + (-v.clamp(-100., 100.)).exp())
}
fn iou(a: &Detection, b: &Detection) -> f32 {
    let (a, b) = (a.values, b.values);
    let x = ((a[0] + a[2] / 2.).min(b[0] + b[2] / 2.) - (a[0] - a[2] / 2.).max(b[0] - b[2] / 2.))
        .max(0.);
    let y = ((a[1] + a[3] / 2.).min(b[1] + b[3] / 2.) - (a[1] - a[3] / 2.).max(b[1] - b[3] / 2.))
        .max(0.);
    x * y / (a[2] * a[3] + b[2] * b[3] - x * y).max(1e-6)
}
fn weighted_nms(mut candidates: Vec<Detection>) -> Vec<Detection> {
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut result = Vec::new();
    while !candidates.is_empty() {
        let seed = candidates.remove(0);
        let mut merged = seed.clone();
        let mut weight = seed.score;
        merged.values.iter_mut().for_each(|v| *v *= seed.score);
        candidates.retain(|other| {
            if iou(&seed, other) > 0.3 {
                weight += other.score;
                for i in 0..16 {
                    merged.values[i] += other.values[i] * other.score;
                }
                false
            } else {
                true
            }
        });
        merged.values.iter_mut().for_each(|v| *v /= weight);
        result.push(merged);
    }
    result
}
fn warp(image: &RgbImage, rect: [f32; 4], size: usize, detector: bool) -> Result<Tensor> {
    let [cx, cy, side, rotation] = rect;
    let (sin, cos) = rotation.sin_cos();
    let mut values = Vec::with_capacity(size * size * 3);
    let pixel = |x: i32, y: i32, c: usize| -> i32 {
        if detector && (x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32) {
            0
        } else {
            i32::from(
                image.get_pixel(
                    x.clamp(0, image.width() as i32 - 1) as u32,
                    y.clamp(0, image.height() as i32 - 1) as u32,
                )[c],
            )
        }
    };
    for y in 0..size {
        for x in 0..size {
            let dx = (x as f32 / size as f32 - 0.5) * side;
            let dy = (y as f32 / size as f32 - 0.5) * side;
            let sx = ((cx + cos * dx - sin * dy) * 32.).round_ties_even() as i32;
            let sy = ((cy + sin * dx + cos * dy) * 32.).round_ties_even() as i32;
            let (x0, y0, fx, fy) = (sx >> 5, sy >> 5, sx & 31, sy & 31);
            for c in 0..3 {
                let sum = pixel(x0, y0, c) * (32 - fx) * (32 - fy)
                    + pixel(x0 + 1, y0, c) * fx * (32 - fy)
                    + pixel(x0, y0 + 1, c) * (32 - fx) * fy
                    + pixel(x0 + 1, y0 + 1, c) * fx * fy;
                let value = ((sum + 512) >> 10) as f32 / 255.;
                values.push(if detector { value * 2. - 1. } else { value });
            }
        }
    }
    convert(Tensor::from_shape(&[1, size, size, 3], &values))
}
fn hint(face: &DiagnosticFace, w: u32, h: u32) -> FaceHint {
    let p = |i: usize| {
        [
            face.landmarks[i][0] * w as f32,
            face.landmarks[i][1] * h as f32,
        ]
    };
    let dist = |i, j| {
        let a = p(i);
        let b = p(j);
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
    };
    let left = (dist(385, 380) + dist(387, 373)) / (2. * dist(362, 263).max(1e-5));
    let right = (dist(160, 144) + dist(158, 153)) / (2. * dist(33, 133).max(1e-5));
    let bl = face.blendshapes[9].clamp(0., 1.);
    let br = face.blendshapes[10].clamp(0., 1.);
    let bounds = face.bounds;
    let small = (bounds[2] - bounds[0]) * (w as f32) < 96.
        || (bounds[3] - bounds[1]) * (h as f32) < 96.
        || dist(362, 263) < 12.
        || dist(33, 133) < 12.;
    let spans = dist(362, 263) / dist(33, 133).max(1e-5);
    let crop = face
        .landmarks
        .iter()
        .take(468)
        .any(|p| p[0] < 0. || p[1] < 0. || p[0] > 1. || p[1] > 1.)
        || !(0.4..=2.5).contains(&spans)
        || left > 0.8
        || right > 0.8;
    let (status, reason) = if small {
        ("uncertain", "smallFace")
    } else if crop {
        ("uncertain", "poseOrCrop")
    } else if left <= 0.16 && right <= 0.16 && bl >= 0.35 && br >= 0.35 {
        ("possibleClosed", "bothEyes")
    } else if (left <= 0.14 && bl >= 0.5 && right >= 0.18)
        || (right <= 0.14 && br >= 0.5 && left >= 0.18)
    {
        ("possibleBlink", "oneEye")
    } else {
        ("noHint", "noStrongHint")
    };
    FaceHint {
        bounds,
        blink_left: bl,
        blink_right: br,
        eye_ratio_left: left.clamp(0., 4.),
        eye_ratio_right: right.clamp(0., 4.),
        status: status.into(),
        reason: reason.into(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn weighted_nms_preserves_distinct_people_and_averages_overlap() {
        let d = |x, score| Detection {
            score,
            values: [
                x, 10., 10., 10., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0.,
            ],
        };
        let a = weighted_nms(vec![d(10., 0.8), d(11., 0.6), d(100., 0.7)]);
        assert_eq!(a.len(), 2);
        assert!((a[0].values[0] - 10.428572).abs() < 1e-5);
        assert_eq!(a[1].values[0], 100.);
    }
    #[test]
    fn detector_padding_and_mesh_border_have_different_meanings() {
        let i = RgbImage::from_pixel(4, 2, image::Rgb([255, 128, 0]));
        let a = warp(&i, [2., 1., 4., 0.], 4, true).unwrap();
        let b = warp(&i, [2., 1., 4., 0.], 4, false).unwrap();
        assert_eq!(a.to_plain_array_view::<f32>().unwrap()[[0, 0, 0, 0]], -1.);
        assert_eq!(b.to_plain_array_view::<f32>().unwrap()[[0, 0, 0, 0]], 1.);
    }
}
