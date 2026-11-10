//! Local, bounded observer output for the separately identified benchmark app.
//! This module and its IPC command are absent from normal desktop builds.
use crate::error::{AppError, Result};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};
use tauri::Manager;

static PROCESS_START: OnceLock<Instant> = OnceLock::new();
static ACTIVE_PROBE: OnceLock<Probe> = OnceLock::new();
pub fn mark_process_start() {
    PROCESS_START.get_or_init(Instant::now);
}
fn elapsed_ms() -> f64 {
    PROCESS_START
        .get_or_init(Instant::now)
        .elapsed()
        .as_secs_f64()
        * 1000.
}
struct Output {
    file: File,
    bytes: usize,
    events: usize,
}
#[derive(Clone)]
pub struct Probe {
    output: Arc<Mutex<Output>>,
    run_id: String,
}
impl Probe {
    pub fn new(app: &tauri::AppHandle) -> Result<Self> {
        let expected = if cfg!(feature = "offline-network-probe") {
            "com.travelgallery.offlineaudit"
        } else {
            "com.travelgallery.benchmark"
        };
        if app.config().identifier != expected {
            return Err(AppError::new(
                "PERMISSION_DENIED",
                "性能采集仅用于独立基准应用",
            ));
        }
        let directory = app
            .path()
            .app_data_dir()
            .map_err(|_| AppError::new("INTERNAL", "无法定位基准目录"))?
            .join("performance");
        std::fs::create_dir_all(&directory)?;
        let run_id = format!("{}-{}", crate::now(), std::process::id());
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(format!("{run_id}.jsonl")))?;
        writeln!(
            file,
            "{}",
            json!({"type":"nativeStart","runId":run_id,
            "pid":std::process::id(),"atUtc":chrono::Utc::now().to_rfc3339(),
            "nativeElapsedMs":elapsed_ms(),"identifier":app.config().identifier,
            "scope":"observer only; no synthetic input or UI control"})
        )?;
        let probe = Self {
            output: Arc::new(Mutex::new(Output {
                file,
                bytes: 0,
                events: 0,
            })),
            run_id,
        };
        let _ = ACTIVE_PROBE.set(probe.clone());
        Ok(probe)
    }
    fn native_command(&self, name: &str, start: f64, duration: f64) {
        let row = json!({"type":"nativeCatalogCommand","runId":self.run_id,
            "command":name,"nativeStartedMs":start,"durationMs":duration});
        self.write_native(row);
    }
    fn write_native(&self, row: Value) {
        let Ok(mut line) = serde_json::to_vec(&row) else { return };
        line.push(b'\n');
        let Ok(mut output) = self.output.lock() else { return };
        if output.bytes + line.len() > 64 * 1024 * 1024 || output.events >= 200_000 { return; }
        if output.file.write_all(&line).and_then(|_|output.file.flush()).is_ok() {
            output.bytes += line.len();
            output.events += 1;
        }
    }
    fn record(&self, request: Value, received_ms: f64) -> Result<Value> {
        let events = request
            .get("events")
            .and_then(Value::as_array)
            .ok_or_else(|| AppError::new("VALIDATION_ERROR", "缺少性能观察数组"))?;
        if events.len() > 256 {
            return Err(AppError::new("VALIDATION_ERROR", "性能观察批次过大"));
        }
        let mut lines = Vec::new();
        for event in events {
            serde_json::to_writer(
                &mut lines,
                &json!({"type":"frontendBatchEvent",
                "runId":self.run_id,"nativeReceivedMs":received_ms,"event":event}),
            )
            .map_err(|_| AppError::new("VALIDATION_ERROR", "性能观察格式错误"))?;
            lines.push(b'\n');
        }
        if lines.len() > 256 * 1024 {
            return Err(AppError::new("VALIDATION_ERROR", "性能观察内容过大"));
        }
        let mut output = self
            .output
            .lock()
            .map_err(|_| AppError::new("INTERNAL", "性能记录锁异常"))?;
        if output.bytes + lines.len() > 64 * 1024 * 1024 || output.events + events.len() > 200_000 {
            return Err(AppError::new("VALIDATION_ERROR", "本次性能记录已达到上限"));
        }
        output.file.write_all(&lines)?;
        output.file.flush()?;
        output.bytes += lines.len();
        output.events += events.len();
        Ok(json!({"runId":self.run_id,"nativeReceivedMs":received_ms,
            "nativeRespondedMs":elapsed_ms(),"eventsWritten":output.events}))
    }
}
pub(crate) fn record_lifecycle(stage: &str, details: Value) {
    if let Some(probe) = ACTIVE_PROBE.get() {
        probe.write_native(json!({"type":"nativeLifecycleEvent","runId":probe.run_id,
            "pid":std::process::id(),"nativeElapsedMs":elapsed_ms(),"stage":stage,"details":details}));
    }
}
pub struct CommandTimer<'a> {
    name: &'a str,
    started: Instant,
    native_started: f64,
}
impl<'a> CommandTimer<'a> {
    pub fn start(name: &'a str) -> Option<Self> {
        // Startup diagnosis only; do not add per-command logging to a long
        // grid/keyboard run, and never record request or result contents.
        (elapsed_ms() < 10_000.).then(|| Self { name, started: Instant::now(), native_started: elapsed_ms() })
    }
}
impl Drop for CommandTimer<'_> {
    fn drop(&mut self) {
        if let Some(probe) = ACTIVE_PROBE.get() {
            probe.native_command(self.name, self.native_started, self.started.elapsed().as_secs_f64()*1000.);
        }
    }
}
#[tauri::command]
pub async fn performance_probe(state: tauri::State<'_, Probe>, request: Value) -> Result<Value> {
    let received_ms = elapsed_ms();
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.record(request, received_ms))
        .await
        .map_err(|_| AppError::new("INTERNAL", "性能记录任务异常"))?
}
