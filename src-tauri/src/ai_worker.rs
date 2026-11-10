//! Isolated CPU inference. The main app sends only a validated preview cache
//! name or short query. This process has no HTTP client, database, or write API.
use crate::{
    error::{AppError, Result},
    model_resources::{self, profile},
};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::siglip;
use cap_std::{ambient_authority, fs::Dir};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use tokenizers::Tokenizer;

#[derive(Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Request {
    Image { id: String, cache_file: String },
    Text { id: String, text: String },
    Shutdown,
}
fn failure() -> AppError {
    AppError::new(
        "AI_INFERENCE_FAILED",
        "本地模型无法处理此项，请重试或重新导入模型",
    )
}
fn normalized(features: Tensor) -> Result<Vec<f32>> {
    let mut vector = features
        .flatten_all()
        .map_err(|_| failure())?
        .to_vec1::<f32>()
        .map_err(|_| failure())?;
    if vector.len() != profile().embedding_dimension || vector.iter().any(|n| !n.is_finite()) {
        return Err(failure());
    }
    let norm = vector
        .iter()
        .map(|v| f64::from(*v).powi(2))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm < 1e-8 {
        return Err(failure());
    }
    vector.iter_mut().for_each(|n| *n /= norm as f32);
    Ok(vector)
}
struct Encoder {
    model: siglip::Model,
    tokenizer: Tokenizer,
    cache: Dir,
    pad: u32,
    eos: u32,
}
impl Encoder {
    fn load(model_dir: &Path, cache_dir: &Path) -> Result<Self> {
        let dir = Dir::open_ambient_dir(model_dir, ambient_authority())?;
        model_resources::validate_files(&dir, || Ok(()))?;
        let config: siglip::Config = serde_json::from_slice(&dir.read("config.json")?)?;
        if config.vision_config.image_size != 224
            || config.text_config.max_position_embeddings != 64
            || config.text_config.hidden_size != 768
            || config.text_config.vocab_size != 256_000
        {
            return Err(failure());
        }
        let mut tokenizer =
            Tokenizer::from_file(model_dir.join("tokenizer.json")).map_err(|_| failure())?;
        tokenizer.with_padding(None);
        let pad = tokenizer.token_to_id("<pad>").ok_or_else(failure)?;
        let eos = tokenizer.token_to_id("<eos>").ok_or_else(failure)?;
        if pad != 0 || eos != 1 {
            return Err(failure());
        }
        let weights = model_dir.join("model.safetensors");
        // Only this short-lived worker maps the pinned, hash-verified data file.
        // No pickle, dynamic model code or arbitrary architecture is accepted.
        let vb =
            unsafe { VarBuilder::from_mmaped_safetensors(&[weights], DType::F32, &Device::Cpu) }
                .map_err(|_| failure())?;
        let model = siglip::Model::new(&config, vb).map_err(|_| failure())?;
        Ok(Self {
            model,
            tokenizer,
            cache: Dir::open_ambient_dir(cache_dir, ambient_authority())?,
            pad,
            eos,
        })
    }
    fn text(&self, text: &str) -> Result<Vec<f32>> {
        if text.trim().is_empty() || text.chars().count() > 200 || text.len() > 2048 {
            return Err(AppError::new(
                "INVALID_FILTER",
                "请输入不超过 200 字的内容描述",
            ));
        }
        // The author-provided tokenizer configuration enables lowercase text,
        // EOS, right padding, and no BOS. Padding is Gemma's <pad>=0, not the
        // original SigLIP v1 configuration's default pad=1.
        let normalized_text = text.trim().to_lowercase();
        let encoded = self
            .tokenizer
            .encode(normalized_text, true)
            .map_err(|_| failure())?;
        let mut ids = encoded.get_ids().to_vec();
        if ids.last() != Some(&self.eos) {
            ids.push(self.eos);
        }
        if ids.len() > profile().max_text_tokens {
            return Err(AppError::new(
                "INVALID_FILTER",
                "描述超过模型可处理的长度，请缩短后重试",
            ));
        }
        ids.resize(profile().max_text_tokens, self.pad);
        let input = Tensor::from_vec(ids, (1, profile().max_text_tokens), &Device::Cpu)
            .map_err(|_| failure())?;
        normalized(
            self.model
                .get_text_features(&input)
                .map_err(|_| failure())?,
        )
    }
    fn image(&self, cache_file: &str) -> Result<Vec<f32>> {
        // The author configuration uses PIL BILINEAR, direct 224x224 resize,
        // RGB, mean=.5 and std=.5, without a center crop.
        let pixels = crate::cache_pixels::read_image(&self.cache, cache_file)?
            .resize_exact(224, 224, image::imageops::FilterType::Triangle)
            .to_rgb8()
            .into_raw();
        let input = Tensor::from_vec(pixels, (224, 224, 3), &Device::Cpu)
            .map_err(|_| failure())?
            .permute((2, 0, 1))
            .map_err(|_| failure())?
            .to_dtype(DType::F32)
            .map_err(|_| failure())?
            .affine(2. / 255., -1.)
            .map_err(|_| failure())?
            .unsqueeze(0)
            .map_err(|_| failure())?;
        normalized(
            self.model
                .get_image_features(&input)
                .map_err(|_| failure())?,
        )
    }
}
fn send(value: &Value) -> Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}
pub fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 5 || args[1] != "--model-dir" || args[3] != "--cache-dir" {
        return Err(failure());
    }
    let start = Instant::now();
    let encoder = Encoder::load(&PathBuf::from(&args[2]), &PathBuf::from(&args[4]))?;
    send(
        &json!({"type":"ready","protocolVersion":1,"profileId":profile().id,"dimension":profile().embedding_dimension,"preprocessVersion":model_resources::PREPROCESS_VERSION,"threadLimit":2,"loadMs":start.elapsed().as_secs_f64()*1000.}),
    )?;
    let mut input = io::stdin().lock();
    loop {
        let mut line = String::new();
        let n = (&mut input).take(8193).read_line(&mut line)?;
        if n == 0 {
            return Ok(());
        }
        if n > 8192 || !line.ends_with('\n') {
            return Err(failure());
        }
        let request: Request = serde_json::from_str(&line)?;
        let start = Instant::now();
        let (id, result) = match request {
            Request::Image { id, cache_file } => (id, encoder.image(&cache_file)),
            Request::Text { id, text } => (id, encoder.text(&text)),
            Request::Shutdown => return Ok(()),
        };
        if id.is_empty() || id.len() > 128 {
            return Err(failure());
        }
        match result {
            Ok(vector) => send(
                &json!({"type":"result","id":id,"vector":vector,"elapsedMs":start.elapsed().as_secs_f64()*1000.}),
            )?,
            Err(error) => {
                send(&json!({"type":"error","id":id,"code":error.code,"message":error.message}))?
            }
        }
    }
}
