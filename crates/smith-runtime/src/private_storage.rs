//! Owner-only, crash-safe files beneath Smith's user state.
//!
//! Exact checkpoints and completed-session snapshots use the same primitive so
//! neither format can accidentally regress to `File::create` under a
//! permissive umask. Smith's currently supported persistence hosts are macOS
//! and Linux. Other targets fail explicitly instead of claiming owner-only
//! durability without a native access-control implementation.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::os::unix::fs::PermissionsExt;

/// An exclusive advisory lease released when its file handle is dropped.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) struct PrivateFileLock(std::fs::File);

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) struct PrivateFileLock;

impl std::fmt::Debug for PrivateFileLock {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PrivateFileLock([held])")
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Drop for PrivateFileLock {
    fn drop(&mut self) {
        let _ = rustix::fs::flock(&self.0, rustix::fs::FlockOperation::Unlock);
    }
}

/// Distinguishes concurrent writers to the same destination.
static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Creates `directory`, then constrains its leaf permissions to the owner.
pub(crate) async fn ensure_private_directory(directory: &Path) -> Result<(), RuntimeError> {
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = directory;
        return Err(RuntimeError::new(
            ErrorKind::Config,
            "owner-only Smith persistence is unavailable on this platform",
        ));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        match tokio::fs::symlink_metadata(directory).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() => {
                return Err(io_error(format!(
                    "private path `{}` is not a regular directory",
                    directory.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(io_error(format!(
                    "cannot inspect private directory `{}`: {error}",
                    directory.display()
                )));
            }
        }
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|error| {
                io_error(format!(
                    "cannot create private directory `{}`: {error}",
                    directory.display()
                ))
            })?;
        let metadata = tokio::fs::symlink_metadata(directory)
            .await
            .map_err(|error| {
                io_error(format!(
                    "cannot inspect private directory `{}`: {error}",
                    directory.display()
                ))
            })?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            return Err(io_error(format!(
                "private path `{}` is not a regular directory",
                directory.display()
            )));
        }
        tokio::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(|error| {
                io_error(format!(
                    "cannot protect private directory `{}`: {error}",
                    directory.display()
                ))
            })
    }
}

/// Acquires a cross-process exclusive lease on an owner-only lock file.
///
/// Acquisition runs on the blocking pool because `flock(LOCK_EX)` waits for
/// the current writer. The persistent lock file is not a stale-lock marker:
/// the operating system releases the advisory lease when the descriptor
/// closes, including after a process crash.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) async fn acquire_private_lock(path: &Path) -> Result<PrivateFileLock, RuntimeError> {
    let path = path.to_owned();
    let directory = path.parent().ok_or_else(|| {
        io_error(format!(
            "`{}` has no parent directory for a private lock",
            path.display()
        ))
    })?;
    ensure_private_directory(directory).await?;
    tokio::task::spawn_blocking(move || acquire_private_lock_blocking(&path))
        .await
        .map_err(|_| io_error("private lock task stopped before acquisition".to_owned()))?
}

