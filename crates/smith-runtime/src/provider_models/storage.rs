//! Descriptor-relative private cache I/O. Every cache path component uses
//! NOFOLLOW; publication replaces a name rather than following its target.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use fs2::FileExt;

pub(super) struct CacheDirectory {
    directory: File,
}

impl CacheDirectory {
    #[cfg(unix)]
    pub(super) fn open(user_dir: &Path, create: bool) -> Result<Self, String> {
        use rustix::fs::{Mode, OFlags};
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
        let root = File::from(
            rustix::fs::open(user_dir, flags, Mode::empty())
                .map_err(|_| "cannot open the private Smith user directory".to_owned())?,
        );
        let mut directory = root;
        for name in ["cache", "provider-models-v1"] {
            if create {
                match rustix::fs::mkdirat(&directory, name, Mode::RWXU) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                    Err(_) => {
                        return Err("cannot create the private provider model directory".to_owned());
                    }
                }
            }
            directory = File::from(
                rustix::fs::openat(&directory, name, flags, Mode::empty())
                    .map_err(|_| "cannot open the private provider model directory".to_owned())?,
            );
            let metadata = directory
                .metadata()
                .map_err(|_| "cannot inspect the provider model directory".to_owned())?;
            if metadata.uid() != rustix::process::geteuid().as_raw() {
                return Err("the provider model directory belongs to another user".to_owned());
            }
            if create {
                rustix::fs::fchmod(&directory, Mode::RWXU)
                    .map_err(|_| "cannot protect the provider model directory".to_owned())?;
            } else if metadata.permissions().mode() & 0o077 != 0 {
                return Err("the provider model directory is not owner-private".to_owned());
            }
        }
        Ok(Self { directory })
    }

    #[cfg(not(unix))]
    pub(super) fn open(_user_dir: &Path, _create: bool) -> Result<Self, String> {
        Err("private provider model caches are unavailable on this platform".to_owned())
    }

    #[cfg(unix)]
    pub(super) fn read(&self, digest: &str, bound: usize) -> Option<Vec<u8>> {
        use rustix::fs::{Mode, OFlags};
        let filename = format!("{digest}.json");
        let file = File::from(
            rustix::fs::openat(
                &self.directory,
                filename.as_str(),
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .ok()?,
        );
        let metadata = file.metadata().ok()?;
        if !metadata.is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.permissions().mode() & 0o777 != 0o600
            || metadata.nlink() != 1
            || metadata.len() > bound as u64
        {
            return None;
        }
        let mut bytes = Vec::with_capacity((metadata.len() as usize).min(bound));
        file.take(bound as u64 + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() <= bound).then_some(bytes)
    }

    #[cfg(not(unix))]
    pub(super) fn read(&self, _digest: &str, _bound: usize) -> Option<Vec<u8>> {
        None
    }

    #[cfg(unix)]
    pub(super) async fn lock(&self, digest: &str) -> Result<CacheLock, String> {
        use rustix::fs::{Mode, OFlags};
        let filename = format!("{digest}.lock");
        let file = File::from(
            rustix::fs::openat(
                &self.directory,
                filename.as_str(),
                OFlags::RDWR
                    | OFlags::CREATE
                    | OFlags::CLOEXEC
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|_| "cannot open the private provider model lock".to_owned())?,
        );
        let metadata = file
            .metadata()
            .map_err(|_| "cannot inspect the provider model lock".to_owned())?;
        if !metadata.is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.nlink() != 1
        {
            return Err("the provider model lock is not an owner-held regular file".to_owned());
        }
        rustix::fs::fchmod(&file, Mode::RUSR | Mode::WUSR)
            .map_err(|_| "cannot protect the provider model lock".to_owned())?;
        // Non-blocking attempts make cancellation close the descriptor promptly,
        // even if a different Smith process currently owns the lock.
        tokio::time::timeout(Duration::from_secs(6), async {
            loop {
                match file.try_lock_exclusive() {
                    Ok(()) => return Ok(CacheLock(file)),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(_) => return Err("cannot acquire the provider model lock".to_owned()),
                }
            }
        })
        .await
        .map_err(|_| "another process is still refreshing this provider".to_owned())?
    }

    #[cfg(not(unix))]
    pub(super) async fn lock(&self, _digest: &str) -> Result<CacheLock, String> {
        Err("private provider model caches are unavailable on this platform".to_owned())
    }

    #[cfg(unix)]
    pub(super) fn publish(&self, digest: &str, bytes: &[u8]) -> Result<(), String> {
        use rustix::fs::{Mode, OFlags};
        let temporary = format!(".{digest}.{}.tmp", uuid::Uuid::new_v4());
        let destination = format!("{digest}.json");
        let result = (|| {
            let mut file = File::from(
                rustix::fs::openat(
                    &self.directory,
                    temporary.as_str(),
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::CLOEXEC
                        | OFlags::NOFOLLOW,
                    Mode::RUSR | Mode::WUSR,
                )
                .map_err(|_| "cannot create a private provider model temporary".to_owned())?,
            );
            // Explicit fchmod also handles an unusually restrictive umask.
            rustix::fs::fchmod(&file, Mode::RUSR | Mode::WUSR)
                .map_err(|_| "cannot protect the provider model temporary".to_owned())?;
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|_| "cannot write the provider model cache".to_owned())?;
            rustix::fs::renameat(
                &self.directory,
                temporary.as_str(),
                &self.directory,
                destination.as_str(),
            )
            .map_err(|_| "cannot atomically publish the provider model cache".to_owned())?;
            self.directory
                .sync_all()
                .map_err(|_| "cannot sync the provider model cache directory".to_owned())
        })();
        if result.is_err() {
            let _ = rustix::fs::unlinkat(
                &self.directory,
                temporary.as_str(),
                rustix::fs::AtFlags::empty(),
            );
        }
        result
    }

    #[cfg(not(unix))]
    pub(super) fn publish(&self, _digest: &str, _bytes: &[u8]) -> Result<(), String> {
        Err("private provider model caches are unavailable on this platform".to_owned())
    }
}

pub(super) struct CacheLock(File);

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
