//! Directory notifications only schedule the existing read-only incremental scanner.
use crate::{
    catalog::{Catalog, Inner},
    error::Result,
};
use notify::{Config, Event, EventKind, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use serde_json::json;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc, Weak,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchStatus {
    pub active: bool,
    pub watched_roots: usize,
    pub polling_roots: usize,
    pub last_error: Option<String>,
}
#[derive(Default)]
pub(crate) struct WatchControl {
    stop: Option<Arc<AtomicBool>>,
    status: WatchStatus,
}
impl Catalog {
    pub fn start_watching(&self) {
        let mut control = self.inner.watcher.lock().unwrap();
        if control.stop.is_some() {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        control.stop = Some(stop.clone());
        control.status.active = true;
        let weak = Arc::downgrade(&self.inner);
        std::thread::spawn(move || run(weak, stop));
    }
    pub fn stop_watching(&self) {
        let mut control = self.inner.watcher.lock().unwrap();
        if let Some(stop) = control.stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        control.status = WatchStatus::default();
    }
    pub fn watch_status(&self) -> WatchStatus {
        self.inner.watcher.lock().unwrap().status.clone()
    }
}
struct Root {
    path: PathBuf,
    recursive: bool,
    online: bool,
    backend: Option<bool>, // true means polling fallback
    retry_at: Instant,
}
fn sender(
    tx: SyncSender<notify::Result<Event>>,
    overflow: Arc<AtomicBool>,
) -> impl FnMut(notify::Result<Event>) + Send + 'static {
    move |event| {
        if event
            .as_ref()
            .is_ok_and(|e| matches!(e.kind, EventKind::Access(_)))
        {
            return;
        }
        if tx.try_send(event).is_err() {
            overflow.store(true, Ordering::Relaxed);
        }
    }
}
fn roots(cat: &Catalog) -> Result<Vec<(String, PathBuf, bool)>> {
    let c = cat.connection()?;
    let mut statement = c.prepare("SELECT r.id,r.absolute_path,COALESCE(json_extract(o.options_json,'$.recursive'),1) FROM source_roots r LEFT JOIN source_options o ON o.source_id=r.id WHERE r.is_enabled=1")?;
    let rows = statement.query_map([], |r| {
        Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?), r.get(2)?))
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}
fn unwatch(root: &Root, native: &mut Option<RecommendedWatcher>, poll: &mut Option<PollWatcher>) {
    match root.backend {
        Some(true) => {
            if let Some(w) = poll {
                let _ = w.unwatch(&root.path);
            }
        }
        Some(false) => {
            if let Some(w) = native {
                let _ = w.unwatch(&root.path);
            }
        }
        None => {}
    }
}
fn run(weak: Weak<Inner>, stop: Arc<AtomicBool>) {
    let (tx, rx) = mpsc::sync_channel(256);
    let overflow = Arc::new(AtomicBool::new(false));
    let config = Config::default().with_follow_symlinks(false);
    let mut native = RecommendedWatcher::new(sender(tx.clone(), overflow.clone()), config).ok();
    // Polling is a fallback for filesystems without native notification support.
    // It compares metadata, never file contents, and never follows symbolic links.
    let mut poll = None;
    let mut watched = HashMap::<String, Root>::new();
    let mut dirty = HashMap::<String, Instant>::new();
    let mut reconcile_at = Instant::now();
    let mut last_error = None;
    while !stop.load(Ordering::Relaxed) {
        let Some(inner) = weak.upgrade() else {
            break;
        };
        let cat = Catalog { inner };
        {
            let _gate = cat.inner.maintenance.read().unwrap();
            if Instant::now() >= reconcile_at {
                reconcile_at = Instant::now() + Duration::from_secs(1);
                match roots(&cat) {
                    Err(error) => last_error = Some(error.message),
                    Ok(current) => {
                        watched.retain(|sid, root| {
                            let keep = current.iter().any(|(id, path, recursive)| {
                                id == sid && path == &root.path && recursive == &root.recursive
                            });
                            if !keep {
                                unwatch(root, &mut native, &mut poll);
                                dirty.remove(sid);
                            }
                            keep
                        });
                        for (sid, path, recursive) in current {
                            let root = watched.entry(sid.clone()).or_insert_with(|| {
                                dirty.insert(sid.clone(), Instant::now());
                                Root {
                                    path,
                                    recursive,
                                    online: false,
                                    backend: None,
                                    retry_at: Instant::now(),
                                }
                            });
                            let online = root.path.is_dir();
                            if root.online != online {
                                dirty.insert(sid.clone(), Instant::now());
                                if !online {
                                    unwatch(root, &mut native, &mut poll);
                                    root.backend = None;
                                }
                                root.online = online;
                                root.retry_at = Instant::now();
                            }
                            if !online || root.backend.is_some() || Instant::now() < root.retry_at {
                                continue;
                            }
                            let mode = if root.recursive {
                                RecursiveMode::Recursive
                            } else {
                                RecursiveMode::NonRecursive
                            };
                            let native_result = native.as_mut().map(|w| w.watch(&root.path, mode));
                            if native_result.is_some_and(|result| result.is_ok()) {
                                root.backend = Some(false);
                                last_error = None;
                                continue;
                            }
                            if poll.is_none() {
                                match PollWatcher::new(
                                    sender(tx.clone(), overflow.clone()),
                                    config.with_poll_interval(Duration::from_secs(30)),
                                ) {
                                    Ok(watcher) => poll = Some(watcher),
                                    Err(error) => {
                                        last_error =
                                            Some(format!("自动监听暂不可用，请手动重扫：{error}"))
                                    }
                                }
                            }
                            if let Some(watcher) = poll.as_mut() {
                                match watcher.watch(&root.path, mode) {
                                    Ok(()) => {
                                        root.backend = Some(true);
                                        last_error = None;
                                    }
                                    Err(error) => {
                                        last_error =
                                            Some(format!("自动监听暂不可用，请手动重扫：{error}"))
                                    }
                                }
                            }
                            root.retry_at = Instant::now() + Duration::from_secs(30);
                        }
                    }
                }
            }
            for event in rx.try_iter() {
                match event {
                    Ok(event) => {
                        let all = event.need_rescan() || event.paths.is_empty();
                        for (sid, root) in &watched {
                            if all || event.paths.iter().any(|path| path.starts_with(&root.path)) {
                                dirty.insert(sid.clone(), Instant::now());
                            }
                        }
                    }
                    Err(error) => {
                        last_error = Some(format!("目录监听发生错误，将重新扫描：{error}"));
                        for sid in watched.keys() {
                            dirty.insert(sid.clone(), Instant::now());
                        }
                    }
                }
            }
            if overflow.swap(false, Ordering::Relaxed) {
                for sid in watched.keys() {
                    dirty.insert(sid.clone(), Instant::now());
                }
            }
            if let Ok(c) = cat.connection() {
                dirty.retain(|sid, since| {
                    if stop.load(Ordering::Relaxed) {
                        return false;
                    }
                    if since.elapsed() < Duration::from_millis(800) {
                        return true;
                    }
                    match cat.queue(&c, "SCAN", json!({"sourceId":sid})) {
                        Ok(_) => false,
                        // Keep a pending scan if another scan is already running or paused.
                        Err(error) => {
                            if error.code != "DB_CONFLICT" {
                                last_error = Some(error.message);
                            }
                            true
                        }
                    }
                });
            }
            let mut control = cat.inner.watcher.lock().unwrap();
            if control
                .stop
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &stop))
            {
                control.status = WatchStatus {
                    active: true,
                    watched_roots: watched.values().filter(|r| r.backend.is_some()).count(),
                    polling_roots: watched.values().filter(|r| r.backend == Some(true)).count(),
                    last_error: last_error.clone(),
                };
            }
        }
        // The watcher holds no strong catalog reference while idle, so closing a
        // catalog releases its database lock even when a watch thread is alive.
        drop(cat);
        std::thread::sleep(Duration::from_millis(200));
    }
}
