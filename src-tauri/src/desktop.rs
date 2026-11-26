use crate::{
    catalog::Catalog,
    error::{required, AppError, Result},
    exit_guard::{ExitDecision, ExitGuard},
    files,
};
use serde_json::{json, Value};
use std::sync::{
    mpsc::{self, SyncSender},
    Arc, Mutex,
};
use tauri::{Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

macro_rules! commands {($($name:ident),* $(,)?)=>{$(
    #[tauri::command]
    async fn $name(state:State<'_,Catalog>,request:Option<Value>)->Result<Value>{let cat=state.inner().clone();tauri::async_runtime::spawn_blocking(move||cat.call(stringify!($name),request.unwrap_or(json!({})))).await.map_err(|_|AppError::new("INTERNAL","后台任务异常"))?}
)*};}
commands!(
    get_app_status,
    list_sources,
    add_source,
    rescan_source,
    set_source_enabled,
    remove_source,
    query_assets,
    query_burst_groups,
    create_burst_snapshot,
    get_cull_progress,
    get_asset,
    save_capture_time,
    get_asset_neighbors,
    resolve_preview,
    resolve_original,
    create_selection_snapshot,
    apply_review_batch,
    undo_review_batch,
    list_tags,
    upsert_tag,
    merge_tags,
    delete_tag,
    apply_tags_batch,
    start_duplicate_scan,
    query_duplicate_groups,
    query_group_members,
    set_group_representative,
    exclude_group_member,
    list_export_presets,
    save_export_preset,
    delete_export_preset,
    preview_rename,
    get_rename_preview,
    get_trip_story,
    get_quality_status,
    set_detail_enabled,
    set_eyes_enabled,
    import_eyes_model,
    remove_eyes_model,
    get_eyes_model_info,
    start_quality_analysis,
    get_quality_results,
    get_asset_quality,
    set_quality_dismissed,
    get_ai_status,
    get_ai_model_info,
    set_ai_enabled,
    import_ai_model,
    remove_ai_model,
    start_ai_index,
    start_ai_cluster,
    semantic_search,
    get_ai_clusters,
    preview_story_export,
    start_story_export,
    preview_export,
    start_export,
    get_export_report,
    list_jobs,
    get_job,
    get_job_errors,
    cancel_job,
    pause_job,
    resume_job,
    retry_job,
    backup_catalog,
    export_diagnostics,
    prepare_restore,
    restore_catalog,
    get_settings,
    save_settings,
    get_session,
    save_session,
    clear_cache,
    cache_stats,
    preview_relink_source,
    commit_relink_source,
    list_trips,
    save_trip,
    delete_trip,
    add_to_trip,
    remove_from_trip,
    set_trip_cover,
    list_saved_filters,
    save_filter,
    delete_filter,
    timeline_stats,
    map_assets,
    map_trails,
    get_media_links,
    link_media,
    unlink_media
);

#[tauri::command]
async fn pick_directory(
    app: tauri::AppHandle,
    state: State<'_, Catalog>,
    request: Value,
) -> Result<Value> {
    let purpose = required(&request, "purpose")?.to_string();
    if !["source", "export", "backup", "relink", "model", "diagnostics"].contains(&purpose.as_str()) {
        return Err(AppError::new("INVALID_FILTER", "目录用途无效"));
    }
    let cat = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let selected = app
            .dialog()
            .file()
            .set_title(match purpose.as_str() {
                "source" => "选择照片来源（只读索引）",
                "export" => "选择复制导出目录",
                "backup" => "选择资料库备份目录",
                "model" => "选择完整的旅图离线模型包文件夹",
                "diagnostics" => "选择脱敏诊断保存目录",
                _ => "选择来源的新位置",
            })
            .blocking_pick_folder();
        match selected {
            Some(p) => {
                let path = p
                    .into_path()
                    .map_err(|_| AppError::new("SOURCE_PATH_INVALID", "请选择本地目录"))?;
                let p = cat.authorize(&path, &purpose)?;
                Ok(json!({"path":p}))
            }
            None => Ok(json!({"path":null})),
        }
    })
    .await
    .map_err(|_| AppError::new("INTERNAL", "无法打开目录选择器"))?
}
#[tauri::command]
async fn pick_backup(app: tauri::AppHandle, state: State<'_, Catalog>) -> Result<Value> {
    let cat = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        match app
            .dialog()
            .file()
            .set_title("选择旅图资料库备份")
            .add_filter("SQLite 资料库", &["sqlite", "db"])
            .blocking_pick_file()
        {
            Some(p) => {
                let path = p
                    .into_path()
                    .map_err(|_| AppError::new("SOURCE_PATH_INVALID", "备份路径无效"))?;
                Ok(json!({"path":cat.authorize(&path,"restore")?}))
            }
            None => Ok(json!({"path":null})),
        }
    })
    .await
    .map_err(|_| AppError::new("INTERNAL", "无法选择备份"))?
}
#[tauri::command]
async fn open_asset(state: State<'_, Catalog>, request: Value) -> Result<Value> {
    let cat = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let c = cat.connection()?;
        let p = files::source_path(&c, required(&request, "assetId")?)?;
        open::that_detached(p)
            .map_err(|_| AppError::new("ASSET_UNSUPPORTED", "系统无法打开此媒体"))?;
        Ok(json!({"opened":true}))
    })
    .await
    .map_err(|_| AppError::new("INTERNAL", "无法打开媒体"))?
}
#[tauri::command]
async fn open_asset_location(state: State<'_, Catalog>, request: Value) -> Result<Value> {
    let cat = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let c = cat.connection()?;
        let directory = files::source_directory(&c, required(&request, "assetId")?)?;
        open::that(directory)
            .map_err(|_| AppError::new("PERMISSION_DENIED", "系统无法打开所在文件夹"))?;
        Ok(json!({"opened":true}))
    })
    .await
    .map_err(|_| AppError::new("INTERNAL", "无法打开所在文件夹"))?
}
#[tauri::command]
async fn resolve_playback(state: State<'_, Catalog>, request: Value) -> Result<Value> {
    let c = state.connection()?;
    let aid = required(&request, "assetId")?;
    let a = state.detail(&c, aid)?;
    if a["mediaKind"] != "video" {
        return Err(AppError::new("ASSET_UNSUPPORTED", "此媒体不是视频"));
    }
    let p = files::source_path(&c, aid)?;
    let mime = if a["extension"] == "webm" {
        "video/webm"
    } else if a["extension"] == "mov" {
        "video/quicktime"
    } else if a["extension"] == "mkv" {
        "video/x-matroska"
    } else if a["extension"] == "avi" {
        "video/x-msvideo"
    } else if a["extension"] == "mts" || a["extension"] == "m2ts" {
        "video/mp2t"
    } else {
        "video/mp4"
    };
    let token = crate::id();
    let mut handles = state.inner.previews.lock().unwrap();
    if handles.len() >= 5000 {
        handles.clear();
    }
    handles.insert(token.clone(), (p, mime.into()));
    Ok(json!({"resourceUrl":format!("media://localhost/{token}")}))
}

