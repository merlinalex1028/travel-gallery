//! Only native commands can choose the release endpoint and hold an Update handle.
//! No JS updater/process permission or arbitrary URLs/installer paths are exposed.
use crate::{
    catalog::Catalog,
    error::{AppError, Result},
    update_safety::{handoff_for_restart, with_idle_catalog},
};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::{ipc::Channel, AppHandle, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub struct UpdateState {
    busy: AtomicBool,
    pub installing: AtomicBool,
    pub restarting: AtomicBool,
    candidate: Mutex<Option<Update>>,
    package: Mutex<Option<Vec<u8>>>,
}
struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
impl UpdateState {
    fn acquire(&self) -> Result<Busy<'_>> {
        self.busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| AppError::new("UPDATE_BUSY", "更新操作进行中，请稍候"))?;
        Ok(Busy(&self.busy))
    }
}
fn support(app: &AppHandle) -> bool {
    !cfg!(debug_assertions)
        && app.config().identifier == "com.travelgallery.desktop"
        && (cfg!(all(target_os = "macos", target_arch = "aarch64"))
            || cfg!(all(target_os = "windows", target_arch = "x86_64")))
}
fn require_support(app: &AppHandle) -> Result<()> {
    if !support(app) {
        return Err(AppError::new("UPDATE_UNAVAILABLE", "仅支持已安装的 macOS Apple Silicon / Windows x64 正式发行应用；开发、验收与浏览器版本不检查或安装更新"));
    }
    Ok(())
}
#[tauri::command]
pub fn update_support(app: AppHandle) -> Value {
    json!({"supported":support(&app),"version":app.package_info().version.to_string()})
}
#[tauri::command]
pub async fn check_app_update(app: AppHandle, state: State<'_, UpdateState>) -> Result<Value> {
    require_support(&app)?;
    let _busy = state.acquire()?;
    // Retain a successfully staged package until the user installs it or quits.
    if state.package.lock().unwrap().is_some() {
        return Err(AppError::new(
            "UPDATE_BUSY",
            "更新已下载，请安装或退出应用后重新检查",
        ));
    }
    let mut update = app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| AppError::new("UPDATE_CONFIG", "更新配置无效，请从官网下载新版"))?
        .check()
        .await
        .map_err(|_| {
            AppError::new(
                "UPDATE_CHECK_FAILED",
                "无法检查更新：请检查网络或稍后重试；尚未发布正式更新清单时也会出现此提示",
            )
        })?;
    if let Some(update) = &mut update {
        update.timeout = Some(Duration::from_secs(15 * 60));
        // The plugin follows HTTPS redirects; the final bytes must also pass the
        // embedded public-key signature check before installation.
        if !trusted_download(update.download_url.as_str()) {
            return Err(AppError::new(
                "UPDATE_SOURCE",
                "更新下载地址不属于旅图的 GitHub Releases，已阻止下载",
            ));
        }
    }
    let result = update
        .as_ref()
        .map(|u| json!({"version":u.version,"notes":u.body}));
    *state.candidate.lock().unwrap() = update;
    Ok(json!(result))
}
fn trusted_download(url: &str) -> bool {
    url.starts_with("https://github.com/merlinalex1028/travel-gallery/releases/download/v")
        && !url.contains(['?', '#', '\\'])
        && !url.contains("/../")
}
#[tauri::command]
pub async fn download_app_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
    progress: Channel<Value>,
) -> Result<()> {
    require_support(&app)?;
    let _busy = state.acquire()?;
    if state.package.lock().unwrap().is_some() {
        return Ok(());
    }
    let update = state
        .candidate
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| AppError::new("UPDATE_MISSING", "请先检查新版本"))?;
    let mut downloaded = 0u64;
    let bytes = update
        .download(
            |chunk, total| {
                downloaded = downloaded.saturating_add(chunk as u64);
                let _ = progress.send(json!({"downloaded":downloaded,"total":total}));
            },
            || {},
        )
        .await
        .map_err(|_| {
            AppError::new(
                "UPDATE_DOWNLOAD_FAILED",
                "更新下载或签名校验失败，未修改应用；请重试下载",
            )
        })?;
    *state.package.lock().unwrap() = Some(bytes);
    Ok(())
}
#[tauri::command]
pub async fn install_app_update(
    app: AppHandle,
    catalog: State<'_, Catalog>,
    state: State<'_, UpdateState>,
) -> Result<()> {
    require_support(&app)?;
    let _busy = state.acquire()?;
    let cat = catalog.inner().clone();
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<UpdateState>();
        let update = state.candidate.lock().unwrap().clone()
            .ok_or_else(|| AppError::new("UPDATE_MISSING", "请先检查新版本"))?;
        let bytes = state.package.lock().unwrap().take()
            .ok_or_else(|| AppError::new("UPDATE_MISSING", "请先下载更新"))?;
        state.installing.store(true, Ordering::SeqCst);
        let result = with_idle_catalog(&cat, || {
            update.install(&bytes).map_err(|_| AppError::new("UPDATE_INSTALL_FAILED", "无法安装更新，请确认应用已移至可写的安装目录，然后重试；也可从官网下载新版覆盖安装"))?;
            // Tauri launches the new macOS process before exiting this one. Hand
            // off the process lock while retaining the exclusive maintenance
            // lease, so this process cannot mutate the catalog during relaunch.
            handoff_for_restart(&cat)?;
            state.restarting.store(true, Ordering::SeqCst);
            app.restart();
        });
        // Windows exits inside the official installer. macOS restarts above.
        // On failure keep the verified bytes in memory so installation can retry.
        *state.package.lock().unwrap() = Some(bytes);
        state.installing.store(false, Ordering::SeqCst);
        result
    }).await.map_err(|_| AppError::new("UPDATE_INSTALL_FAILED", "更新任务异常，请重启应用后重试"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operations_are_serialized_and_release_on_error() {
        let state = UpdateState::default();
        let lease = state.acquire().unwrap();
        assert!(state.acquire().is_err());
        drop(lease);
        assert!(state.acquire().is_ok());
    }
    #[test]
    fn only_official_release_assets_are_accepted() {
        assert!(trusted_download(
            "https://github.com/merlinalex1028/travel-gallery/releases/download/v1.0.0/app.tar.gz"
        ));
        for url in [
            "http://github.com/merlinalex1028/travel-gallery/releases/download/v1/a",
            "https://github.com/other/repo/releases/download/v1/a",
            "https://github.com/merlinalex1028/travel-gallery/releases/download/v1/a?url=other",
            "https://github.com/merlinalex1028/travel-gallery/releases/download/v1/../a",
        ] {
            assert!(!trusted_download(url));
        }
    }
}