/// Attempts an exclusive owner-only lease without waiting for another process.
///
/// Session lifecycle ownership uses this form so a second Smith process fails
/// predictably instead of appearing hung behind an interactive turn in the
/// first process.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) async fn try_acquire_private_lock(path: &Path) -> Result<PrivateFileLock, RuntimeError> {
    let path = path.to_owned();
    let directory = path.parent().ok_or_else(|| {
        io_error(format!(
            "`{}` has no parent directory for a private lock",
            path.display()
        ))
    })?;
    ensure_private_directory(directory).await?;
    tokio::task::spawn_blocking(move || try_acquire_private_lock_blocking(&path))
        .await
        .map_err(|_| io_error("private lock task stopped before acquisition".to_owned()))?
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) async fn try_acquire_private_lock(path: &Path) -> Result<PrivateFileLock, RuntimeError> {
    let _ = path;
    Err(RuntimeError::new(
        ErrorKind::Config,
        "owner-only Smith persistence is unavailable on this platform",
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) async fn acquire_private_lock(path: &Path) -> Result<PrivateFileLock, RuntimeError> {
    let _ = path;
    Err(RuntimeError::new(
        ErrorKind::Config,
        "owner-only Smith persistence is unavailable on this platform",
    ))
}

/// Blocking form used while initializing the synchronous OS credential API.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn acquire_private_lock_blocking(path: &Path) -> Result<PrivateFileLock, RuntimeError> {
    let file = open_private_lock_blocking(path)?;
    rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive).map_err(|error| {
        io_error(format!(
            "cannot acquire private lock `{}`: {error}",
            path.display()
        ))
    })?;
    Ok(PrivateFileLock(file))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn try_acquire_private_lock_blocking(path: &Path) -> Result<PrivateFileLock, RuntimeError> {
    let file = open_private_lock_blocking(path)?;
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).map_err(
        |error| {
            if error == rustix::io::Errno::WOULDBLOCK {
                RuntimeError::conflict("the Smith session is already active in another host")
            } else {
                io_error(format!(
                    "cannot acquire private lock `{}`: {error}",
                    path.display()
                ))
            }
        },
    )?;
    Ok(PrivateFileLock(file))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn open_private_lock_blocking(path: &Path) -> Result<std::fs::File, RuntimeError> {
    let directory = path.parent().ok_or_else(|| {
        io_error(format!(
            "`{}` has no parent directory for a private lock",
            path.display()
        ))
    })?;
    ensure_private_directory_blocking(directory)?;
    let descriptor = rustix::fs::open(
        path,
        rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::RDWR
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .map_err(|error| {
        io_error(format!(
            "cannot open private lock `{}`: {error}",
            path.display()
        ))
    })?;
    let file = std::fs::File::from(descriptor);
    let stat = rustix::fs::fstat(&file).map_err(|error| {
        io_error(format!(
            "cannot inspect private lock `{}`: {error}",
            path.display()
        ))
    })?;
    if !rustix::fs::FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err(io_error(format!(
            "private lock `{}` is not a regular file",
            path.display()
        )));
    }
    rustix::fs::fchmod(&file, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR).map_err(
        |error| {
            io_error(format!(
                "cannot protect private lock `{}`: {error}",
                path.display()
            ))
        },
    )?;
    Ok(file)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn ensure_private_directory_blocking(directory: &Path) -> Result<(), RuntimeError> {
    match std::fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() => {
            return Err(io_error(format!(
                "private path `{}` is not a regular directory",
                directory.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(io_error(format!(
                "cannot inspect private directory `{}`: {error}",
                directory.display()
            )));
        }
    }
    std::fs::create_dir_all(directory).map_err(|error| {
        io_error(format!(
            "cannot create private directory `{}`: {error}",
            directory.display()
        ))
    })?;
    let metadata = std::fs::symlink_metadata(directory).map_err(|error| {
        io_error(format!(
            "cannot inspect private directory `{}`: {error}",
            directory.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(io_error(format!(
            "private path `{}` is not a regular directory",
            directory.display()
        )));
    }
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).map_err(|error| {
        io_error(format!(
            "cannot protect private directory `{}`: {error}",
            directory.display()
        ))
    })
}

/// Reads an owner-only regular file.
///
/// Existing files are repaired to mode `0600` before their contents are read.
/// A symlink is refused: an authenticated payload should never be fetched from
/// a path selected indirectly outside the reviewed user-state tree.
pub(crate) async fn read_private(path: &Path) -> Result<Option<Vec<u8>>, RuntimeError> {
    read_private_bounded(path, usize::MAX).await
}

/// Reads an owner-only regular file while refusing an oversized record.
pub(crate) async fn read_private_bounded(
    path: &Path,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>, RuntimeError> {
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (path, max_bytes);
        return Err(RuntimeError::new(
            ErrorKind::Config,
            "owner-only Smith persistence is unavailable on this platform",
        ));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let metadata = match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(io_error(format!(
                    "cannot inspect private file `{}`: {error}",
                    path.display()
                )));
            }
        };
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(io_error(format!(
                "private path `{}` is not a regular file",
                path.display()
            )));
        }
        if metadata.permissions().mode() & 0o777 != 0o600 {
            tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .await
                .map_err(|error| {
                    io_error(format!(
                        "cannot protect private file `{}`: {error}",
                        path.display()
                    ))
                })?;
        }
        let max_bytes_u64 = u64::try_from(max_bytes).unwrap_or(u64::MAX);
        if metadata.len() > max_bytes_u64 {
            return Err(io_error(format!(
                "private file `{}` exceeds its size limit",
                path.display()
            )));
        }
        let file = tokio::fs::File::open(path).await.map_err(|error| {
            io_error(format!(
                "cannot open private file `{}`: {error}",
                path.display()
            ))
        })?;
        let take_limit = max_bytes_u64.saturating_add(1);
        let mut bytes = Vec::with_capacity(
            usize::try_from(metadata.len())
                .unwrap_or(max_bytes)
                .min(max_bytes),
        );
        file.take(take_limit)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| {
                io_error(format!(
                    "cannot read private file `{}`: {error}",
                    path.display()
                ))
            })?;
        if bytes.len() > max_bytes {
            return Err(io_error(format!(
                "private file `{}` exceeds its size limit",
                path.display()
            )));
        }
        Ok(Some(bytes))
    }
}

/// Atomically replaces `path` with an owner-only file containing `bytes`.
///
/// The sibling temporary is opened with `create_new` and mode `0600`, written
/// and fsynced, then renamed over the destination. The directory fsync makes
/// the rename durable. No temporary name is model-selectable or reused.
pub(crate) async fn write_private_atomically(
    path: &Path,
    bytes: &[u8],
) -> Result<(), RuntimeError> {
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (path, bytes);
        return Err(RuntimeError::new(
            ErrorKind::Config,
            "owner-only Smith persistence is unavailable on this platform",
        ));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let directory = path.parent().ok_or_else(|| {
            io_error(format!(
                "`{}` has no parent directory to write into",
                path.display()
            ))
        })?;
        ensure_private_directory(directory).await?;
        let temporary = temporary_path(path);

        let write = async {
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true).create_new(true).mode(0o600);
            let mut file = options.open(&temporary).await?;
            file.write_all(bytes).await?;
            file.sync_all().await
        };
        if let Err(error) = write.await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(io_error(format!(
                "cannot write private file `{}`: {error}",
                path.display()
            )));
        }

        if let Err(error) = tokio::fs::rename(&temporary, path).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(io_error(format!(
                "cannot replace private file `{}`: {error}",
                path.display()
            )));
        }
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|error| {
                io_error(format!(
                    "cannot protect private file `{}`: {error}",
                    path.display()
                ))
            })?;

        // A file fsync alone does not make its directory entry durable.
        if let Ok(handle) = tokio::fs::File::open(directory).await {
            handle.sync_all().await.map_err(|error| {
                io_error(format!(
                    "cannot sync private directory `{}`: {error}",
                    directory.display()
                ))
            })?;
        }
        Ok(())
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    directory.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("smith-state"),
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn io_error(message: String) -> RuntimeError {
    RuntimeError::new(ErrorKind::Internal, message)
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_owner_only_file_and_directory_without_a_temporary_sibling() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("private");
        let path = directory.join("record");

        write_private_atomically(&path, b"secret").await.unwrap();

        assert_eq!(
            tokio::fs::metadata(&directory)
                .await
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            tokio::fs::metadata(&path)
                .await
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(read_private(&path).await.unwrap().unwrap(), b"secret");
        assert_eq!(
            tokio::fs::read_dir(&directory)
                .await
                .unwrap()
                .next_entry()
                .await
                .unwrap()
                .unwrap()
                .file_name(),
            "record"
        );
    }

    #[tokio::test]
    async fn read_repairs_permissive_mode_and_refuses_symlink() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        tokio::fs::write(&target, b"value").await.unwrap();
        tokio::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644))
            .await
            .unwrap();

        assert_eq!(read_private(&target).await.unwrap().unwrap(), b"value");
        assert_eq!(
            tokio::fs::metadata(&target)
                .await
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let link = root.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(read_private(&link).await.is_err());
    }

    #[tokio::test]
    async fn private_directory_refuses_a_symlinked_leaf() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        tokio::fs::create_dir(&target).await.unwrap();
        let link = root.path().join("private");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(ensure_private_directory(&link).await.is_err());
    }
}
