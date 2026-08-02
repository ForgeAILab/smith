//! Owner-only plaintext credentials stored in `~/.smith/auth.json`.
//!
//! This backend is intentionally separate from the encrypted `file:`
//! credential placeholder. It is a Smith-owned, versioned JSON document and
//! is used only through trusted `authfile:<entry>` references. The contents are
//! plaintext: mode bits keep other Unix users out, but same-user processes and
//! backups can read them.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use agent_runtime_core::store::Secret;
use serde::{Deserialize, Serialize};

/// Fixed filename below Smith's user-state directory.
pub const AUTH_FILE_NAME: &str = "auth.json";

const AUTH_LOCK_NAME: &str = ".auth.lock";
const AUTH_FILE_SCHEMA_VERSION: u32 = 1;
const AUTH_FILE_MAX_BYTES: usize = 1024 * 1024;
static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Injectable plaintext auth-file operations.
///
/// Tests provide an in-memory implementation so no test can reach the
/// developer's home directory or operating-system credential service.
pub trait AuthFileBackend: Send + Sync {
    /// Reads one opaque secret entry.
    fn read(&self, entry: &str) -> Result<Option<Secret>, AuthFileError>;

    /// Replaces one entry while preserving every other entry.
    fn store(&self, entry: &str, secret: &Secret) -> Result<(), AuthFileError>;

    /// Atomically captures and replaces one entry.
    ///
    /// Production overrides this so the prior read and replacement share one
    /// cross-process lease. Injected stores may use the default composition.
    fn replace(&self, entry: &str, secret: &Secret) -> Result<Option<Secret>, AuthFileError> {
        let prior = self.read(entry)?;
        self.store(entry, secret)?;
        Ok(prior)
    }

    /// Removes one entry while preserving every other entry.
    fn remove(&self, entry: &str) -> Result<(), AuthFileError>;
}

/// Production owner-only auth-file backend.
#[derive(Debug, Clone)]
pub struct OwnerOnlyAuthFile {
    user_state: Option<PathBuf>,
}

impl OwnerOnlyAuthFile {
    /// Uses an explicitly selected user-state directory.
    pub fn new(user_state: impl Into<PathBuf>) -> Self {
        Self {
            user_state: Some(user_state.into()),
        }
    }

    /// Discovers Smith's fixed `~/.smith` user-state directory.
    pub fn discover() -> Self {
        Self {
            user_state: dirs::home_dir().map(|home| home.join(".smith")),
        }
    }

