// Standalone, data-only inference process. It does not start Tauri or a catalog.
fn main() {
    std::env::set_var("RAYON_NUM_THREADS", "2");
    std::env::set_var("CANDLE_NUM_THREADS", "2");
    std::env::set_var("TOKENIZERS_PARALLELISM", "false");
    if travel_gallery::ai_worker::run().is_err() {
        // Do not write model paths, image names or private queries into logs.
        println!("{{\"type\":\"error\",\"code\":\"AI_INFERENCE_FAILED\",\"message\":\"本地模型无法启动或处理请求\"}}");
        std::process::exit(1);
    }
}
