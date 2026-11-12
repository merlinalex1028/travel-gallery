use crate::{
    catalog::Catalog,
    error::{required, AppError, Result},
    files, id, now,
};
use image::{DynamicImage, ImageReader};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const METADATA_VERSION: i64 = 3;
pub fn metadata_version_current(ext: &str, version: i64) -> bool {
    // Version 3 only fixes HEIF EXIF axis normalization. Other version-2
    // metadata and matching previews remain valid, avoiding needless decoding.
    version == METADATA_VERSION || (version == 2 && !["heic", "heif"].contains(&ext))
}
pub fn preview_format_version(ext: &str) -> i64 {
    if ext.eq_ignore_ascii_case("heic") || ext.eq_ignore_ascii_case("heif") { 2 } else { 1 }
}
pub fn is_raw(ext: &str) -> bool {
    [
        "arw", "cr2", "cr3", "nef", "dng", "orf", "rw2", "raf", "pef", "srw",
    ]
    .contains(&ext)
}
pub fn kind(ext: &str) -> Option<&'static str> {
    if ["mp4", "mov", "m4v", "avi", "mkv", "mts", "m2ts", "webm"].contains(&ext) {
        Some("video")
    } else if [
        "jpg", "jpeg", "png", "webp", "gif", "tif", "tiff", "bmp", "heic", "heif",
    ]
    .contains(&ext)
        || is_raw(ext)
    {
        Some("image")
    } else {
        None
    }
}
pub(crate) fn run(mut cmd: Command, timeout: u64) -> Result<std::process::Output> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| AppError::new("ASSET_UNSUPPORTED", "本机未安装此格式的解码工具"))?;
    // Drain while the child runs: waiting before reading can deadlock on a full pipe.
    let stdout = child.stdout.take().unwrap();
    let reader = thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut output = vec![];
        let mut stdout = stdout;
        (&mut stdout)
            .take(1024 * 1024 + 1)
            .read_to_end(&mut output)?;
        std::io::copy(&mut stdout, &mut std::io::sink())?;
        Ok(output)
    });
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let stdout = reader
                .join()
                .map_err(|_| AppError::new("ASSET_UNSUPPORTED", "媒体探测读取失败"))??;
            if stdout.len() > 1024 * 1024 {
                return Err(AppError::new(
                    "ASSET_UNSUPPORTED",
                    "媒体探测输出超过安全上限",
                ));
            }
            return Ok(std::process::Output {
                status,
                stdout,
                stderr: vec![],
            });
        }
        if start.elapsed() > Duration::from_secs(timeout) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(AppError::new("ASSET_UNSUPPORTED", "媒体探测超时"));
        }
        thread::sleep(Duration::from_millis(30));
    }
}
fn tool(name: &str) -> PathBuf {
    // Tauri places external binaries beside the application executable on macOS/Windows.
    // A packaged build must not silently depend on Homebrew or a user's PATH.
    let filename = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    if let Some(path) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.join(&filename)))
    {
        if path.is_file() || cfg!(all(feature = "desktop", not(debug_assertions))) {
            return path;
        }
    }
    #[cfg(target_os = "macos")]
    for folder in ["/opt/homebrew/bin", "/usr/local/bin"] {
        let p = Path::new(folder).join(name);
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from(name)
}
fn exif_data(path: &Path) -> Option<exif::Exif> {
    let file = File::open(path).ok()?;
    let mut bytes = vec![];
    file.take(32 * 1024 * 1024).read_to_end(&mut bytes).ok()?;
    exif::Reader::new()
        .read_from_container(&mut std::io::Cursor::new(bytes))
        .ok()
}
fn field(e: &exif::Exif, tag: exif::Tag) -> Option<String> {
    e.get_field(tag, exif::In::PRIMARY).map(|f| {
        if let exif::Value::Ascii(parts) = &f.value {
            return parts
                .first()
                .map(|p| {
                    String::from_utf8_lossy(p)
                        .trim_matches('\0')
                        .trim()
                        .to_string()
                })
                .unwrap_or_default();
        }
        f.display_value()
            .with_unit(e)
            .to_string()
            .trim_matches('"')
            .to_string()
    })
}
fn numeric(e: &exif::Exif, tag: exif::Tag) -> Option<u32> {
    e.get_field(tag, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
}
fn coordinate(e: &exif::Exif, tag: exif::Tag, reference: exif::Tag) -> Option<f64> {
    let f = e.get_field(tag, exif::In::PRIMARY)?;
    let reference = field(e, reference)?.to_ascii_uppercase();
    let negative = match (tag, reference.as_str()) {
        (exif::Tag::GPSLatitude, "N") | (exif::Tag::GPSLongitude, "E") => false,
        (exif::Tag::GPSLatitude, "S") | (exif::Tag::GPSLongitude, "W") => true,
        _ => return None,
    };
    if let exif::Value::Rational(a) = &f.value {
        if a.len() >= 3 {
            let n = a[0].to_f64() + a[1].to_f64() / 60. + a[2].to_f64() / 3600.;
            let limit = if tag == exif::Tag::GPSLatitude {
                90.
            } else {
                180.
            };
            if !n.is_finite() || n < 0. || n > limit {
                return None;
            }
            return Some(if negative { -n } else { n });
        }
    }
    None
}
pub struct Metadata {
    pub values: Value,
    pub state: &'static str,
}
// TIFF-based RAW files often place a tiny thumbnail in IFD0. Follow bounded
// SubIFD/Exif pointers to retain the largest recorded pixel area instead.
fn raw_tiff_dimensions(path: &Path) -> Option<(u32, u32)> {
    let mut bytes = vec![];
    File::open(path)
        .ok()?
        .take(32 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .ok()?;
    let little = match bytes.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let short = |i: usize| -> Option<u16> {
        let b = bytes.get(i..i.checked_add(2)?)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let long = |i: usize| -> Option<u32> {
        let b = bytes.get(i..i.checked_add(4)?)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    if short(2)? != 42 {
        return None;
    }
    let mut pending = vec![long(4)? as usize];
    let mut seen = std::collections::HashSet::new();
    let mut best = None;
    while let Some(offset) = pending.pop() {
        if offset == 0 || seen.len() >= 32 || !seen.insert(offset) {
            continue;
        }
        let Some(count) = short(offset).map(usize::from).filter(|n| *n <= 2048) else {
            continue;
        };
        let (mut width, mut height) = (None, None);
        for n in 0..count {
            let at = offset + 2 + n * 12;
            let (Some(tag), Some(format), Some(count)) = (short(at), short(at + 2), long(at + 4))
            else {
                break;
            };
            let scalar = match (format, count) {
                (3, 1) => short(at + 8).map(u32::from),
                (4, 1) => long(at + 8),
                _ => None,
            };
            match tag {
                0x100 | 0xa002 => {
                    if scalar.is_some() {
                        width = scalar;
                    }
                }
                0x101 | 0xa003 => {
                    if scalar.is_some() {
                        height = scalar;
                    }
                }
                0x14a | 0x8769 if format == 4 && (1..=16).contains(&count) => {
                    if count == 1 {
                        if let Some(v) = scalar {
                            pending.push(v as usize);
                        }
                    } else if let Some(base) = long(at + 8) {
                        for n in 0..count {
                            if let Some(v) = long(base as usize + n as usize * 4) {
                                pending.push(v as usize);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some((w, h)) = width
            .zip(height)
            .filter(|(w, h)| (1..=40000).contains(w) && (1..=40000).contains(h))
        {
            if best
                .is_none_or(|(bw, bh)| u64::from(w) * u64::from(h) > u64::from(bw) * u64::from(bh))
            {
                best = Some((w, h));
            }
        }
        if let Some(next) = long(offset + 2 + count * 12) {
            pending.push(next as usize);
        }
    }
    best
}
fn content_identifier(raw: &str) -> Option<String> {
    uuid::Uuid::parse_str(raw.trim_matches(|c: char| c == '\0' || c.is_ascii_whitespace()))
        .ok()
        .map(|v| v.to_string())
}
// Apple MakerNote: 10-byte signature, version, byte order, then an IFD at offset 14.
// Tag 0x0011 contains ContentIdentifier; value offsets are relative to this header.
fn apple_identifier(bytes: &[u8]) -> Option<String> {
    if !bytes.starts_with(b"Apple iOS\0") {
        return None;
    }
    let little = match bytes.get(12..14)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |i| -> Option<u16> {
        let b = bytes.get(i..i + 2)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u32_at = |i| -> Option<u32> {
        let b = bytes.get(i..i + 4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    let count = usize::from(u16_at(14)?);
    if count > 4096 {
        return None;
    }
    for n in 0..count {
        let entry = 16 + n * 12;
        if u16_at(entry)? != 0x11 {
            continue;
        }
        if u16_at(entry + 2)? != 2 {
            return None;
        }
        let len = u32_at(entry + 4)? as usize;
        if !(1..=256).contains(&len) {
            return None;
        }
        let offset = if len <= 4 {
            entry + 8
        } else {
            u32_at(entry + 8)? as usize
        };
        return content_identifier(
            std::str::from_utf8(bytes.get(offset..offset.checked_add(len)?)?).ok()?,
        );
    }
    None
}
fn probe_values(v: &mut Value, probe: &Value) -> bool {
    let stream = probe["streams"]
        .as_array()
        .and_then(|a| a.iter().find(|s| s["width"].is_number()))
        .cloned()
        .unwrap_or(Value::Null);
    v["width"] = stream["width"].clone();
    v["height"] = stream["height"].clone();
    let rotation = stream["side_data_list"]
        .as_array()
        .and_then(|list| list.iter().find_map(|s| s["rotation"].as_f64()))
        .or_else(|| {
            stream["tags"]["rotate"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
        })
        .unwrap_or(0.);
    let rotation = (rotation.round() as i64).rem_euclid(360);
    if rotation == 90 || rotation == 270 {
        v["width"] = stream["height"].clone();
        v["height"] = stream["width"].clone();
    }
    v["metadata"]["rotationDegrees"] = json!(rotation);
    v["codec"] = stream["codec_name"].clone();
    v["durationMs"] = json!(probe["format"]["duration"]
        .as_str()
        .or(stream["duration"].as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n >= 0.)
        .map(|n| (n * 1000.) as i64));
    if let Some(identifier) = probe["format"]["tags"]["com.apple.quicktime.content.identifier"]
        .as_str()
        .or(stream["tags"]["com.apple.quicktime.content.identifier"].as_str())
        .and_then(content_identifier)
    {
        v["metadata"]["contentIdentifier"] = json!(identifier);
    }
    if let Some(date) = probe["format"]["tags"]["creation_time"]
        .as_str()
        .or(stream["tags"]["creation_time"].as_str())
    {
        if let Ok(t) = chrono::DateTime::parse_from_rfc3339(date) {
            v["captureUtcMs"] = json!(t.timestamp_millis());
            v["captureLocal"] = json!(t.format("%Y-%m-%d %H:%M:%S").to_string());
            v["captureTzMin"] = json!(t.offset().local_minus_utc() / 60);
            v["captureTimeSource"] = json!("quicktime");
        }
    }
    !stream.is_null()
}
pub fn metadata(path: &Path, ext: &str, media_kind: &str, mtime: i64) -> Metadata {
    let mut v = json!({"captureUtcMs":mtime,"captureLocal":chrono::DateTime::from_timestamp_millis(mtime).map(|d|d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()),"captureTimeSource":"mtime","metadata":{"_version":METADATA_VERSION},"orientation":1});
    if media_kind == "video" {
        let mut cmd = Command::new(tool("ffprobe"));
        cmd.args(["-v","error","-show_entries","format=duration:format_tags=creation_time,com.apple.quicktime.content.identifier:stream=codec_name,width,height,duration:stream_tags=rotate,creation_time,com.apple.quicktime.content.identifier:stream_side_data=rotation","-of","json"]).arg(path);
        if let Ok(out) = run(cmd, 12) {
            if let Ok(probe) = serde_json::from_slice::<Value>(&out.stdout) {
                let available = probe_values(&mut v, &probe);
                return Metadata {
                    values: v,
                    state: if available {
                        "available"
                    } else {
                        "unsupported"
                    },
                };
            }
        }
        return Metadata {
            values: v,
            state: "unsupported",
        };
    }
    if let Some(e) = exif_data(path) {
        v["width"] = json!(numeric(&e, exif::Tag::PixelXDimension));
        v["height"] = json!(numeric(&e, exif::Tag::PixelYDimension));
        if let Some(exif::Field {
            value: exif::Value::Undefined(bytes, _),
            ..
        }) = e.get_field(exif::Tag::MakerNote, exif::In::PRIMARY)
        {
            if let Some(identifier) = apple_identifier(bytes) {
                v["metadata"]["contentIdentifier"] = json!(identifier);
            }
        }
        v["cameraMake"] = json!(field(&e, exif::Tag::Make));
        v["cameraModel"] = json!(field(&e, exif::Tag::Model));
        v["orientation"] = json!(numeric(&e, exif::Tag::Orientation).unwrap_or(1));
        v["gpsLat"] = json!(coordinate(
            &e,
            exif::Tag::GPSLatitude,
            exif::Tag::GPSLatitudeRef
        ));
        v["gpsLon"] = json!(coordinate(
            &e,
            exif::Tag::GPSLongitude,
            exif::Tag::GPSLongitudeRef
        ));
        for (key, tag) in [
            ("镜头", exif::Tag::LensModel),
            ("焦距", exif::Tag::FocalLength),
            ("光圈", exif::Tag::FNumber),
            ("快门", exif::Tag::ExposureTime),
            ("ISO", exif::Tag::PhotographicSensitivity),
        ] {
            if let Some(value) = field(&e, tag) {
                v["metadata"][key] = json!(value);
            }
        }
        if let Some(date) = field(&e, exif::Tag::DateTimeOriginal) {
            if let Ok(t) = chrono::NaiveDateTime::parse_from_str(&date, "%Y-%m-%d %H:%M:%S")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(&date, "%Y:%m:%d %H:%M:%S"))
            {
                v["captureLocal"] = json!(t.format("%Y-%m-%d %H:%M:%S").to_string());
                v["captureUtcMs"] = Value::Null;
                v["captureTimeSource"] = json!("exif");
                if let Some(offset) = field(&e, exif::Tag::OffsetTimeOriginal) {
                    let stamp = format!("{}{}", t.format("%Y-%m-%dT%H:%M:%S"), offset);
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&stamp) {
                        v["captureUtcMs"] = json!(dt.timestamp_millis());
                        v["captureTzMin"] = json!(dt.offset().local_minus_utc() / 60);
                    }
                }
            }
        }
    }
    let dims = if is_raw(ext) {
        raw_tiff_dimensions(path)
    } else {
        None
    }
    .or_else(|| {
        ImageReader::open(path)
            .ok()
            .and_then(|r| r.with_guessed_format().ok())
            .and_then(|r| r.into_dimensions().ok())
    })
    .or_else(|| {
        if !["heic", "heif"].contains(&ext) {
            return None;
        }
        let width = u32::try_from(v["width"].as_u64()?).ok()?;
        let height = u32::try_from(v["height"].as_u64()?).ok()?;
        (width > 0 && height > 0).then_some((width, height))
    });
    if let Some((w, h)) = dims {
        let orientation = v["orientation"].as_u64().unwrap_or(1);
        let (w, h) = if (5..=8).contains(&orientation) {
            (h, w)
        } else {
            (w, h)
        };
        v["width"] = json!(w);
        v["height"] = json!(h);
        Metadata {
            values: v,
            state: if is_raw(ext) || ["heic", "heif"].contains(&ext) {
                "unsupported"
            } else {
                "available"
            },
        }
    } else {
        Metadata {
            values: v,
            state: if is_raw(ext) || ["heic", "heif"].contains(&ext) {
                "unsupported"
            } else {
                "corrupt"
            },
        }
    }
}
fn embedded_jpeg(path: &Path) -> Result<DynamicImage> {
    let mut data = vec![];
    File::open(path)?
        .take(64 * 1024 * 1024)
        .read_to_end(&mut data)?;
    let mut candidates = vec![];
    let mut start = None;
    for i in 0..data.len().saturating_sub(1) {
        if data[i] == 0xff && data[i + 1] == 0xd8 {
            start = Some(i);
        }
        if data[i] == 0xff && data[i + 1] == 0xd9 {
            if let Some(s) = start.take() {
                if i - s > 4096 {
                    candidates.push((s, i + 2));
                }
            }
        }
    }
    candidates.sort_by_key(|(s, e)| std::cmp::Reverse(e - s));
    for (s, e) in candidates.into_iter().take(8) {
        let mut r =
            ImageReader::with_format(std::io::Cursor::new(&data[s..e]), image::ImageFormat::Jpeg);
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(256 * 1024 * 1024);
        limits.max_image_width = Some(40000);
        limits.max_image_height = Some(40000);
        r.limits(limits);
        if let Ok(img) = r.decode() {
            return Ok(img);
        }
    }
    Err(AppError::new(
        "ASSET_UNSUPPORTED",
        "未找到可用的 RAW 内嵌 JPEG",
    ))
}
pub fn decode(path: &Path, orientation: u32) -> Result<DynamicImage> {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let mut img = if is_raw(&ext) {
        embedded_jpeg(path)?
    } else {
        let mut reader = ImageReader::open(path)?.with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(256 * 1024 * 1024);
        limits.max_image_width = Some(40000);
        limits.max_image_height = Some(40000);
        reader.limits(limits);
        reader
            .decode()
            .map_err(|_| AppError::new("ASSET_UNSUPPORTED", "此格式无法解码或超过预览内存上限"))?
    };
    if let Some(o) = image::metadata::Orientation::from_exif(orientation as u8) {
        img.apply_orientation(o);
    }
    Ok(img)
}
pub fn thumbnail(cat: &Catalog, c: &mut Connection, aid: &str, variant: &str) -> Result<PathBuf> {
    thumbnail_with_priority(cat, c, aid, variant, false)
}
fn cached_thumbnail(
    cat: &Catalog,
    c: &Connection,
    aid: &str,
    variant: &str,
    size: i64,
    mtime: Option<i64>,
    format_version: i64,
) -> Result<Option<PathBuf>> {
    let mut cached:Option<String>=c.query_row("SELECT cache_rel_path FROM thumbnail_cache WHERE asset_id=? AND variant=? AND source_byte_size=? AND source_mtime_ms IS ? AND format_version=?",params![aid,variant,size,mtime,format_version],|r|r.get(0)).optional()?;
    if cached.is_none() && format_version == 2 && files::source_path(c, aid).is_err() {
        cached = c.query_row("SELECT cache_rel_path FROM thumbnail_cache WHERE asset_id=? AND variant=? AND source_byte_size=? AND source_mtime_ms IS ? AND format_version=1", params![aid,variant,size,mtime], |r| r.get(0)).optional()?;
    }
    if let Some(path) = cached
        .map(|path| cat.inner.cache.join(path))
        .filter(|path| path.is_file())
    {
        c.execute(
            "INSERT OR REPLACE INTO cache_access VALUES(?,?,?)",
            params![aid, variant, now()],
        )?;
        return Ok(Some(path));
    }
    Ok(None)
}
fn thumbnail_with_priority(
    cat: &Catalog,
    c: &mut Connection,
    aid: &str,
    variant: &str,
    foreground: bool,
) -> Result<PathBuf> {
    if !["small", "medium", "screen", "poster"].contains(&variant) {
        return Err(AppError::new("INVALID_FILTER", "预览尺寸无效"));
    }
    let row: (Option<i64>, i64, i64, String, String, Option<String>) = c.query_row(
        "SELECT mtime_ms,byte_size,COALESCE(orientation,1),media_kind,extension,file_version
         FROM media_assets WHERE id=?",
        [aid],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )?;
    let pixel_version = preview_format_version(&row.4);
    if let Some(path) = cached_thumbnail(cat, c, aid, variant, row.1, row.0, pixel_version)? {
        return Ok(path);
    }
    let _permit = DecodePermit::acquire(cat, foreground);
    // Another queued worker may have completed this preview while we waited.
    if let Some(path) = cached_thumbnail(cat, c, aid, variant, row.1, row.0, pixel_version)? {
        return Ok(path);
    }
    let path = files::source_path(c, aid)?;
    let before = files::FileVersion::read(&path)?;
    if !before.matches_index(row.1 as u64, row.0, row.5.as_deref()) {
        return Err(AppError::new(
            "DB_CONFLICT",
            "媒体内容已变化，请重新扫描来源",
        ));
    }
    let key = format!(
        "{:x}",
        Sha256::digest(format!(
            "{aid}-{variant}-{}-{}-v{pixel_version}-{}",
            row.1, row.2, before.token
        ))
    );
    let cache_name = format!("{key}.jpg");
    let target = cat.inner.cache.join(&cache_name);
    let temp = cat.inner.cache.join(format!("{}.part.jpg", id()));
    let max = match variant {
        "small" => 256,
        "medium" => 512,
        _ => 2560,
    };
    let result = (|| -> Result<()> {
        let mut dimensions = None;
        if row.3 == "video" {
            let mut cmd = Command::new(tool("ffmpeg"));
            cmd.args(["-v", "error", "-nostdin", "-ss", "0", "-i"])
                .arg(&path)
                .args([
                    "-frames:v",
                    "1",
                    "-vf",
                    &format!("scale={max}:{max}:force_original_aspect_ratio=decrease"),
                    "-y",
                ])
                .arg(&temp);
            if !run(cmd, 20)?.status.success() {
                return Err(AppError::new("ASSET_UNSUPPORTED", "无法提取视频海报帧"));
            }
        } else {
            let mut decoded = decode(&path, row.2 as u32);
            #[cfg(target_os = "macos")]
            if decoded.is_err() && ["heic", "heif"].contains(&row.4.as_str()) {
                let conversion = cat.inner.cache.join(format!("{}.conversion.jpg", id()));
                let mut cmd = Command::new("/usr/bin/sips");
                cmd.args(["-s", "format", "jpeg"])
                    .arg(&path)
                    .arg("--out")
                    .arg(&conversion);
                if run(cmd, 20).is_ok_and(|o| o.status.success()) {
                    let orientation = exif_data(&conversion)
                        .and_then(|e| numeric(&e, exif::Tag::Orientation))
                        .unwrap_or(1);
                    decoded = decode(&conversion, orientation);
                }
                let _ = fs::remove_file(conversion);
            }
            let img = decoded?;
            if ["heic", "heif"].contains(&row.4.as_str()) {
                dimensions = Some((img.width(), img.height()));
            }
            let thumb = img.thumbnail(max, max).to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(File::create(&temp)?, 86)
                .encode_image(&thumb)
                .map_err(|_| AppError::new("ASSET_CORRUPT", "缩略图生成失败"))?;
        }
        std::fs::OpenOptions::new()
            .write(true)
            .open(&temp)?
            .sync_all()?;
        // Publish the file and its database reference together with respect to
        // quota collection, including when an old key is regenerated.
        let _publish = cat.inner.cache_publish.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM media_assets WHERE id=? AND mtime_ms IS ?
             AND byte_size=? AND COALESCE(orientation,1)=? AND media_kind=? AND extension=?
             AND (file_version IS NULL OR file_version=?))",
            params![aid, row.0, row.1, row.2, row.3, row.4, before.token],
            |r| r.get(0),
        )?;
        if !current
            || files::source_path(&tx, aid)? != path
            || files::FileVersion::read(&path)? != before
        {
            return Err(AppError::new(
                "DB_CONFLICT",
                "媒体在生成预览时已变化，请重新扫描",
            ));
        }
        fs::rename(&temp, &target)?;
        tx.execute(
            "INSERT OR REPLACE INTO thumbnail_cache VALUES(?,?,?,?,?,?,?)",
            params![aid, variant, cache_name, row.0, row.1, pixel_version, now()],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO cache_access VALUES(?,?,?)",
            params![aid, variant, now()],
        )?;
        if let Some((width, height)) = dimensions {
            tx.execute(
                "UPDATE media_assets SET width=COALESCE(width,?),height=COALESCE(height,?) WHERE id=?",
                params![width, height, aid],
            )?;
        }
        if row.4 == "heic" || row.4 == "heif" || is_raw(&row.4) {
            tx.execute("UPDATE media_assets SET file_state='available' WHERE id=? AND file_state='unsupported'",[aid])?;
        }
        tx.commit()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result?;
    Ok(target)
}
pub fn resolve(cat: &Catalog, c: &mut Connection, r: &Value) -> Result<Value> {
    let aid = required(r, "assetId")?;
    let variant = required(r, "variant")?;
    let foreground = match r.get("priority") {
        None => true,
        Some(Value::String(value)) if value == "foreground" => true,
        Some(Value::String(value)) if value == "prefetch" => false,
        _ => return Err(AppError::new("INVALID_FILTER", "预览优先级无效")),
    };
    if foreground {
        cat.inner
            .interactive_until
            .store(now() + 200, std::sync::atomic::Ordering::Relaxed);
    }
    match thumbnail_with_priority(cat, c, aid, variant, foreground) {
        Ok(path) => {
            let token = id();
            let mut previews = cat.inner.previews.lock().unwrap();
            if previews.len() > 5000 {
                previews.clear();
            }
            previews.insert(token.clone(), (path, "image/jpeg".into()));
            Ok(
                json!({"assetId":aid,"variant":variant,"resourceUrl":format!("media://localhost/{token}"),"mimeType":"image/jpeg","ready":true}),
            )
        }
        Err(e) => Ok(
            json!({"assetId":aid,"variant":variant,"resourceUrl":null,"mimeType":null,"ready":false,"reason":e.message}),
        ),
    }
}
pub fn original(cat: &Catalog, c: &Connection, r: &Value) -> Result<Value> {
    let aid = required(r, "assetId")?;
    let path = files::source_path(c, aid)?;
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => {
            return Err(AppError::new(
                "ASSET_UNSUPPORTED",
                "此格式提供屏幕预览，请使用系统应用查看原文件",
            ))
        }
    };
    if !path.is_file() {
        return Err(AppError::new("ASSET_NOT_FOUND", "原文件缺失或无法读取"));
    }
    let original = File::open(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            AppError::new("PERMISSION_DENIED", "无法读取原文件，请重新授权来源目录")
        } else {
            AppError::new("ASSET_NOT_FOUND", "原文件缺失或无法读取")
        }
    })?;
    if original.metadata()?.len() > 64 * 1024 * 1024 {
        return Err(AppError::new(
            "ASSET_UNSUPPORTED",
            "原图超过 64 MB 安全预览上限，请使用系统应用查看",
        ));
    }
    let token = id();
    cat.inner
        .previews
        .lock()
        .unwrap()
        .insert(token.clone(), (path, mime.into()));
    Ok(json!({"resourceUrl":format!("media://localhost/{token}"),"mimeType":mime}))
}
pub fn cache_stats(cat: &Catalog, c: &Connection) -> Result<Value> {
    let mut bytes = 0u64;
    for entry in fs::read_dir(&cat.inner.cache)? {
        let e = entry?;
        if e.file_type()?.is_file() {
            bytes += e.metadata()?.len();
        }
    }
    let count: i64 = c.query_row("SELECT count(*) FROM thumbnail_cache", [], |r| r.get(0))?;
    Ok(json!({"bytes":bytes,"count":count}))
}
impl Catalog {
    pub fn clear_cache(&self, c: &mut Connection) -> Result<()> {
        let _publish = self.inner.cache_publish.lock().unwrap();
        let tx = c.transaction()?;
        tx.execute("DELETE FROM thumbnail_cache", [])?;
        tx.execute("DELETE FROM cache_access", [])?;
        tx.commit()?;
        self.inner.previews.lock().unwrap().clear();
        for e in fs::read_dir(&self.inner.cache)? {
            let e = e?;
            if e.file_type()?.is_file() {
                fs::remove_file(e.path())?;
            }
        }
        Ok(())
    }
    pub fn trim_cache(&self, c: &Connection) -> Result<()> {
        let _publish = self.inner.cache_publish.lock().unwrap();
        let settings: Value = db_settings(c);
        let max = (settings["cacheLimitGb"].as_f64().unwrap_or(5.) * 1024. * 1024. * 1024.) as u64;
        let stats = cache_stats(self, c)?;
        let mut bytes = stats["bytes"].as_u64().unwrap_or(0);
        if bytes <= max {
            return Ok(());
        }
        // Replaced decoder/content versions and interrupted publications can
        // leave files without a row. Reclaim these before evicting useful LRU
        // entries. Fresh temporary files may still belong to an active decoder.
        let cutoff = std::time::SystemTime::now() - Duration::from_secs(600);
        let mut referenced = std::collections::HashSet::new();
        let mut refs = c.prepare("SELECT cache_rel_path FROM thumbnail_cache")?;
        for row in refs.query_map([], |r| r.get::<_, String>(0))? {
            referenced.insert(row?);
        }
        for entry in fs::read_dir(&self.inner.cache)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let generated = name.strip_suffix(".jpg").is_some_and(|key| key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()))
                || [".part.jpg", ".conversion.jpg"].iter().any(|suffix| name.strip_suffix(suffix).is_some_and(|key| uuid::Uuid::parse_str(key).is_ok()));
            if !generated || referenced.contains(&name) || !entry.file_type()?.is_file() { continue; }
            let metadata = entry.metadata()?;
            if metadata.modified().is_ok_and(|time| time <= cutoff) && fs::remove_file(entry.path()).is_ok() {
                bytes = bytes.saturating_sub(metadata.len());
            }
        }
        let mut st=c.prepare("SELECT t.asset_id,t.variant,t.cache_rel_path FROM thumbnail_cache t LEFT JOIN cache_access ca ON ca.asset_id=t.asset_id AND ca.variant=t.variant ORDER BY COALESCE(ca.last_access,t.created_at)")?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (aid, v, p) in rows {
            if bytes <= max {
                break;
            }
            if Path::new(&p).file_name().and_then(|name| name.to_str()) != Some(p.as_str()) {
                continue;
            }
            let path = self.inner.cache.join(p);
            let metadata = fs::symlink_metadata(&path).ok();
            if metadata.as_ref().is_some_and(|m| !m.file_type().is_file()) { continue; }
            let size = metadata.map(|m| m.len()).unwrap_or(0);
            if fs::remove_file(&path).is_ok() || !path.exists() {
                c.execute(
                    "DELETE FROM thumbnail_cache WHERE asset_id=? AND variant=?",
                    params![&aid, &v],
                )?;
                c.execute("DELETE FROM cache_access WHERE asset_id=? AND variant=?", params![aid,v])?;
                bytes = bytes.saturating_sub(size);
            }
        }
        Ok(())
    }
}
#[derive(Default)]
pub struct DecodeState {
    active: usize,
    background: usize,
    waiting_foreground: usize,
}
pub(crate) fn foreground_busy(cat: &Catalog) -> bool {
    if cat
        .inner
        .interactive_until
        .load(std::sync::atomic::Ordering::Relaxed)
        > now()
    {
        return true;
    }
    let state = cat.inner.decoders.0.lock().unwrap();
    state.waiting_foreground > 0 || state.active > state.background
}
struct DecodePermit {
    cat: Catalog,
    foreground: bool,
}
impl DecodePermit {
    fn acquire(cat: &Catalog, foreground: bool) -> Self {
        let (lock, cond) = &cat.inner.decoders;
        let mut n = lock.lock().unwrap();
        if foreground {
            n.waiting_foreground += 1;
        }
        // One slot stays available for interactive previews; background work
        // cannot fill both slots with long RAW/video conversions.
        while n.active >= 2 || (!foreground && (n.background >= 1 || n.waiting_foreground > 0)) {
            n = cond.wait(n).unwrap();
        }
        if foreground {
            n.waiting_foreground -= 1;
        } else {
            n.background += 1;
        }
        n.active += 1;
        Self {
            cat: cat.clone(),
            foreground,
        }
    }
}
impl Drop for DecodePermit {
    fn drop(&mut self) {
        let (lock, cond) = &self.cat.inner.decoders;
        let mut n = lock.lock().unwrap();
        n.active -= 1;
        if !self.foreground {
            n.background -= 1;
        }
        cond.notify_all();
    }
}
fn db_settings(c: &Connection) -> Value {
    crate::db::setting(c, "preferences")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(json!({}))
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn bundled_probe_takes_precedence_over_machine_installations() {
        use std::os::unix::fs::PermissionsExt;
        const CHILD: &str = "TRAVEL_GALLERY_BUNDLED_PROBE_TEST";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            let result = metadata(&root.join("fixture.mov"), "mov", "video", 0);
            assert_eq!(result.state, "available");
            assert_eq!(result.values["width"], 613);
            assert_eq!(result.values["height"], 317);
            assert!(root.join("selected-bundled-probe").is_file());
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("probe-regression");
        fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        let probe = root.path().join("ffprobe");
        fs::write(&probe, "#!/bin/sh\nprintf selected > \"$TRAVEL_GALLERY_BUNDLED_PROBE_TEST/selected-bundled-probe\"\nprintf '%s' '{\"streams\":[{\"codec_name\":\"h264\",\"width\":613,\"height\":317}],\"format\":{\"duration\":\"1.25\"}}'\n").unwrap();
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o755)).unwrap();
        let result = Command::new(executable)
            .args(["--exact", "media::metadata_tests::bundled_probe_takes_precedence_over_machine_installations", "--nocapture"])
            .env(CHILD, root.path())
            .env("PATH", root.path().join("no-external-tools"))
            .output().unwrap();
        assert!(result.status.success(), "{}{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
    }
    #[test]
    fn interactive_decode_has_a_reserved_slot_and_precedes_queued_background_work() {
        let temp = tempfile::tempdir().unwrap();
        let cat = Catalog::new(&temp.path().join("data"), &temp.path().join("cache")).unwrap();
        let background = DecodePermit::acquire(&cat, false);
        let foreground = DecodePermit::acquire(&cat, true);
        let (order_tx, order_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let bg_cat = cat.clone();
        let bg_tx = order_tx.clone();
        let bg = thread::spawn(move || {
            let _permit = DecodePermit::acquire(&bg_cat, false);
            bg_tx.send("background").unwrap();
        });
        let fg_cat = cat.clone();
        let fg = thread::spawn(move || {
            let _permit = DecodePermit::acquire(&fg_cat, true);
            order_tx.send("foreground").unwrap();
            release_rx.recv().unwrap();
        });
        let start = Instant::now();
        while cat.inner.decoders.0.lock().unwrap().waiting_foreground == 0 {
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(1));
        }
        drop(background);
        assert_eq!(
            order_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "foreground"
        );
        release_tx.send(()).unwrap();
        assert_eq!(
            order_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "background"
        );
        drop(foreground);
        fg.join().unwrap();
        bg.join().unwrap();
        assert_eq!(cat.inner.decoders.0.lock().unwrap().active, 0);
    }
    #[test]
    fn apple_identifier_requires_bounded_valid_ifd_and_uuid() {
        let expected = "4a1c0798-8b79-4b47-8ad4-a392cb54b8c5";
        for little in [false, true] {
            let mut data = b"Apple iOS\0\0\x01MM".to_vec();
            if little {
                data[12..14].copy_from_slice(b"II");
            }
            let put16 = |n: u16| {
                if little {
                    n.to_le_bytes()
                } else {
                    n.to_be_bytes()
                }
            };
            let put32 = |n: u32| {
                if little {
                    n.to_le_bytes()
                } else {
                    n.to_be_bytes()
                }
            };
            data.extend(put16(1));
            data.extend(put16(0x11));
            data.extend(put16(2));
            data.extend(put32(37));
            data.extend(put32(32));
            data.extend([0; 4]);
            data.extend(expected.to_uppercase().as_bytes());
            data.push(0);
            assert_eq!(apple_identifier(&data).as_deref(), Some(expected));
            for len in 0..data.len() {
                assert!(apple_identifier(&data[..len]).is_none());
            }
            data[24..28].copy_from_slice(&put32(u32::MAX));
            assert!(apple_identifier(&data).is_none());
        }
        assert!(content_identifier("random-name").is_none());
    }
    #[test]
    fn gps_requires_valid_hemisphere_and_coordinate_bounds() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("gps.jpg");
        for (latitude, reference, expected) in [
            (40u32, b'N', Some(40.)),
            (40, b'S', Some(-40.)),
            (999, b'N', None),
            (40, b'X', None),
        ] {
            let mut data = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
            data.extend(1u16.to_le_bytes());
            data.extend(0x8825u16.to_le_bytes());
            data.extend(4u16.to_le_bytes());
            data.extend(1u32.to_le_bytes());
            data.extend(26u32.to_le_bytes());
            data.extend(0u32.to_le_bytes());
            data.extend(4u16.to_le_bytes());
            for (tag, format, count, value) in [
                (1u16, 2u16, 2u32, u32::from(reference)),
                (2, 5, 3, 80),
                (3, 2, 2, u32::from(b'E')),
                (4, 5, 3, 104),
            ] {
                data.extend(tag.to_le_bytes());
                data.extend(format.to_le_bytes());
                data.extend(count.to_le_bytes());
                data.extend(value.to_le_bytes());
            }
            data.extend(0u32.to_le_bytes());
            for coordinate in [latitude, 73] {
                for value in [coordinate, 0, 0] {
                    data.extend(value.to_le_bytes());
                    data.extend(1u32.to_le_bytes());
                }
            }
            let mut jpeg = std::io::Cursor::new(vec![]);
            image::DynamicImage::new_rgb8(16, 8)
                .write_to(&mut jpeg, image::ImageFormat::Jpeg)
                .unwrap();
            let jpeg = jpeg.into_inner();
            let mut output = jpeg[..2].to_vec();
            output.extend([0xff, 0xe1]);
            output.extend(((data.len() + 8) as u16).to_be_bytes());
            output.extend(b"Exif\0\0");
            output.extend(data);
            output.extend(&jpeg[2..]);
            std::fs::write(&path, output).unwrap();
            let v = metadata(&path, "jpg", "image", 0).values;
            assert_eq!(v["gpsLat"].as_f64(), expected);
            assert_eq!(v["gpsLon"].as_f64(), Some(73.));
        }
    }
    #[test]
    fn raw_dimensions_follow_sub_ifd_instead_of_thumbnail() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sample.nef");
        let mut bytes = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
        bytes.extend(3u16.to_le_bytes());
        for (tag, value) in [(0x100u16, 160u32), (0x101, 120), (0x14a, 50)] {
            bytes.extend(tag.to_le_bytes());
            bytes.extend(4u16.to_le_bytes());
            bytes.extend(1u32.to_le_bytes());
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        for (tag, value) in [(0x100u16, 6000u32), (0x101, 4000)] {
            bytes.extend(tag.to_le_bytes());
            bytes.extend(4u16.to_le_bytes());
            bytes.extend(1u32.to_le_bytes());
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(8u32.to_le_bytes()); // A cycle back to IFD0 must terminate.
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(raw_tiff_dimensions(&path), Some((6000, 4000)));
        bytes[42..46].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(raw_tiff_dimensions(&path), Some((160, 120)));
    }
    #[test]
    fn quicktime_rotation_identifier_and_offset_are_preserved() {
        let mut v = json!({"metadata":{}});
        let probe = json!({"streams":[{"width":1920,"height":1080,"codec_name":"h264","side_data_list":[{"rotation":-90}]}],"format":{"duration":"1.25","tags":{"creation_time":"2024-02-01T08:00:00+08:00","com.apple.quicktime.content.identifier":"4A1C0798-8B79-4B47-8AD4-A392CB54B8C5"}}});
        assert!(probe_values(&mut v, &probe));
        assert_eq!(v["width"], 1080);
        assert_eq!(v["height"], 1920);
        assert_eq!(v["durationMs"], 1250);
        assert_eq!(v["captureTzMin"], 480);
        assert_eq!(v["metadata"]["rotationDegrees"], 270);
        assert_eq!(
            v["metadata"]["contentIdentifier"],
            "4a1c0798-8b79-4b47-8ad4-a392cb54b8c5"
        );
    }
}
