//! Bounded reads of the app-owned, orientation-normalized JPEG preview cache.
use crate::error::{AppError, Result};
use cap_std::fs::Dir;
use std::io::{Cursor, Read};
fn invalid() -> AppError {
    AppError::new("PREVIEW_FAILED", "预览缓存无效，请重新生成预览")
}
pub fn read_image(cache: &Dir, name: &str) -> Result<image::DynamicImage> {
    if name.len() != 68
        || !name.ends_with(".jpg")
        || !name.as_bytes()[..64].iter().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid());
    }
    let stat = cache.symlink_metadata(name)?;
    const LIMIT: u64 = 16 * 1024 * 1024;
    if !stat.is_file() || stat.file_type().is_symlink() || stat.len() > LIMIT {
        return Err(invalid());
    }
    let mut bytes = Vec::with_capacity(stat.len() as usize);
    cache.open(name)?.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != stat.len() || bytes.len() as u64 > LIMIT {
        return Err(invalid());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|_| invalid())
}
#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;
    #[test]
    fn rejects_paths_oversize_and_misnamed_data_before_decode() {
        let temp = tempfile::tempdir().unwrap();
        let dir = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
        assert!(read_image(&dir, "../image.jpg").is_err());
        let name = format!("{}.jpg", "a".repeat(64));
        image::RgbImage::new(32, 24)
            .save(temp.path().join(&name))
            .unwrap();
        assert_eq!(read_image(&dir, &name).unwrap().width(), 32);
        let f = dir.create(&name).unwrap();
        f.set_len(16 * 1024 * 1024 + 1).unwrap();
        assert!(read_image(&dir, &name).is_err());
        #[cfg(unix)]
        {
            dir.remove_file(&name).unwrap();
            std::os::unix::fs::symlink("outside.jpg", temp.path().join(&name)).unwrap();
            assert!(read_image(&dir, &name).is_err());
        }
    }
}
