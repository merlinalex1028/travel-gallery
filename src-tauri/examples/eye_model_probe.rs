#[cfg(feature = "quality-worker")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use image::ImageDecoder;
    use serde_json::json;
    use std::{path::PathBuf, time::Instant};
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() < 2 {
        return Err("expected model dir and public sample paths".into());
    }
    let start = Instant::now();
    let model = travel_gallery::eye_engine::Engine::load(&PathBuf::from(&args[0]))?;
    eprintln!("Models loaded");
    let load = start.elapsed().as_secs_f64() * 1000.;
    let mut rows = vec![];
    for path in &args[1..] {
        eprintln!("Sample {:?}", path);
        let start = Instant::now();
        let path = PathBuf::from(path);
        let mut decoder = image::ImageReader::open(&path)?
            .with_guessed_format()?
            .into_decoder()?;
        let orientation = decoder.orientation()?;
        let mut image = image::DynamicImage::from_decoder(decoder)?;
        image.apply_orientation(orientation);
        let rgb = image.to_rgb8();
        let (faces, limit) = model.diagnostics(&rgb)?;
        let hints = model.analyze(&rgb)?;
        rows.push(json!({"file":path.file_name().unwrap().to_string_lossy(),"width":rgb.width(),"height":rgb.height(),"faces":faces.iter().map(|f|json!({"landmarks":f.landmarks,"blendshapes":f.blendshapes,"bounds":f.bounds})).collect::<Vec<_>>(),"faceLimitReached":limit,"hints":hints,"elapsedMs":start.elapsed().as_secs_f64()*1000.}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"profileId":travel_gallery::model_resources::eye_profile().id,"preprocessVersion":travel_gallery::model_resources::EYE_PREPROCESS_VERSION,"loadMs":load,"samples":rows})
        )?
    );
    Ok(())
}
#[cfg(not(feature = "quality-worker"))]
fn main() {
    eprintln!("explicit quality-worker feature required");
    std::process::exit(1);
}
