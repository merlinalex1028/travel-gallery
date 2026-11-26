use crate::{
    catalog::Catalog,
    error::{AppError, Result},
};

/// Keep admission closed until installation/restart finishes. A worker or IPC command
/// already holding a read lease must finish before this exclusive lease is acquired.
pub fn with_idle_catalog<T>(catalog: &Catalog, install: impl FnOnce() -> Result<T>) -> Result<T> {
    let _lease = catalog
        .inner
        .maintenance
        .try_write()
        .map_err(|_| AppError::new("UPDATE_BUSY", "资料库正在使用，请稍后重试安装"))?;
    let connection = catalog.connection()?;
    let pending: i64 = connection.query_row(
        "SELECT count(*) FROM jobs WHERE status IN ('queued','running','paused')",
        [],
        |row| row.get(0),
    )?;
    if pending > 0 {
        return Err(AppError::new(
            "UPDATE_BUSY",
            "请先完成或取消所有后台任务（包括已暂停任务），再安装更新",
        ));
    }
    drop(connection);
    install()
}

/// Only call inside the exclusive install lease, immediately before exiting.
pub fn handoff_for_restart(catalog: &Catalog) -> Result<()> {
    catalog.stop_watching();
    fs2::FileExt::unlock(&catalog.inner._catalog_lock).map_err(|_| {
        AppError::new(
            "UPDATE_RESTART_FAILED",
            "更新已安装，但未能释放资料库锁；请退出并重新打开旅图",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installation_is_exclusive_and_never_runs_with_unfinished_work() {
        let dir = tempfile::tempdir().unwrap();
        let cat = Catalog::new(&dir.path().join("data"), &dir.path().join("cache")).unwrap();
        let read = cat.inner.maintenance.read().unwrap();
        assert!(with_idle_catalog(&cat, || Ok(())).is_err());
        drop(read);
        with_idle_catalog(&cat, || {
            assert!(cat.inner.maintenance.try_read().is_err());
            Ok(())
        })
        .unwrap();
        let c = cat.connection().unwrap();
        let jid = crate::id();
        crate::tasks::insert(&c, &jid, "SCAN", &serde_json::json!({})).unwrap();
        for status in ["queued", "running", "paused"] {
            c.execute(
                "UPDATE jobs SET status=? WHERE id=?",
                rusqlite::params![status, jid],
            )
            .unwrap();
            assert!(with_idle_catalog::<()>(&cat, || panic!("must not install")).is_err());
        }
        c.execute("UPDATE jobs SET status='cancelled'", []).unwrap();
        assert!(with_idle_catalog(&cat, || Ok(())).is_ok());
        assert!(cat.inner.maintenance.try_read().is_ok());
    }
    #[test]
    fn restart_hands_off_process_lock_without_admitting_old_catalog_operations() {
        let dir = tempfile::tempdir().unwrap();
        let cat = Catalog::new(&dir.path().join("data"), &dir.path().join("cache")).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(cat.inner.data.join("catalog.lock"))
            .unwrap();
        assert!(fs2::FileExt::try_lock_exclusive(&lock).is_err());
        with_idle_catalog(&cat, || {
            handoff_for_restart(&cat)?;
            assert!(cat.inner.maintenance.try_read().is_err());
            fs2::FileExt::try_lock_exclusive(&lock).unwrap();
            Ok(())
        })
        .unwrap();
    }
}