struct MediaRequest {
    catalog: Catalog,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
}
struct MediaProtocol(SyncSender<MediaRequest>);
impl MediaProtocol {
    fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel::<MediaRequest>(64);
        let receiver = Arc::new(Mutex::new(receiver));
        for index in 0..2 {
            let receiver = receiver.clone();
            std::thread::Builder::new()
                .name(format!("media-read-{index}"))
                .spawn(move || loop {
                    let job = receiver.lock().unwrap().recv();
                    let Ok(job) = job else { break };
                    let response = media_response(&job.catalog, &job.request)
                        .unwrap_or_else(|_| media_error(403));
                    job.responder.respond(response);
                })
                .expect("could not start media reader");
        }
        Self(sender)
    }
    fn submit(&self, job: MediaRequest) {
        if let Err(error) = self.0.try_send(job) {
            let job = match error {
                mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job) => job,
            };
            job.responder.respond(media_error(503));
        }
    }
}
fn media_error(status: u16) -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(status)
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-store")
        .header("Content-Length", "0")
        .body(vec![])
        .unwrap()
}
fn media_response(
    cat: &Catalog,
    request: &tauri::http::Request<Vec<u8>>,
) -> Result<tauri::http::Response<Vec<u8>>> {
    let token = request.uri().path().trim_start_matches('/');
    let (path, mime) = cat
        .inner
        .previews
        .lock()
        .unwrap()
        .get(token)
        .cloned()
        .ok_or_else(|| AppError::new("PERMISSION_DENIED", "预览句柄无效"))?;
    if path.canonicalize()? != path {
        return Err(AppError::new("PERMISSION_DENIED", "预览路径已变化"));
    }
    let mut file = std::fs::File::open(&path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(AppError::new("PERMISSION_DENIED", "此路径不是媒体文件"));
    }
    let plan = crate::media_response::plan(
        request.method().as_str(),
        request.headers().get("range").and_then(|r| r.to_str().ok()),
        request.headers().contains_key("if-range"),
        metadata.len(),
    );
    let body = crate::media_response::read_body(&mut file, &plan)?;
    let mut response = tauri::http::Response::builder()
        .status(plan.status)
        .header("Content-Type", mime)
        .header("Content-Length", plan.content_length.to_string())
        .header("Access-Control-Allow-Origin", "*")
        .header("Accept-Ranges", "bytes")
        .header("Cache-Control", "no-store");
    if let Some(range) = plan.content_range {
        response = response.header("Content-Range", range);
    }
    if plan.status == 405 {
        response = response.header("Allow", "GET, HEAD");
    }
    Ok(response.body(body).unwrap())
}

