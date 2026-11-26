pub mod bursts;
pub mod capture_time;
pub mod catalog;
pub mod db;
#[cfg(feature = "desktop")]
pub mod desktop;
#[cfg(feature = "desktop")]
mod updater;
#[cfg(any(feature = "desktop", test))]
mod update_safety;
#[cfg(any(feature = "desktop", test))]
mod exit_guard;
#[cfg(feature = "performance-probe")]
pub mod performance_probe;
#[cfg(feature = "offline-network-probe")]
pub mod offline_probe;
pub mod error;
pub mod export_options;
pub mod export_errors;
pub mod files;
pub mod media;
pub mod media_response;
pub mod platform_power;
pub mod power;
pub mod publish;
pub mod query;
pub mod rename_preview;
pub mod review;
pub mod story;
pub mod tasks;
pub mod trails;
pub mod watcher;
pub mod xmp;

pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(feature = "ai-worker")]
pub mod ai_worker;
pub mod model_resources;

pub mod ai_runtime;
pub mod ai_vectors;

pub mod ai_models;
pub mod local_ai;

#[cfg(feature = "quality-worker")]
pub mod eye_engine;
pub mod quality_metrics;
pub mod quality_types;
#[cfg(feature = "quality-worker")]
pub mod quality_worker;
#[cfg(feature = "quality-worker")]
mod tflite_compat;

pub mod quality;

mod cache_pixels;

mod similarity_index;
