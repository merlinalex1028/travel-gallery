//! Atomically publish a complete temporary copy without replacing a destination.
//! Parent directories are opened through cap-std; the OS operation uses that handle.
use cap_std::fs::{Dir, File, OpenOptions};
use std::{
    io::{self, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
};

fn leaf(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

#[cfg(unix)]
fn identity(file: &impl std::os::fd::AsFd) -> io::Result<(u64, u64)> {
    let stat = rustix::fs::fstat(file)?;
    Ok((stat.st_dev as u64, stat.st_ino as u64))
}

#[cfg(windows)]
fn identity(file: &impl std::os::windows::io::AsRawHandle) -> io::Result<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // The borrowed handle and output allocation remain valid throughout this call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((
        info.dwVolumeSerialNumber as u64,
        ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    ))
}

/// Keep the parent and created file pinned across writing, verification and publication.
pub struct StagedFile {
    root: Dir,
    parent: Dir,
    parent_path: PathBuf,
    part: PathBuf,
    destination: PathBuf,
    file: File,
    file_identity: (u64, u64),
    committed: bool,
}

impl StagedFile {
    pub fn create(root: &Dir, part: &Path, destination: &Path) -> io::Result<Self> {
        for path in [part, destination] {
            if path.as_os_str().is_empty()
                || path
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err(io::ErrorKind::InvalidInput.into());
            }
        }
        if part.parent() != destination.parent() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let parent_path = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = root.open_dir(parent_path)?;
        let root = root.try_clone()?;
        let part = PathBuf::from(part.file_name().unwrap());
        let destination = PathBuf::from(destination.file_name().unwrap());
        let file = parent.open_with(
            &part,
            OpenOptions::new().read(true).write(true).create_new(true),
        )?;
        let file_identity = identity(&file)?;
        Ok(Self {
            root,
            parent,
            parent_path: parent_path.to_path_buf(),
            part,
            destination,
            file,
            file_identity,
            committed: false,
        })
    }

    pub fn writer(&self) -> io::Result<File> {
        self.file.try_clone()
    }

    pub fn reader(&self) -> io::Result<File> {
        let mut file = self.file.try_clone()?;
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    }

    fn owns_part_entry(&self) -> io::Result<bool> {
        if self
            .parent
            .symlink_metadata(&self.part)?
            .file_type()
            .is_symlink()
        {
            return Ok(false);
        }
        Ok(identity(&self.parent.open(&self.part)?)? == self.file_identity)
    }

    pub fn commit(&mut self) -> io::Result<()> {
        let current_parent = self.root.open_dir(&self.parent_path)?;
        if identity(&current_parent)? != identity(&self.parent)? || !self.owns_part_entry()? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "export parent or temporary file was replaced",
            ));
        }
        no_replace(&self.parent, &self.part, &self.destination)?;
        self.committed = true;
        if self
            .parent
            .symlink_metadata(&self.destination)?
            .file_type()
            .is_symlink()
            || identity(&self.parent.open(&self.destination)?)? != self.file_identity
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "published file identity changed",
            ));
        }
        Ok(())
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        if !self.committed && self.owns_part_entry().unwrap_or(false) {
            let _ = self.parent.remove_file(&self.part);
        }
    }
}

pub fn no_replace(dir: &Dir, part: &Path, destination: &Path) -> io::Result<()> {
    // A nested path must be pinned with StagedFile before creating its part.
    if !leaf(part) || !leaf(destination) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let parent = dir.try_clone()?;
    let old = part
        .file_name()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let new = destination
        .file_name()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    #[cfg(unix)]
    {
        rustix::fs::renameat_with(
            &parent,
            old,
            &parent,
            new,
            rustix::fs::RenameFlags::NOREPLACE,
        )?;
        parent.into_std_file().sync_all()?;
        Ok(())
    }
    #[cfg(windows)]
    {
        use cap_std::fs::{OpenOptions, OpenOptionsExt};
        use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
        use windows_sys::Wdk::Storage::FileSystem::{
            FileRenameInformation, NtSetInformationFile, FILE_RENAME_INFORMATION,
        };
        use windows_sys::Win32::Foundation::RtlNtStatusToDosError;
        use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_FLAG_BACKUP_SEMANTICS};
        use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
        let mut options = OpenOptions::new();
        options.access_mode(DELETE);
        if parent.symlink_metadata(old)?.is_dir() {
            options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
        }
        let source = parent.open_with(old, &options)?;
        let name: Vec<u16> = new.encode_wide().collect();
        let size = (std::mem::offset_of!(FILE_RENAME_INFORMATION, FileName) + name.len() * 2)
            .max(std::mem::size_of::<FILE_RENAME_INFORMATION>());
        let mut buffer = vec![0u64; size.div_ceil(8)];
        let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
        let mut status_block = IO_STATUS_BLOCK::default();
        // The zeroed, u64-aligned allocation is large enough for the flexible UTF-16 name.
        // The handles outlive this synchronous call. ReplaceIfExists=false is mandatory.
        // Use the NT API so the leaf name resolves against the pinned directory handle,
        // without Win32 DOS-path translation or dependence on the process working directory.
        unsafe {
            (*info).Anonymous.ReplaceIfExists = false;
            (*info).RootDirectory = parent.as_raw_handle();
            (*info).FileNameLength = (name.len() * 2) as u32;
            std::ptr::copy_nonoverlapping(
                name.as_ptr(),
                std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
                name.len(),
            );
            let status = NtSetInformationFile(
                source.as_raw_handle(),
                &mut status_block,
                info.cast(),
                size as u32,
                FileRenameInformation,
            );
            if status < 0 {
                return Err(io::Error::from_raw_os_error(
                    RtlNtStatusToDosError(status) as i32,
                ));
            }
        }
        Ok(())
    }
}