fn trace_exit(_stage: &str, _details: Value) {
    #[cfg(feature = "performance-probe")]
    crate::performance_probe::record_lifecycle(_stage, _details);
}

fn guard_exit(app: &tauri::AppHandle, origin: &str, prevent_exit: impl FnOnce()) {
    let update = app.state::<crate::updater::UpdateState>();
    if update.installing.load(std::sync::atomic::Ordering::SeqCst) {
        if !update.restarting.load(std::sync::atomic::Ordering::SeqCst) { prevent_exit(); }
        return;
    }
    trace_exit("exitGuardRequested", json!({"origin":origin}));
    let pending = app.state::<Catalog>().connection().ok().and_then(|c| {
        c.query_row("SELECT count(*) FROM jobs WHERE status IN ('queued','running','paused')", [], |r| r.get::<_, i64>(0)).ok()
    }).map(|count| count > 0).unwrap_or(true);
    let decision = app.state::<ExitGuard>().request(pending);
    trace_exit("exitGuardDecision", json!({"origin":origin,"unfinished":pending,"decision":format!("{decision:?}")}));
    match decision {
        ExitDecision::Allow => {}
        ExitDecision::WaitForPrompt => prevent_exit(),
        ExitDecision::Prompt => {
            prevent_exit();
            let window = app.get_webview_window("main");
            let mut dialog = app.dialog().message("退出会中断尚未完成的后台任务，重启后可重试；复制导出的临时文件会在报告中注明。确定退出？")
                .kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCancelCustom("退出并保留进度".into(), "继续工作".into()));
            if let Some(window) = window.as_ref() {
                dialog = dialog.parent(window);
            }
            trace_exit("exitDialogRequested", json!({"origin":origin,"parentWindow":window.is_some()}));
            let app = app.clone();
            dialog.show(move |exit| {
                    trace_exit("exitDialogResolved", json!({"confirmed":exit}));
                    app.state::<ExitGuard>().resolve(exit);
                    if exit { app.exit(0); }
                });
        }
    }
}

const QUIT_MENU_ID: &str = "travel-gallery-quit";

fn application_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    let menu = Menu::default(app)?;
    let quit_text = PredefinedMenuItem::quit(app, None)?.text()?;
    let quit = MenuItem::with_id(app, QUIT_MENU_ID, &quit_text, true, Some("CmdOrCtrl+Q"))?;
    // Native macOS Quit sends terminate: directly and skips ExitRequested.
    // Derive its label from the same factory to preserve the default menu and locale.
    for item in menu.items()? {
        if let Some(submenu) = item.as_submenu() {
            for (index, item) in submenu.items()?.iter().enumerate() {
                if let Some(native) = item.as_predefined_menuitem() {
                    if native.text()? == quit_text {
                        submenu.remove_at(index)?;
                        submenu.insert(&quit, index)?;
                    }
                }
            }
        }
    }
    Ok(menu)
}

