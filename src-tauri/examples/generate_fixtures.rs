//! Generates disposable media for manual UI QA. Never point this at personal media.
use std::{fs, path::PathBuf};
fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("pass a NEW fixture directory");
    assert!(!root.exists(), "fixture directory must not already exist");
    fs::create_dir_all(&root).unwrap();
    for n in 0..64u32 {
        let extension = match n % 5 {
            0 => "png",
            1 => "webp",
            2 => "tiff",
            _ => "jpg",
        };
        let img = image::RgbImage::from_fn(640, 420, |x, y| {
            let hill = 250. - ((x as f32 + n as f32 * 13.) / 75.).sin() * 55.;
            if (y as f32) < hill {
                image::Rgb([(130 + n % 50) as u8, (174 + n % 35) as u8, 190])
            } else {
                image::Rgb([
                    (64 + n % 35) as u8,
                    (105 + n % 30) as u8,
                    (84 + n % 30) as u8,
                ])
            }
        });
        let path = root.join(format!("旅途_{n:03}.{extension}"));
        img.save(path).unwrap();
    }
    fs::copy(root.join("旅途_003.jpg"), root.join("完全重复_改名.jpg")).unwrap();
    fs::write(root.join("损坏文件.jpg"), b"invalid-jpeg").unwrap();
    fs::write(root.join("零字节.jpg"), []).unwrap();
    let sub = root.join("子目录");
    fs::create_dir(&sub).unwrap();
    fs::copy(root.join("旅途_003.jpg"), sub.join("旅途_003.jpg")).unwrap();
    println!("{}", root.display());
}
