//! Normalized model vectors and deterministic spherical k-means. Similarity is
//! not a quality score or calibrated probability. No review state is written.
use crate::error::{AppError, Result};
pub const DIM: usize = 768;
pub const CLUSTER_VERSION: &str = "spherical-kmeans/v1-k20-n10000-i15";
fn invalid() -> AppError {
    AppError::new("INVALID_FILTER", "内容向量或分组参数无效")
}
pub fn pack(v: &[f32]) -> Result<Vec<u8>> {
    valid(v, DIM)?;
    Ok(v.iter().flat_map(|n| n.to_le_bytes()).collect())
}
pub fn unpack(bytes: &[u8]) -> Result<Vec<f32>> {
    if bytes.len() != DIM * 4 {
        return Err(invalid());
    }
    let v: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    valid(&v, DIM)?;
    Ok(v)
}
fn valid(v: &[f32], dim: usize) -> Result<()> {
    if v.len() != dim
        || v.iter().any(|n| !n.is_finite())
        || (v.iter().map(|n| f64::from(*n).powi(2)).sum::<f64>() - 1.).abs() > 0.01
    {
        return Err(invalid());
    }
    Ok(())
}
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| a * b)
        .sum::<f32>()
        .clamp(-1., 1.)
}
pub fn group(
    vectors: &[Vec<f32>],
    k: usize,
    mut control: impl FnMut() -> Result<()>,
) -> Result<(Vec<usize>, Vec<f32>)> {
    if vectors.is_empty() || vectors.len() > 10_000 || !(1..=20).contains(&k) || k > vectors.len() {
        return Err(invalid());
    }
    let dim = vectors[0].len();
    if dim == 0 || dim > DIM {
        return Err(invalid());
    }
    for v in vectors {
        valid(v, dim)?;
    }
    let mut centers = vec![vectors[0].clone()];
    let mut used = vec![0];
    while centers.len() < k {
        let mut farthest = None;
        for (i, v) in vectors.iter().enumerate() {
            if i % 32 == 0 {
                control()?;
            }
            if used.contains(&i) {
                continue;
            }
            let distance = 1. - centers.iter().map(|c| cosine(v, c)).fold(-1., f32::max);
            if farthest.is_none_or(|(_, best)| distance > best) {
                farthest = Some((i, distance));
            }
        }
        let (index, _) = farthest.ok_or_else(invalid)?;
        used.push(index);
        centers.push(vectors[index].clone());
    }
    let mut assigned = vec![usize::MAX; vectors.len()];
    for _ in 0..15 {
        control()?;
        let mut changed = false;
        for (i, v) in vectors.iter().enumerate() {
            if i % 32 == 0 {
                control()?;
            }
            let mut best = (0, -2.);
            for (ci, c) in centers.iter().enumerate() {
                let score = cosine(v, c);
                if score > best.1 {
                    best = (ci, score);
                }
            }
            changed |= assigned[i] != best.0;
            assigned[i] = best.0;
        }
        if !changed {
            break;
        }
        let mut sums = vec![vec![0f64; dim]; k];
        let mut counts = vec![0usize; k];
        for (i, v) in vectors.iter().enumerate() {
            if i % 128 == 0 {
                control()?;
            }
            let ci = assigned[i];
            counts[ci] += 1;
            for (sum, n) in sums[ci].iter_mut().zip(v) {
                *sum += f64::from(*n);
            }
        }
        for ci in 0..k {
            if counts[ci] > 0 {
                let norm = sums[ci].iter().map(|n| n * n).sum::<f64>().sqrt();
                if norm > 1e-10 {
                    centers[ci] = sums[ci].iter().map(|n| (n / norm) as f32).collect();
                }
            }
        }
    }
    let mut scores = vec![];
    for (i, v) in vectors.iter().enumerate() {
        if i % 32 == 0 {
            control()?;
        }
        let mut best = (0, -2.);
        for (ci, c) in centers.iter().enumerate() {
            let score = cosine(v, c);
            if score > best.1 {
                best = (ci, score);
            }
        }
        assigned[i] = best.0;
        scores.push(best.1);
    }
    Ok((assigned, scores))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vectors_reject_nonfinite_wrong_size_and_nonunit_data() {
        let mut vector = vec![0.; DIM];
        vector[0] = 1.;
        assert_eq!(unpack(&pack(&vector).unwrap()).unwrap(), vector);
        vector[0] = f32::NAN;
        assert!(pack(&vector).is_err());
        assert!(unpack(&[0; DIM * 4]).is_err());
        assert!(unpack(&[0; 12]).is_err());
    }
    #[test]
    fn semantic_groups_are_stable_preserve_all_members_and_honor_cancel() {
        let unit = |x: f32, y: f32| {
            let n = (x * x + y * y).sqrt();
            vec![x / n, y / n]
        };
        let v = vec![unit(1., 0.), unit(1., 0.1), unit(-1., 0.), unit(-1., -0.1)];
        let a = group(&v, 2, || Ok(())).unwrap();
        assert_eq!(a, group(&v, 2, || Ok(())).unwrap());
        assert_eq!(a.0[0], a.0[1]);
        assert_eq!(a.0[2], a.0[3]);
        assert_ne!(a.0[0], a.0[2]);
        assert_eq!(a.1.len(), 4);
        assert!(group(&v, 5, || Ok(())).is_err());
        assert_eq!(
            group(&v, 2, || Err(AppError::new("CANCELLED", "cancel")))
                .unwrap_err()
                .code,
            "CANCELLED"
        );
        let same = group(&vec![unit(1., 0.); 4], 3, || Ok(())).unwrap();
        assert_eq!(same.0, vec![0; 4]);
        assert_eq!(same.1, vec![1.; 4]);
    }
}
