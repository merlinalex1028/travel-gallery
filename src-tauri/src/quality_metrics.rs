//! Reproducible detail measurements, not an absolute quality/blur classifier.
use crate::{
    error::{AppError, Result},
    quality_types::DetailMetrics,
};
use image::DynamicImage;
pub const VERSION: &str = "detail-laplacian4-sobel3-luma512-v1";
pub fn measure(image: &DynamicImage) -> Result<DetailMetrics> {
    if image.width() < 3 || image.height() < 3 {
        return Err(AppError::new(
            "QUALITY_UNSUPPORTED",
            "照片尺寸太小，无法测量细节",
        ));
    }
    let gray = image
        .resize(512, 512, image::imageops::FilterType::Triangle)
        .to_luma8();
    let (w, h) = gray.dimensions();
    if w < 3 || h < 3 {
        return Err(AppError::new(
            "QUALITY_UNSUPPORTED",
            "照片尺寸太小，无法测量细节",
        ));
    }
    let p = |x: u32, y: u32| f64::from(gray.get_pixel(x, y)[0]);
    let (mut n, mut sum, mut square, mut edge) = (0f64, 0f64, 0f64, 0f64);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let lap = p(x - 1, y) + p(x + 1, y) + p(x, y - 1) + p(x, y + 1) - 4. * p(x, y);
            let gx = -p(x - 1, y - 1) + p(x + 1, y - 1) - 2. * p(x - 1, y) + 2. * p(x + 1, y)
                - p(x - 1, y + 1)
                + p(x + 1, y + 1);
            let gy = -p(x - 1, y - 1) - 2. * p(x, y - 1) - p(x + 1, y - 1)
                + p(x - 1, y + 1)
                + 2. * p(x, y + 1)
                + p(x + 1, y + 1);
            n += 1.;
            sum += lap;
            square += lap * lap;
            edge += (gx * gx + gy * gy) / 16.;
        }
    }
    let result = DetailMetrics {
        laplacian_variance: (square / n - (sum / n).powi(2)).max(0.),
        edge_energy: edge / n,
        measured_width: w,
        measured_height: h,
    };
    if !result.validate() {
        return Err(AppError::new("QUALITY_UNSUPPORTED", "细节测量结果无效"));
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detail_is_scale_bounded_and_blur_reduces_edges_on_same_scene() {
        let rgb = image::RgbImage::from_fn(512, 384, |x, y| {
            let v = if (x / 8 + y / 8) % 2 == 0 { 20 } else { 230 };
            image::Rgb([v, v, v])
        });
        let sharp = DynamicImage::ImageRgb8(rgb);
        let a = measure(&sharp).unwrap();
        let b = measure(&sharp.blur(2.)).unwrap();
        assert!(a.laplacian_variance > b.laplacian_variance * 2.);
        assert!(a.edge_energy > b.edge_energy);
        assert_eq!(
            measure(&DynamicImage::new_rgb8(1, 1)).unwrap_err().code,
            "QUALITY_UNSUPPORTED"
        );
        let flat = measure(&DynamicImage::new_rgb8(512, 512)).unwrap();
        assert_eq!(flat.laplacian_variance, 0.);
    }
}
