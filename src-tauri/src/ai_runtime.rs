//! One reusable, isolated inference worker. Frames, time, CPU, and lifetime are
//! bounded. Dropping or cancelling the runtime kills and reaps only our child.
use crate::{
    catalog::Catalog,
    error::{AppError, Result},
    id,
    model_resources::{self, profile},
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
        Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct Runtime {
    process: Mutex<Option<Process>>,
    generation: AtomicU64,
    phase: AtomicU8,
    eyes: bool,
}
struct Process {
    child: Child,
    input: ChildStdin,
    output: Receiver<std::result::Result<Value, ()>>,
    last_used: Instant,
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn error(message: &str) -> AppError {
    AppError::new("AI_RUNTIME_FAILED", message)
}
fn program(eyes: bool) -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    let dir = executable
        .parent()
        .ok_or_else(|| error("本地推理程序路径无效"))?;
    let name = if eyes {
        if cfg!(windows) {
            "travel-gallery-quality.exe"
        } else {
            "travel-gallery-quality"
        }
    } else if cfg!(windows) {
        "travel-gallery-ai.exe"
    } else {
        "travel-gallery-ai"
    };
    let mut path = dir.join(name);
    // Cargo test/example executables live one level below their companion bin.
    // Packaged desktop builds only use the signed sibling executable.
    if !cfg!(feature = "desktop")
        && !path.is_file()
        && dir
            .file_name()
            .is_some_and(|n| n == "deps" || n == "examples")
    {
        path = dir
            .parent()
            .ok_or_else(|| error("本地推理程序缺失"))?
            .join(name);
    }
    if !path.is_file() {
        return Err(error("本地推理程序缺失，请重新安装完整应用"));
    }
    Ok(path)
}
impl Process {
    fn spawn(cat: &Catalog, eyes: bool) -> Result<Self> {
        let mut command = Command::new(program(eyes)?);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .args(["--model-dir"])
            .arg(if eyes {
                model_resources::eye_installed_path(&cat.inner.data)
            } else {
                model_resources::installed_path(&cat.inner.data)
            })
            .args(["--cache-dir"])
            .arg(&cat.inner.cache)
            .env("RAYON_NUM_THREADS", if eyes { "1" } else { "2" })
            .env("CANDLE_NUM_THREADS", if eyes { "1" } else { "2" })
            .env("TOKENIZERS_PARALLELISM", "false")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| error("无法启动本地推理程序，请检查应用安装"))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| error("本地推理进程无法通信"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| error("本地推理进程无法通信"))?;
        let (send, output) = mpsc::sync_channel(4);
        thread::Builder::new()
            .name("local-ai-output".into())
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let mut line = String::new();
                    let frame = match (&mut reader).take(65_537).read_line(&mut line) {
                        Ok(n) if n > 0 && n <= 65_536 && line.ends_with('\n') => {
                            serde_json::from_str(&line).map_err(|_| ())
                        }
                        _ => Err(()),
                    };
                    let terminal = frame.is_err();
                    if send.send(frame).is_err() || terminal {
                        break;
                    }
                }
            })
            .map_err(|_| {
                let _ = child.kill();
                let _ = child.wait();
                error("无法启动本地推理通信")
            })?;
        Ok(Self {
            child,
            input,
            output,
            last_used: Instant::now(),
        })
    }
    fn receive(
        &mut self,
        deadline: Instant,
        mut control: impl FnMut() -> Result<()>,
    ) -> Result<Value> {
        loop {
            control()?;
            if Instant::now() > deadline {
                return Err(error("本地推理超时，进程已释放；可缩小范围后重试"));
            }
            match self.output.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(value)) => return Ok(value),
                Ok(Err(())) | Err(RecvTimeoutError::Disconnected) => {
                    return Err(error("本地推理进程已退出或响应无效，请重试"))
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
    fn send(&mut self, request: &Value) -> Result<()> {
        serde_json::to_writer(&mut self.input, request).map_err(|_| error("本地推理通信失败"))?;
        self.input
            .write_all(b"\n")
            .map_err(|_| error("本地推理通信失败"))?;
        self.input.flush().map_err(|_| error("本地推理通信失败"))?;
        Ok(())
    }
}
impl Runtime {
    pub fn eyes() -> Self {
        Self {
            eyes: true,
            ..Self::default()
        }
    }
    pub fn phase(&self) -> &'static str {
        match self.phase.load(Ordering::SeqCst) {
            1 => "loading",
            2 => "loaded",
            _ => "idle",
        }
    }
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut slot) = self.process.try_lock() {
            *slot = None;
            self.phase.store(0, Ordering::SeqCst);
        }
    }
    pub fn reap_idle(&self) {
        if let Ok(mut slot) = self.process.try_lock() {
            if slot.as_mut().is_some_and(|p| {
                p.last_used.elapsed() > Duration::from_secs(60)
                    || p.child.try_wait().ok().flatten().is_some()
            }) {
                *slot = None;
                self.phase.store(0, Ordering::SeqCst);
            }
        }
    }
    pub fn request(
        &self,
        cat: &Catalog,
        request: Value,
        control: impl FnMut(bool) -> Result<()>,
    ) -> Result<Vec<f32>> {
        self.decoded(cat, request, control, |response| {
            let vector: Vec<f32> = serde_json::from_value(response["vector"].clone())
                .map_err(|_| error("本地推理向量无效"))?;
            if vector.len() != profile().embedding_dimension
                || vector.iter().any(|v| !v.is_finite() || v.abs() > 1.0001)
            {
                return Err(error("本地推理向量无效"));
            }
            let norm = vector.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
            if (norm - 1.).abs() > 0.01 {
                return Err(error("本地推理向量未正确归一化"));
            }
            Ok(vector)
        })
    }
    pub fn eye_request(
        &self,
        cat: &Catalog,
        request: Value,
        control: impl FnMut(bool) -> Result<()>,
    ) -> Result<crate::quality_types::EyeHints> {
        self.decoded(cat, request, control, |response| {
            let hints: crate::quality_types::EyeHints =
                serde_json::from_value(response["hints"].clone())
                    .map_err(|_| error("闭眼提示响应无效"))?;
            if !hints.validate() {
                return Err(error("闭眼提示响应无效"));
            }
            Ok(hints)
        })
    }
    fn decoded<T>(
        &self,
        cat: &Catalog,
        mut request: Value,
        mut control: impl FnMut(bool) -> Result<()>,
        decode: impl FnOnce(Value) -> Result<T>,
    ) -> Result<T> {
        let generation = self.generation.load(Ordering::SeqCst);
        let check = || {
            if self.generation.load(Ordering::SeqCst) != generation {
                Err(AppError::new("CANCELLED", "本地 AI 已关闭或任务已取消"))
            } else {
                Ok(())
            }
        };
        let lock_deadline = Instant::now() + Duration::from_secs(90);
        let mut slot = loop {
            check()?;
            control(true)?;
            if let Ok(slot) = self.process.try_lock() {
                break slot;
            }
            if Instant::now() > lock_deadline {
                return Err(error("本地 AI 正在处理其他请求，请稍后重试"));
            }
            thread::sleep(Duration::from_millis(25));
        };
        let result = (|| -> Result<std::result::Result<T, AppError>> {
            if slot.is_none() {
                self.phase.store(1, Ordering::SeqCst);
                let mut process = Process::spawn(cat, self.eyes)?;
                let ready = process.receive(Instant::now() + Duration::from_secs(90), || {
                    check()?;
                    control(false)
                })?;
                let p = if self.eyes {
                    model_resources::eye_profile()
                } else {
                    profile()
                };
                let preprocessor = if self.eyes {
                    model_resources::EYE_PREPROCESS_VERSION
                } else {
                    model_resources::PREPROCESS_VERSION
                };
                if ready["type"] != "ready"
                    || ready["protocolVersion"] != 1
                    || ready["profileId"] != p.id
                    || ready["dimension"] != p.embedding_dimension
                    || ready["preprocessVersion"] != preprocessor
                    || ready["threadLimit"] != if self.eyes { 1 } else { 2 }
                {
                    return Err(error("本地推理程序与模型版本不匹配，请重新安装模型包"));
                }
                *slot = Some(process);
                self.phase.store(2, Ordering::SeqCst);
            }
            check()?;
            control(false)?;
            let id = id();
            request["id"] = json!(id);
            let process = slot.as_mut().unwrap();
            process.send(&request)?;
            let response = process.receive(Instant::now() + Duration::from_secs(30), || {
                check()?;
                control(false)
            })?;
            process.last_used = Instant::now();
            if response["id"] != id {
                return Err(error("本地推理响应序号无效"));
            }
            if response["type"] == "error" {
                let code = if response["code"] == "INVALID_FILTER" {
                    "INVALID_FILTER"
                } else {
                    "AI_INFERENCE_FAILED"
                };
                return Ok(Err(AppError::new(
                    code,
                    response["message"]
                        .as_str()
                        .unwrap_or("本地模型无法处理此项"),
                )));
            }
            if response["type"] != "result" {
                return Err(error("本地推理响应类型无效"));
            }
            Ok(Ok(decode(response)?))
        })();
        match result {
            Ok(inner) => inner,
            Err(error) => {
                *slot = None;
                self.phase.store(0, Ordering::SeqCst);
                Err(error)
            }
        }
    }
}
pub fn watch_idle(cat: &Catalog) {
    let weak = std::sync::Arc::downgrade(&cat.inner);
    let _ = thread::Builder::new()
        .name("local-ai-idle".into())
        .stack_size(64 * 1024)
        .spawn(move || loop {
            thread::sleep(Duration::from_secs(5));
            let Some(inner) = weak.upgrade() else {
                break;
            };
            inner.ai.reap_idle();
            inner.eyes.reap_idle();
        });
}
