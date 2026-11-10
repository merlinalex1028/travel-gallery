//! Strict local helper protocol; no database, network, source path or face IDs.
use crate::{
    error::{AppError, Result},
    eye_engine::Engine,
    model_resources,
};
use cap_std::{ambient_authority, fs::Dir};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, Read, Write},
    path::PathBuf,
    time::Instant,
};
#[derive(Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Request {
    Image { id: String, cache_file: String },
    Shutdown,
}
fn failure() -> AppError {
    AppError::new("QUALITY_INFERENCE_FAILED", "闭眼模型无法处理此项")
}
fn send(value: &Value) -> Result<()> {
    let mut o = io::stdout().lock();
    serde_json::to_writer(&mut o, value)?;
    o.write_all(b"\n")?;
    o.flush()?;
    Ok(())
}
pub fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 5 || args[1] != "--model-dir" || args[3] != "--cache-dir" {
        return Err(failure());
    }
    let start = Instant::now();
    let model = Engine::load(&PathBuf::from(&args[2]))?;
    let cache = Dir::open_ambient_dir(PathBuf::from(&args[4]), ambient_authority())?;
    send(
        &json!({"type":"ready","protocolVersion":1,"profileId":model_resources::eye_profile().id,"dimension":0,"preprocessVersion":model_resources::EYE_PREPROCESS_VERSION,"threadLimit":1,"loadMs":start.elapsed().as_secs_f64()*1000.}),
    )?;
    let mut input = io::stdin().lock();
    loop {
        let mut line = String::new();
        let n = (&mut input).take(8193).read_line(&mut line)?;
        if n == 0 {
            break;
        }
        if n > 8192 || !line.ends_with('\n') {
            return Err(failure());
        }
        let r: Request = serde_json::from_str(&line)?;
        match r {
            Request::Shutdown => break,
            Request::Image { id, cache_file } => {
                if id.is_empty() || id.len() > 128 {
                    return Err(failure());
                }
                let result = (|| -> Result<_> {
                    model.analyze(&crate::cache_pixels::read_image(&cache, &cache_file)?.to_rgb8())
                })();
                match result {
                    Ok(hints) => send(&json!({"type":"result","id":id,"hints":hints}))?,
                    Err(_) => send(
                        &json!({"type":"error","id":id,"code":"QUALITY_INFERENCE_FAILED","message":"闭眼模型无法处理此项，请检查预览或重新导入模型"}),
                    )?,
                }
            }
        }
    }
    Ok(())
}