    fn paths(&self) -> Result<(&Path, PathBuf, PathBuf), AuthFileError> {
        let root = self
            .user_state
            .as_deref()
            .ok_or(AuthFileError::Unavailable)?;
        Ok((root, root.join(AUTH_FILE_NAME), root.join(AUTH_LOCK_NAME)))
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn with_document<T>(
        &self,
        operation: impl FnOnce(&mut AuthFileDocument) -> Result<(T, bool), AuthFileError>,
    ) -> Result<T, AuthFileError> {
        let (root, auth_path, lock_path) = self.paths()?;
        ensure_private_directory(root)?;
        let _lock = PrivateAuthLock::acquire(&lock_path)?;
        let mut document = read_document(&auth_path)?;
        let (value, changed) = operation(&mut document)?;
        if changed {
            write_document(&auth_path, &document)?;
        }
        Ok(value)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn with_document<T>(
        &self,
        _operation: impl FnOnce(&mut AuthFileDocument) -> Result<(T, bool), AuthFileError>,
    ) -> Result<T, AuthFileError> {
        Err(AuthFileError::UnsupportedPlatform)
    }
}

impl AuthFileBackend for OwnerOnlyAuthFile {
    fn read(&self, entry: &str) -> Result<Option<Secret>, AuthFileError> {
        self.with_document(|document| {
            Ok((
                document.credentials.get(entry).cloned().map(Secret::new),
                false,
            ))
        })
    }

    fn store(&self, entry: &str, secret: &Secret) -> Result<(), AuthFileError> {
        self.with_document(|document| {
            let changed = document
                .credentials
                .insert(entry.to_owned(), secret.expose().to_owned())
                .as_deref()
                != Some(secret.expose());
            Ok(((), changed))
        })
    }

    fn replace(&self, entry: &str, secret: &Secret) -> Result<Option<Secret>, AuthFileError> {
        self.with_document(|document| {
            let prior = document
                .credentials
                .insert(entry.to_owned(), secret.expose().to_owned())
                .map(Secret::new);
            let changed = prior.as_ref().map(Secret::expose) != Some(secret.expose());
            Ok((prior, changed))
        })
    }

    fn remove(&self, entry: &str) -> Result<(), AuthFileError> {
        self.with_document(|document| {
            let changed = document.credentials.remove(entry).is_some();
            Ok(((), changed))
        })
    }
}

/// A fixed, redaction-safe auth-file failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthFileError {
    /// Smith could not find a home directory for its fixed user state.
    #[error("Smith cannot locate its owner-only auth file")]
    Unavailable,
    /// Owner-only Unix persistence is not implemented on this target.
    #[error("owner-only auth-file storage is unavailable on this platform")]
    UnsupportedPlatform,
    /// The fixed directory, lock, or auth target is not a safe regular type.
    #[error("the owner-only auth path is a symlink or non-regular file")]
    UnsafePath,
    /// The document exceeded Smith's bounded read limit.
    #[error("the owner-only auth file exceeds its size limit")]
    Oversized,
    /// JSON or schema validation failed.
    #[error("the owner-only auth file is malformed or has an unsupported version")]
    Malformed,
    /// A filesystem operation failed. Its raw platform message is omitted so
    /// backend-controlled text cannot enter diagnostics.
    #[error("the owner-only auth file could not be accessed")]
    Io,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthFileDocument {
    schema_version: u32,
    credentials: BTreeMap<String, String>,
}

impl Default for AuthFileDocument {
    fn default() -> Self {
        Self {
            schema_version: AUTH_FILE_SCHEMA_VERSION,
            credentials: BTreeMap::new(),
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct PrivateAuthLock(fs::File);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl PrivateAuthLock {
    fn acquire(path: &Path) -> Result<Self, AuthFileError> {
        let descriptor = rustix::fs::open(
            path,
            rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .map_err(|_| AuthFileError::Io)?;
        let file = fs::File::from(descriptor);
        ensure_regular_descriptor(&file)?;
        rustix::fs::fchmod(&file, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR)
            .map_err(|_| AuthFileError::Io)?;
        rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
            .map_err(|_| AuthFileError::Io)?;
        Ok(Self(file))
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Drop for PrivateAuthLock {
    fn drop(&mut self) {
        let _ = rustix::fs::flock(&self.0, rustix::fs::FlockOperation::Unlock);
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn ensure_private_directory(path: &Path) -> Result<(), AuthFileError> {
    use std::os::unix::fs::PermissionsExt;

    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() => {
            return Err(AuthFileError::UnsafePath);
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(AuthFileError::Io),
    }
    fs::create_dir_all(path).map_err(|_| AuthFileError::Io)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| AuthFileError::Io)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(AuthFileError::UnsafePath);
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| AuthFileError::Io)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn read_document(path: &Path) -> Result<AuthFileDocument, AuthFileError> {
    let descriptor = match rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(rustix::io::Errno::NOENT) => return Ok(AuthFileDocument::default()),
        Err(rustix::io::Errno::LOOP) => return Err(AuthFileError::UnsafePath),
        Err(_) => return Err(AuthFileError::Io),
    };
    let mut file = fs::File::from(descriptor);
    ensure_regular_descriptor(&file)?;
    rustix::fs::fchmod(&file, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR)
        .map_err(|_| AuthFileError::Io)?;

    let stat = rustix::fs::fstat(&file).map_err(|_| AuthFileError::Io)?;
    if stat.st_size < 0
        || u64::try_from(stat.st_size).unwrap_or(u64::MAX)
            > u64::try_from(AUTH_FILE_MAX_BYTES).unwrap_or(u64::MAX)
    {
        return Err(AuthFileError::Oversized);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(stat.st_size).unwrap_or(0));
    Read::by_ref(&mut file)
        .take(u64::try_from(AUTH_FILE_MAX_BYTES).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AuthFileError::Io)?;
    if bytes.len() > AUTH_FILE_MAX_BYTES {
        return Err(AuthFileError::Oversized);
    }
    let document: AuthFileDocument =
        serde_json::from_slice(&bytes).map_err(|_| AuthFileError::Malformed)?;
    if document.schema_version != AUTH_FILE_SCHEMA_VERSION {
        return Err(AuthFileError::Malformed);
    }
    Ok(document)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn write_document(path: &Path, document: &AuthFileDocument) -> Result<(), AuthFileError> {
    use std::os::unix::fs::OpenOptionsExt;

    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.file_type().is_file())
    {
        return Err(AuthFileError::UnsafePath);
    }
    let bytes = serde_json::to_vec_pretty(document).map_err(|_| AuthFileError::Malformed)?;
    if bytes.len() > AUTH_FILE_MAX_BYTES {
        return Err(AuthFileError::Oversized);
    }
    let directory = path.parent().ok_or(AuthFileError::UnsafePath)?;
    let temporary = directory.join(format!(
        ".{}.{}.{}.tmp",
        AUTH_FILE_NAME,
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| AuthFileError::Io)?;
        file.write_all(&bytes).map_err(|_| AuthFileError::Io)?;
        file.sync_all().map_err(|_| AuthFileError::Io)?;
        fs::rename(&temporary, path).map_err(|_| AuthFileError::Io)?;
        let directory = fs::File::open(directory).map_err(|_| AuthFileError::Io)?;
        directory.sync_all().map_err(|_| AuthFileError::Io)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn ensure_regular_descriptor(file: &fs::File) -> Result<(), AuthFileError> {
    let stat = rustix::fs::fstat(file).map_err(|_| AuthFileError::Io)?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode).is_file() {
        Ok(())
    } else {
        Err(AuthFileError::UnsafePath)
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::sync::Arc;

    use super::*;

    #[test]
    fn writes_private_versioned_document_and_preserves_other_entries() {
        let parent = tempfile::tempdir().expect("temporary parent");
        let root = parent.path().join(".smith");
        let store = OwnerOnlyAuthFile::new(&root);

        store
            .store("other", &Secret::new("other-secret"))
            .expect("first entry");
        store
            .store("chatgpt", &Secret::new("chatgpt-secret"))
            .expect("chatgpt entry");
        store.remove("chatgpt").expect("remove chatgpt");

        assert!(store.read("chatgpt").expect("read").is_none());
        assert_eq!(
            store.read("other").expect("read").expect("other").expose(),
            "other-secret"
        );
        assert_eq!(
            fs::metadata(&root)
                .expect("root metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(root.join(AUTH_FILE_NAME))
                .expect("auth metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn refuses_malformed_oversized_and_symlink_documents_without_leaking() {
        let parent = tempfile::tempdir().expect("temporary parent");
        let root = parent.path().join("state");
        fs::create_dir(&root).expect("state");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("permissions");
        let path = root.join(AUTH_FILE_NAME);
        let store = OwnerOnlyAuthFile::new(&root);

        fs::write(&path, b"not-json token-canary").expect("malformed");
        assert_eq!(store.read("chatgpt"), Err(AuthFileError::Malformed));
        assert!(!format!("{}", AuthFileError::Malformed).contains("token-canary"));

        fs::write(&path, vec![b'x'; AUTH_FILE_MAX_BYTES + 1]).expect("oversized");
        assert_eq!(store.read("chatgpt"), Err(AuthFileError::Oversized));

        fs::remove_file(&path).expect("remove oversized");
        let target = parent.path().join("elsewhere");
        fs::write(&target, b"{}").expect("target");
        symlink(&target, &path).expect("symlink");
        assert!(matches!(
            store.read("chatgpt"),
            Err(AuthFileError::UnsafePath | AuthFileError::Io)
        ));
    }

    #[test]
    fn concurrent_writers_keep_every_entry() {
        let parent = tempfile::tempdir().expect("temporary parent");
        let root = parent.path().join("state");
        let store = Arc::new(OwnerOnlyAuthFile::new(&root));
        let threads = (0..12)
            .map(|index| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store
                        .store(
                            &format!("entry-{index}"),
                            &Secret::new(format!("value-{index}")),
                        )
                        .expect("write");
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().expect("writer");
        }
        for index in 0..12 {
            assert_eq!(
                store
                    .read(&format!("entry-{index}"))
                    .expect("read")
                    .expect("entry")
                    .expose(),
                format!("value-{index}")
            );
        }
    }
}
