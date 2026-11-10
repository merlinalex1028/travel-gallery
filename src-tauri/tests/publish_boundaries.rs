//! Filesystem publication boundaries; fixtures contain only tiny text markers.
use cap_std::{ambient_authority, fs::Dir};
use std::io::{Read, Write};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use travel_gallery::export_errors::{classify, Operation};
use travel_gallery::publish;

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "publish-boundary-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn dir(&self) -> Dir {
        Dir::open_ambient_dir(&self.0, ambient_authority()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn complete_part_is_published_without_a_second_copy() {
    let f = Fixture::new();
    fs::write(f.0.join(".part"), b"verified").unwrap();
    publish::no_replace(&f.dir(), Path::new(".part"), Path::new("final.bin")).unwrap();
    assert_eq!(fs::read(f.0.join("final.bin")).unwrap(), b"verified");
    assert!(!f.0.join(".part").exists());
}

#[cfg(unix)]
#[test]
fn outside_parent_link_is_reported_as_an_unsafe_target_with_reselection() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("target")).unwrap();
    fs::create_dir(f.0.join("source")).unwrap();
    fs::write(f.0.join("source/keep"), b"original").unwrap();
    std::os::unix::fs::symlink(f.0.join("source"), f.0.join("target/album")).unwrap();
    let dir = Dir::open_ambient_dir(f.0.join("target"), ambient_authority()).unwrap();
    let error = dir.create_dir_all("album").unwrap_err();
    let failure = classify(Operation::Directory, error.kind());
    assert_eq!(failure.0, "EXPORT_TARGET_UNSAFE");
    assert!(failure.1.contains("重新选择"));
    assert_eq!(fs::read(f.0.join("source/keep")).unwrap(), b"original");
    assert_eq!(fs::read_dir(f.0.join("source")).unwrap().count(), 1);
}

#[test]
fn ordinary_file_in_the_parent_path_is_reported_without_modifying_it() {
    let f = Fixture::new();
    fs::write(f.0.join("album"), b"another writer").unwrap();
    let error = f.dir().create_dir_all("album").unwrap_err();
    assert_eq!(
        classify(Operation::Directory, error.kind()).0,
        "EXPORT_TARGET_UNSAFE"
    );
    assert_eq!(fs::read(f.0.join("album")).unwrap(), b"another writer");
}

#[test]
fn concurrent_destination_file_is_never_replaced() {
    let f = Fixture::new();
    fs::write(f.0.join(".part"), b"verified").unwrap();
    fs::write(f.0.join("final.bin"), b"existing").unwrap();
    assert!(publish::no_replace(&f.dir(), Path::new(".part"), Path::new("final.bin")).is_err());
    assert_eq!(fs::read(f.0.join("final.bin")).unwrap(), b"existing");
    assert_eq!(fs::read(f.0.join(".part")).unwrap(), b"verified");
}

#[test]
fn concurrent_destination_directory_is_never_replaced() {
    let f = Fixture::new();
    fs::write(f.0.join(".part"), b"verified").unwrap();
    fs::create_dir(f.0.join("final.bin")).unwrap();
    assert!(publish::no_replace(&f.dir(), Path::new(".part"), Path::new("final.bin")).is_err());
    assert!(f.0.join("final.bin").is_dir());
}

#[cfg(unix)]
#[test]
fn destination_symlink_does_not_change_its_source() {
    let f = Fixture::new();
    fs::write(f.0.join("source.bin"), b"source").unwrap();
    fs::write(f.0.join(".part"), b"verified").unwrap();
    std::os::unix::fs::symlink("source.bin", f.0.join("final.bin")).unwrap();
    assert!(publish::no_replace(&f.dir(), Path::new(".part"), Path::new("final.bin")).is_err());
    assert!(f.0.join("final.bin").is_symlink());
    assert_eq!(fs::read(f.0.join("source.bin")).unwrap(), b"source");
}

#[cfg(unix)]
#[test]
fn substituted_parent_symlink_cannot_escape_the_root() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("target")).unwrap();
    fs::create_dir(f.0.join("source")).unwrap();
    let root = Dir::open_ambient_dir(f.0.join("target"), ambient_authority()).unwrap();
    fs::create_dir(f.0.join("target/album")).unwrap();
    fs::write(f.0.join("target/album/.part"), b"verified").unwrap();
    fs::rename(f.0.join("target/album"), f.0.join("target/held")).unwrap();
    std::os::unix::fs::symlink(f.0.join("source"), f.0.join("target/album")).unwrap();
    assert!(publish::no_replace(
        &root,
        Path::new("album/.part"),
        Path::new("album/final.bin")
    )
    .is_err());
    assert!(!f.0.join("source/final.bin").exists());
    assert_eq!(
        fs::read(f.0.join("target/held/.part")).unwrap(),
        b"verified"
    );
}

