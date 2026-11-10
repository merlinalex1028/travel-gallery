//! Derived hints, deliberately separate from human reviews and face identity.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FaceHint {
    pub bounds: [f32; 4],
    pub blink_left: f32,
    pub blink_right: f32,
    pub eye_ratio_left: f32,
    pub eye_ratio_right: f32,
    pub status: String,
    pub reason: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EyeHints {
    pub faces: Vec<FaceHint>,
    pub face_limit_reached: bool,
    pub width: u32,
    pub height: u32,
}
impl EyeHints {
    pub fn validate(&self) -> bool {
        (1..=16384).contains(&self.width)
            && (1..=16384).contains(&self.height)
            && self.faces.len() <= 10
            && self.faces.iter().all(|f| {
                f.bounds
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                    && f.bounds[2] >= f.bounds[0]
                    && f.bounds[3] >= f.bounds[1]
                    && [f.blink_left, f.blink_right]
                        .iter()
                        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                    && [f.eye_ratio_left, f.eye_ratio_right]
                        .iter()
                        .all(|v| v.is_finite() && (0.0..=4.0).contains(v))
                    && ["possibleClosed", "possibleBlink", "noHint", "uncertain"]
                        .contains(&f.status.as_str())
                    && [
                        "bothEyes",
                        "oneEye",
                        "noStrongHint",
                        "smallFace",
                        "poseOrCrop",
                    ]
                    .contains(&f.reason.as_str())
            })
    }
    pub fn has_hint(&self) -> bool {
        self.faces
            .iter()
            .any(|f| matches!(f.status.as_str(), "possibleClosed" | "possibleBlink"))
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetailMetrics {
    pub laplacian_variance: f64,
    pub edge_energy: f64,
    pub measured_width: u32,
    pub measured_height: u32,
}
impl DetailMetrics {
    pub fn validate(&self) -> bool {
        self.laplacian_variance.is_finite()
            && (0.0..=2_000_000.0).contains(&self.laplacian_variance)
            && self.edge_energy.is_finite()
            && (0.0..=2_000_000.0).contains(&self.edge_energy)
            && (3..=512).contains(&self.measured_width)
            && (3..=512).contains(&self.measured_height)
    }
}
