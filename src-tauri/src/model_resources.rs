//! The supported model is pinned to author-published, data-only files. Neither
//! the desktop bridge nor the inference worker has a model download client.
use crate::error::{AppError, Result};
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub const PREPROCESS_VERSION: &str = "siglip2-screen-rgb-bilinear224-pad0-v1/candle-0.11.0";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub url: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub format_version: u32,
    pub id: String,
    pub name: String,
    pub kind: String,
    pub repository: String,
    pub revision: String,
    pub license: String,
    pub embedding_dimension: usize,
    pub image_size: usize,
    pub max_text_tokens: usize,
    pub files: Vec<ModelFile>,
}
pub fn profile() -> &'static Profile {
    static PROFILE: OnceLock<Profile> = OnceLock::new();
    PROFILE.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/ai/siglip2-base224.json"))
            .expect("bundled model profile must be valid")
    })
}
pub const EYE_PREPROCESS_VERSION: &str = "blazeface128-mesh256-blend146-cv32-v1/tract-0.23.9-r1";
pub fn eye_profile() -> &'static Profile {
    static PROFILE: OnceLock<Profile> = OnceLock::new();
    PROFILE.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/ai/face-landmarker-v1.json"))
            .expect("bundled face profile must be valid")
    })
}
pub fn eye_installed_path(data: &Path) -> PathBuf {
    data.join("models").join(&eye_profile().id)
}
pub fn installed_path(data: &Path) -> PathBuf {
    data.join("models").join(&profile().id)
}
pub fn invalid(message: &str) -> AppError {
    AppError::new("AI_MODEL_INVALID", message)
}
pub fn validate_marker(dir: &Dir) -> Result<()> {
    validate_marker_for(dir, profile())
}
pub fn validate_marker_for(dir: &Dir, expected: &Profile) -> Result<()> {
    let stat = dir.symlink_metadata("travel-gallery-model.json")?;
    if !stat.is_file() || stat.file_type().is_symlink() || stat.len() > 64 * 1024 {
        return Err(invalid("模型包标识无效，请选择完整的旅图离线模型包"));
    }
    let mut bytes = Vec::with_capacity(stat.len() as usize);
    dir.open("travel-gallery-model.json")?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != stat.len() || bytes.len() > 64 * 1024 {
        return Err(invalid("模型包标识在校验时发生变化"));
    }
    let supplied: Profile = serde_json::from_slice(&bytes)?;
    if &supplied != expected {
        return Err(invalid(
            "模型包版本或文件清单不受支持，请使用指定的离线模型包",
        ));
    }
    Ok(())
}
pub fn validate_files(dir: &Dir, control: impl FnMut() -> Result<()>) -> Result<()> {
    validate_files_for(dir, profile(), control)
}
pub fn validate_files_for(
    dir: &Dir,
    expected: &Profile,
    mut control: impl FnMut() -> Result<()>,
) -> Result<()> {
    validate_marker_for(dir, expected)?;
    for file in &expected.files {
        control()?;
        let stat = dir.symlink_metadata(&file.path)?;
        if !stat.is_file() || stat.file_type().is_symlink() || stat.len() != file.bytes {
            return Err(invalid(&format!("模型文件 {} 的大小或类型不符", file.path)));
        }
        let mut input = dir.open(&file.path)?;
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = vec![0; 1024 * 1024];
        loop {
            control()?;
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            if bytes > file.bytes {
                return Err(invalid("模型文件在校验时发生变化"));
            }
            hash.update(&buffer[..n]);
        }
        if bytes != file.bytes || format!("{:x}", hash.finalize()) != file.sha256 {
            return Err(invalid(&format!(
                "模型文件 {} 的 SHA-256 校验失败",
                file.path
            )));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn face_profile_is_small_separate_and_its_marker_does_not_match_content_model() {
        let p = eye_profile();
        assert_eq!(p.embedding_dimension, 0);
        assert_eq!(p.max_text_tokens, 0);
        assert_eq!(p.files.len(), 5);
        assert_eq!(p.files.iter().map(|f| f.bytes).sum::<u64>(), 10_005_871);
        assert_eq!(
            p.files[0].sha256,
            "64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff"
        );
        let temp = tempfile::tempdir().unwrap();
        let dir = Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        dir.write("travel-gallery-model.json", serde_json::to_vec(p).unwrap())
            .unwrap();
        validate_marker_for(&dir, p).unwrap();
        assert!(validate_marker(&dir).is_err());
    }
    #[test]
    fn pinned_profile_is_data_only_and_has_bounded_fixed_architecture() {
        let p = profile();
        assert_eq!(p.embedding_dimension, 768);
        assert_eq!(p.image_size, 224);
        assert_eq!(p.max_text_tokens, 64);
        assert_eq!(p.revision.len(), 40);
        assert_eq!(p.license, "Apache-2.0");
        assert_eq!(p.files.len(), 7);
        for f in &p.files {
            assert_eq!(Path::new(&f.path).file_name().unwrap(), f.path.as_str());
            assert_eq!(f.sha256.len(), 64);
            assert!(f.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(f.bytes > 0 && f.bytes < 2_000_000_000);
            assert!(f.url.starts_with("https://"));
            assert!(!f.path.ends_with(".bin") && !f.path.ends_with(".py"));
        }
    }
}