#[cfg(unix)]
#[test]
fn substituted_root_path_does_not_redirect_an_open_handle() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("target")).unwrap();
    fs::create_dir(f.0.join("source")).unwrap();
    fs::write(f.0.join("target/.part"), b"verified").unwrap();
    let root = Dir::open_ambient_dir(f.0.join("target"), ambient_authority()).unwrap();
    fs::rename(f.0.join("target"), f.0.join("held")).unwrap();
    std::os::unix::fs::symlink(f.0.join("source"), f.0.join("target")).unwrap();
    publish::no_replace(&root, Path::new(".part"), Path::new("final.bin")).unwrap();
    assert!(!f.0.join("source/final.bin").exists());
    assert_eq!(fs::read(f.0.join("held/final.bin")).unwrap(), b"verified");
}

#[test]
fn swapped_child_directory_must_not_publish_an_unverified_part() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("album")).unwrap();
    fs::write(f.0.join("album/.part"), b"verified").unwrap();
    let root = f.dir();
    assert_eq!(root.read("album/.part").unwrap(), b"verified");
    // Deterministic boundary: replace the child after verification, before publication.
    fs::rename(f.0.join("album"), f.0.join("held")).unwrap();
    fs::create_dir(f.0.join("album")).unwrap();
    fs::write(f.0.join("album/.part"), b"injected").unwrap();
    let result = publish::no_replace(
        &root,
        Path::new("album/.part"),
        Path::new("album/final.bin"),
    );
    assert!(
        result.is_err(),
        "a new parent containing a different part must not be accepted as the verified copy"
    );
    assert!(!f.0.join("album/final.bin").exists());
    assert_eq!(fs::read(f.0.join("held/.part")).unwrap(), b"verified");
}

#[test]
fn staged_nested_copy_still_publishes_verified_bytes() {
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("year/album")).unwrap();
    let mut stage = publish::StagedFile::create(
        &f.dir(),
        Path::new("year/album/.part"),
        Path::new("year/album/final.bin"),
    )
    .unwrap();
    let mut writer = stage.writer().unwrap();
    writer.write_all(b"verified").unwrap();
    writer.sync_all().unwrap();
    drop(writer);
    let mut bytes = Vec::new();
    stage.reader().unwrap().read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"verified");
    stage.commit().unwrap();
    drop(stage);
    assert_eq!(
        fs::read(f.0.join("year/album/final.bin")).unwrap(),
        b"verified"
    );
    assert!(!f.0.join("year/album/.part").exists());
}

#[test]
fn staged_copy_rejects_swapped_parent_and_cleans_only_the_original_part() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("album")).unwrap();
    let mut stage = publish::StagedFile::create(
        &f.dir(),
        Path::new("album/.part"),
        Path::new("album/final.bin"),
    )
    .unwrap();
    stage.writer().unwrap().write_all(b"verified").unwrap();
    fs::rename(f.0.join("album"), f.0.join("held")).unwrap();
    fs::create_dir(f.0.join("album")).unwrap();
    fs::write(f.0.join("album/.part"), b"injected").unwrap();
    assert!(stage.commit().is_err());
    drop(stage);
    assert!(!f.0.join("album/final.bin").exists());
    assert!(!f.0.join("held/.part").exists());
    assert_eq!(fs::read(f.0.join("album/.part")).unwrap(), b"injected");
}

#[test]
fn create_collision_does_not_delete_the_preexisting_part() {
    let f = Fixture::new();
    fs::write(f.0.join(".part"), b"existing").unwrap();
    assert!(
        publish::StagedFile::create(&f.dir(), Path::new(".part"), Path::new("final.bin")).is_err()
    );
    assert_eq!(fs::read(f.0.join(".part")).unwrap(), b"existing");
}

#[test]
fn substituted_part_is_not_published_or_deleted() {
    let f = Fixture::new();
    let mut stage =
        publish::StagedFile::create(&f.dir(), Path::new(".part"), Path::new("final.bin")).unwrap();
    stage.writer().unwrap().write_all(b"verified").unwrap();
    fs::rename(f.0.join(".part"), f.0.join(".held")).unwrap();
    fs::write(f.0.join(".part"), b"injected").unwrap();
    assert!(stage.commit().is_err());
    drop(stage);
    assert_eq!(fs::read(f.0.join(".part")).unwrap(), b"injected");
    assert!(!f.0.join("final.bin").exists());
}

#[test]
fn staged_destination_collision_removes_own_part_and_preserves_destination() {
    let f = Fixture::new();
    let mut stage =
        publish::StagedFile::create(&f.dir(), Path::new(".part"), Path::new("final.bin")).unwrap();
    stage.writer().unwrap().write_all(b"verified").unwrap();
    fs::write(f.0.join("final.bin"), b"existing").unwrap();
    assert!(stage.commit().is_err());
    drop(stage);
    assert!(!f.0.join(".part").exists());
    assert_eq!(fs::read(f.0.join("final.bin")).unwrap(), b"existing");
}
