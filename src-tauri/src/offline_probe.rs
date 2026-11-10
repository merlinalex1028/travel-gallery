//! Active loopback control for the separately signed App Sandbox QA build.
//! This module is absent from normal desktop and performance-only builds.
use crate::error::{AppError, Result};
use serde_json::json;
use std::{
    io::ErrorKind,
    net::{SocketAddr, TcpStream},
    time::Duration,
};
use tauri::Manager;

pub fn verify(app: &tauri::AppHandle) -> Result<()> {
    if !cfg!(target_os = "macos") || app.config().identifier != "com.travelgallery.offlineaudit" {
        return Err(AppError::new(
            "PERMISSION_DENIED",
            "离线控制仅用于独立macOS验收应用",
        ));
    }
    let endpoint = SocketAddr::from(([127, 0, 0, 1], 9));
    let result = TcpStream::connect_timeout(&endpoint, Duration::from_secs(1));
    let denied = result
        .as_ref()
        .err()
        .is_some_and(|error| error.kind() == ErrorKind::PermissionDenied);
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|_| AppError::new("INTERNAL", "无法定位离线验收目录"))?;
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|_| AppError::new("INTERNAL", "无法定位离线验收缓存"))?;
    std::fs::create_dir_all(&directory)?;
    let report = json!({"atUtc":chrono::Utc::now().to_rfc3339(),"pid":std::process::id(),
        "identifier":app.config().identifier,"appDataDirectory":directory,"appCacheDirectory":cache,
        "controlEndpoint":"127.0.0.1:9","connectionDenied":denied,
        "rawOsError":result.as_ref().err().and_then(|error|error.raw_os_error()),
        "error":result.as_ref().err().map(ToString::to_string),
        "scope":"Active local control in the actual QA GUI process; not proof about WebKit XPC or all OS requests."});
    std::fs::write(
        directory.join("offline-process-control.json"),
        serde_json::to_vec_pretty(&report)
            .map_err(|_| AppError::new("INTERNAL", "离线控制报告编码失败"))?,
    )?;
    if !denied {
        return Err(AppError::new(
            "PERMISSION_DENIED",
            "离线验收进程未确认网络隔离，停止启动",
        ));
    }
    Ok(())
}