pub fn run() {
    let media = MediaProtocol::new();
    tauri::Builder::default().plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_updater::Builder::new().build())
    .manage(crate::updater::UpdateState::default())
    .menu(application_menu)
    .on_menu_event(|app,event| { if event.id() == QUIT_MENU_ID {
        trace_exit("quitMenuSelected", json!({}));
        app.exit(0);
    } })
    .setup(|app|{
        #[cfg(feature = "offline-network-probe")]
        crate::offline_probe::verify(app.handle())?;
        #[cfg(feature = "performance-probe")]
        app.manage(crate::performance_probe::Probe::new(app.handle())?);
        let data=app.path().app_data_dir()?;let cache=app.path().app_cache_dir()?.join("thumbs");let cat=Catalog::new(&data,&cache)?;cat.startup_scan()?;cat.start_watching();app.manage(cat);app.manage(ExitGuard::default());Ok(())})
    .register_asynchronous_uri_scheme_protocol("media",move|ctx,request,responder|{
        let cat=ctx.app_handle().state::<Catalog>().inner().clone();
        media.submit(MediaRequest { catalog:cat, request, responder });
    })
    .on_window_event(|window,event|{
        if let tauri::WindowEvent::CloseRequested{api,..}=event{
            guard_exit(window.app_handle(), "windowClose", || api.prevent_close());
        }
    })
    .invoke_handler(tauri::generate_handler![crate::updater::update_support,crate::updater::check_app_update,crate::updater::download_app_update,crate::updater::install_app_update,#[cfg(feature = "performance-probe")] crate::performance_probe::performance_probe,get_app_status,list_sources,add_source,rescan_source,set_source_enabled,remove_source,query_assets,query_burst_groups,create_burst_snapshot,get_cull_progress,get_asset,save_capture_time,get_asset_neighbors,resolve_preview,resolve_original,create_selection_snapshot,apply_review_batch,undo_review_batch,list_tags,upsert_tag,merge_tags,delete_tag,apply_tags_batch,start_duplicate_scan,query_duplicate_groups,query_group_members,set_group_representative,exclude_group_member,list_export_presets,save_export_preset,delete_export_preset,preview_rename,get_rename_preview,get_trip_story,get_quality_status,set_detail_enabled,set_eyes_enabled,import_eyes_model,remove_eyes_model,get_eyes_model_info,start_quality_analysis,get_quality_results,get_asset_quality,set_quality_dismissed,get_ai_status,get_ai_model_info,set_ai_enabled,import_ai_model,remove_ai_model,start_ai_index,start_ai_cluster,semantic_search,get_ai_clusters,preview_story_export,start_story_export,preview_export,start_export,get_export_report,list_jobs,get_job,get_job_errors,cancel_job,pause_job,resume_job,retry_job,backup_catalog,export_diagnostics,prepare_restore,restore_catalog,get_settings,save_settings,get_session,save_session,clear_cache,cache_stats,preview_relink_source,commit_relink_source,list_trips,save_trip,delete_trip,add_to_trip,remove_from_trip,set_trip_cover,list_saved_filters,save_filter,delete_filter,timeline_stats,map_assets,map_trails,get_media_links,link_media,unlink_media,pick_directory,pick_backup,open_asset,open_asset_location,resolve_playback])
    .build(tauri::generate_context!()).expect("Travel Gallery could not start")
    .run(|app,event| {
        match event {
            tauri::RunEvent::ExitRequested { api, code, .. } => {
                trace_exit("runtimeExitRequested", json!({"code":code}));
                guard_exit(app, "applicationExit", || api.prevent_exit());
            }
            tauri::RunEvent::Exit => trace_exit("runtimeExit", json!({})),
            _ => {}
        }
    });
}
