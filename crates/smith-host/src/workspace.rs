//! The project-root filesystem capability.
//!
//! The runtime's default is [`DenyAllWorkspace`](agent_runtime_core::workspace::DenyAllWorkspace),
//! which contains nothing. Smith replaces it with the project root the user
//! opened — and nothing above it.
//!
//! Canonical strings are retained for authorization displays, but actual I/O
//! is performed relative to an already-open [`cap_std::fs::Dir`]. The
//! capability resolver rejects symlink and component races that would escape
//! the opened root on Linux and macOS.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::workspace::Workspace;
use cap_std::ambient_authority;
use cap_std::fs::{Dir, MetadataExt, OpenOptions, PermissionsExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use xattr::FileExt as _;

/// Maximum number of entries returned by one capability walk.
const MAX_CAPABILITY_ENTRIES: usize = 100_000;

/// Stable identity and contents of one completed file read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileVersion {
    /// Canonical project path used only for diagnostics and attribution.
    pub canonical_path: String,
    /// Device identifier captured from the opened file.
    pub device: u64,
    /// Inode identifier captured from the opened file.
    pub inode: u64,
    /// Byte length captured from the opened file.
    pub size: u64,
    /// Exact modified timestamp represented as signed Unix nanoseconds.
    pub modified_unix_nanos: Option<String>,
    /// Exact inode creation timestamp as Unix nanoseconds, where the
    /// filesystem records one.
    ///
    /// The inode number alone does not identify an object: Linux reuses inode
    /// numbers immediately, so deleting a file and recreating it with the same
    /// bytes, size and a restored `mtime` reproduces every other field here.
    /// Birth time is assigned once when an inode is created and is not
    /// settable, so it separates the two. `ctime` cannot be used for this --
    /// renaming an inode updates it, and Smith's own durable replace renames
    /// the file it is verifying.
    ///
    /// Defaulted and compared only when both observations carry one: a
    /// filesystem without birth time, and a version recorded before this
    /// field existed, both leave it `None` rather than reporting every read
    /// stale.
    #[serde(default)]
    pub created_unix_nanos: Option<String>,
    /// SHA-256 of the bytes returned from the same handle.
    pub content_sha256: String,
}

impl FileVersion {
    /// Whether two observations describe the same object state.
    pub fn matches(&self, other: &Self) -> bool {
        self.device == other.device
            && self.inode == other.inode
            && self.size == other.size
            && self.modified_unix_nanos == other.modified_unix_nanos
            && self.content_sha256 == other.content_sha256
            && match (&self.created_unix_nanos, &other.created_unix_nanos) {
                (Some(mine), Some(theirs)) => mine == theirs,
                // One side predates the field, or the filesystem records no
                // birth time. Every other identity component still has to
                // agree, so this cannot widen what matches.
                _ => true,
            }
    }
}

/// Bytes and version captured through one already-opened file handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRead {
    /// Bounded file bytes.
    pub bytes: Vec<u8>,
    /// Identity and hash for those exact bytes.
    pub version: FileVersion,
}

/// One safely enumerated entry below the workspace capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEntry {
    /// Project-relative path, using platform path separators.
    pub relative_path: PathBuf,
    /// Whether the entry is a directory.
    pub is_dir: bool,
    /// Whether the entry is a regular file.
    pub is_file: bool,
    /// Size when metadata was available.
    pub size: Option<u64>,
}

#[derive(Debug)]
struct PreservedMetadata {
    permissions: cap_std::fs::Permissions,
    uid: u32,
    gid: u32,
    xattrs: Vec<(OsString, Vec<u8>)>,
    /// macOS ACL entries.
    ///
    /// Linux keeps POSIX ACLs in the `system.posix_acl_access` extended
    /// attribute, which `xattrs` above already captures and restores; macOS
    /// ACLs are not extended attributes and need the dedicated interface.
    #[cfg(target_os = "macos")]
    acl: Vec<exacl::AclEntry>,
}

/// A workspace rooted at one project directory.
#[derive(Debug, Clone)]
pub struct ProjectWorkspace {
    root: PathBuf,
    display: String,
    directory: Arc<Dir>,
    #[cfg(test)]
    commit_trace: Arc<std::sync::Mutex<Vec<&'static str>>>,
}

impl ProjectWorkspace {
    /// Roots a workspace at `root`, resolving symlinks and `..` components.
    ///
    /// The root must exist: a boundary that cannot be resolved is a boundary
    /// that cannot be enforced.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let root = root.as_ref();
        let canonical = root.canonicalize().map_err(|err| {
            RuntimeError::new(
                ErrorKind::Workspace,
                format!("project root `{}` is unusable: {err}", root.display()),
            )
        })?;
        if !canonical.is_dir() {
            return Err(RuntimeError::new(
                ErrorKind::Workspace,
                format!("project root `{}` is not a directory", canonical.display()),
            ));
        }
        let directory = Dir::open_ambient_dir(&canonical, ambient_authority()).map_err(|err| {
            RuntimeError::new(
                ErrorKind::Workspace,
                format!(
                    "project root `{}` could not be opened as a capability: {err}",
                    canonical.display()
                ),
            )
        })?;
        let display = canonical.to_string_lossy().into_owned();
        Ok(Self {
            root: canonical,
            display,
            directory: Arc::new(directory),
            #[cfg(test)]
            commit_trace: Arc::new(std::sync::Mutex::new(Vec::new())),
        })
    }

    /// Recovers Smith's concrete filesystem capability from a runtime
    /// workspace trait object.
    pub fn from_workspace(workspace: &dyn Workspace) -> Option<&Self> {
        (workspace as &dyn std::any::Any).downcast_ref::<Self>()
    }

    /// Converts a canonical or project-relative presentation path into a
    /// path accepted by the open directory capability.
    pub fn relative_path(&self, path: impl AsRef<Path>) -> Result<PathBuf, RuntimeError> {
        let path = path.as_ref();
        let resolved_absolute;
        let candidate = if path.is_absolute() {
            resolved_absolute =
                self.resolve_lexically(&path.to_string_lossy())
                    .ok_or_else(|| {
                        RuntimeError::new(
                            ErrorKind::Workspace,
                            format!("path `{}` could not be resolved", path.display()),
                        )
                    })?;
            resolved_absolute.strip_prefix(&self.root).map_err(|_| {
                RuntimeError::new(
                    ErrorKind::Workspace,
                    format!(
                        "path `{}` is outside the project `{}`",
                        path.display(),
                        self.display
                    ),
                )
            })?
        } else {
            path
        };
        let mut relative = PathBuf::new();
        for component in candidate.components() {
            match component {
                Component::Normal(part) => relative.push(part),
                Component::CurDir => {}
                Component::ParentDir => {
                    if !relative.pop() {
                        return Err(RuntimeError::new(
                            ErrorKind::Workspace,
                            format!("path `{}` escapes the project", path.display()),
                        ));
                    }
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(RuntimeError::new(
                        ErrorKind::Workspace,
                        format!("path `{}` is not project-relative", path.display()),
                    ));
                }
            }
        }
        Ok(relative)
    }

    /// Canonical display path for a project-relative capability path.
    pub fn display_path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.root.join(relative)
    }

    /// Opens once, checks metadata on that handle, and reads through a strict
    /// `max_bytes + 1` limiter. The returned version describes those bytes.
    pub fn read_bounded(
        &self,
        path: impl AsRef<Path>,
        max_bytes: usize,
    ) -> Result<CapabilityRead, RuntimeError> {
        let relative = self.relative_path(path)?;
        self.read_bounded_relative(&relative, max_bytes)
    }

    /// Reads a file when present, distinguishing an absent leaf from other
    /// capability or I/O failures.
    pub fn read_optional_bounded(
        &self,
        path: impl AsRef<Path>,
        max_bytes: usize,
    ) -> Result<Option<CapabilityRead>, RuntimeError> {
        let relative = self.relative_path(path)?;
        if relative.as_os_str().is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::Tool,
                "the project root is a directory, not a regular file",
            ));
        }
        let file = match self.directory.open(&relative) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(workspace_io("open", &self.display_path(&relative), error));
            }
        };
        self.read_opened(&relative, file, max_bytes).map(Some)
    }

    fn read_bounded_relative(
        &self,
        relative: &Path,
        max_bytes: usize,
    ) -> Result<CapabilityRead, RuntimeError> {
        if relative.as_os_str().is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::Tool,
                format!(
                    "`{}` is not a regular file; use the `list` tool for directories",
                    self.display_path(relative).display()
                ),
            ));
        }
        let file = self
            .directory
            .open(relative)
            .map_err(|err| workspace_io("open", &self.display_path(relative), err))?;
        self.read_opened(relative, file, max_bytes)
    }

    fn read_opened(
        &self,
        relative: &Path,
        mut file: cap_std::fs::File,
        max_bytes: usize,
    ) -> Result<CapabilityRead, RuntimeError> {
        let metadata = file
            .metadata()
            .map_err(|err| workspace_io("inspect", &self.display_path(relative), err))?;
        if !metadata.is_file() {
            return Err(RuntimeError::new(
                ErrorKind::Tool,
                format!(
                    "`{}` is not a regular file; use the `list` tool for directories",
                    self.display_path(relative).display()
                ),
            ));
        }
        let limit = u64::try_from(max_bytes)
            .unwrap_or(u64::MAX - 1)
            .saturating_add(1);
        let mut bytes = Vec::with_capacity(
            usize::try_from(metadata.len().min(limit)).unwrap_or(max_bytes.saturating_add(1)),
        );
        Read::by_ref(&mut file)
            .take(limit)
            .read_to_end(&mut bytes)
            .map_err(|err| workspace_io("read", &self.display_path(relative), err))?;
        if bytes.len() > max_bytes {
            return Err(RuntimeError::new(
                ErrorKind::Tool,
                format!(
                    "`{}` is over the {max_bytes}-byte read limit",
                    self.display_path(relative).display()
                ),
            ));
        }
        Ok(CapabilityRead {
            version: version_from(&self.display_path(relative), &file, &metadata, &bytes),
            bytes,
        })
    }

    /// Enumerates entries through already-opened directory handles. Symlinks
    /// are reported but never used as recursive traversal roots.
    pub fn entries(
        &self,
        path: impl AsRef<Path>,
        recursive: bool,
        include_hidden: bool,
        limit: usize,
    ) -> Result<Vec<WorkspaceEntry>, RuntimeError> {
        let relative = self.relative_path(path)?;
        let directory = self
            .open_directory(&relative)
            .map_err(|err| workspace_io("open directory", &self.display_path(&relative), err))?;
        let mut entries = Vec::new();
        walk_directory(
            directory,
            relative,
            recursive,
            include_hidden,
            limit.min(MAX_CAPABILITY_ENTRIES),
            &mut entries,
        )?;
        Ok(entries)
    }

    /// Inspects one entry through the capability, following only symlinks that
    /// remain beneath the opened root.
    pub fn entry(&self, path: impl AsRef<Path>) -> Result<WorkspaceEntry, RuntimeError> {
        let relative = self.relative_path(path)?;
        if relative.as_os_str().is_empty() {
            let metadata = self
                .directory
                .dir_metadata()
                .map_err(|err| workspace_io("inspect", &self.display_path(&relative), err))?;
            return Ok(WorkspaceEntry {
                relative_path: relative,
                is_dir: metadata.is_dir(),
                is_file: metadata.is_file(),
                size: Some(metadata.len()),
            });
        }
        let file = self
            .directory
            .open(&relative)
            .map_err(|err| workspace_io("open", &self.display_path(&relative), err))?;
        let metadata = file
            .metadata()
            .map_err(|err| workspace_io("inspect", &self.display_path(&relative), err))?;
        Ok(WorkspaceEntry {
            relative_path: relative,
            is_dir: metadata.is_dir(),
            is_file: metadata.is_file(),
            size: Some(metadata.len()),
        })
    }

    /// Creates a new regular file without following or replacing a leaf.
    pub fn create_new(&self, path: impl AsRef<Path>, contents: &[u8]) -> Result<(), RuntimeError> {
        let relative = self.relative_path(path)?;
        let (parent, leaf) = self.open_parent(&relative)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = parent
            .open_with(&leaf, &options)
            .map_err(|err| workspace_io("create", &self.display_path(&relative), err))?;
        if let Err(err) = file.write_all(contents).and_then(|()| file.sync_all()) {
            let _ = parent.remove_file(&leaf);
            return Err(workspace_io("write", &self.display_path(&relative), err));
        }
        sync_directory(&parent)
            .map_err(|err| workspace_io("sync parent of", &self.display_path(&relative), err))
    }

    /// Verifies that a target's parent exists and is reachable beneath the
    /// opened workspace root.
    pub fn ensure_parent_directory(&self, path: impl AsRef<Path>) -> Result<(), RuntimeError> {
        let relative = self.relative_path(path)?;
        self.open_parent(&relative).map(|_| ())
    }

    /// Atomically exchanges a metadata-preserving replacement with the target,
    /// then verifies that the displaced inode matches `expected` before the
    /// previous contents are discarded.
    pub fn replace_if_version(
        &self,
        path: impl AsRef<Path>,
        contents: &[u8],
        expected: &FileVersion,
        max_bytes: usize,
    ) -> Result<(), RuntimeError> {
        let relative = self.relative_path(path)?;
        let (parent, leaf) = self.open_parent(&relative)?;
        let original = parent
            .open(&leaf)
            .map_err(|err| workspace_io("open", &self.display_path(&relative), err))?;
        let preserved = preserved_metadata(&original).map_err(|err| {
            workspace_io("capture metadata for", &self.display_path(&relative), err)
        })?;
        let temporary = OsString::from(format!(".smith-edit-{}.tmp", Uuid::new_v4()));
        let new_version =
            self.write_temporary(&parent, &temporary, &relative, contents, Some(&preserved))?;

        if let Err(err) = exchange(&parent, &temporary, &leaf) {
            let _ = parent.remove_file(&temporary);
            return Err(workspace_io("publish", &self.display_path(&relative), err));
        }
        self.trace_commit("published-exchange");

        let displaced = read_from_dir(
            &parent,
            &temporary,
            &self.display_path(&relative),
            max_bytes,
        );
        if !displaced
            .as_ref()
            .is_ok_and(|read| expected.matches(&read.version))
        {
            rollback_exchange(&parent, &temporary, &leaf, &new_version, max_bytes)?;
            return Err(stale(&self.display_path(&relative)));
        }

        parent.remove_file(&temporary).map_err(|err| {
            workspace_io(
                "remove replaced inode for",
                &self.display_path(&relative),
                err,
            )
        })?;
        self.trace_commit("old-inode-removed");
        sync_directory(&parent)
            .map_err(|err| workspace_io("sync parent of", &self.display_path(&relative), err))?;
        self.trace_commit("parent-synced");
        Ok(())
    }

    /// Deletes only the version named by `expected`. Atomic exchange makes the
    /// displaced object available for verification before it is removed.
    pub fn delete_if_version(
        &self,
        path: impl AsRef<Path>,
        expected: &FileVersion,
        max_bytes: usize,
    ) -> Result<(), RuntimeError> {
        let relative = self.relative_path(path)?;
        let (parent, leaf) = self.open_parent(&relative)?;
        let placeholder = OsString::from(format!(".smith-delete-{}.tmp", Uuid::new_v4()));
        let placeholder_version =
            self.write_temporary(&parent, &placeholder, &relative, &[], None)?;
        if let Err(err) = exchange(&parent, &placeholder, &leaf) {
            let _ = parent.remove_file(&placeholder);
            return Err(workspace_io(
                "stage deletion of",
                &self.display_path(&relative),
                err,
            ));
        }
        let displaced = read_from_dir(
            &parent,
            &placeholder,
            &self.display_path(&relative),
            max_bytes,
        );
        if !displaced
            .as_ref()
            .is_ok_and(|read| expected.matches(&read.version))
        {
            rollback_exchange(
                &parent,
                &placeholder,
                &leaf,
                &placeholder_version,
                max_bytes,
            )?;
            return Err(stale(&self.display_path(&relative)));
        }

        parent
            .remove_file(&placeholder)
            .map_err(|err| workspace_io("remove", &self.display_path(&relative), err))?;
        if current_from_dir(&parent, &leaf, &self.display_path(&relative), max_bytes)
            .is_ok_and(|current| placeholder_version.matches(&current))
        {
            parent.remove_file(&leaf).map_err(|err| {
                workspace_io(
                    "remove deletion placeholder for",
                    &self.display_path(&relative),
                    err,
                )
            })?;
        }
        sync_directory(&parent)
            .map_err(|err| workspace_io("sync parent of", &self.display_path(&relative), err))
    }

    fn open_parent(&self, relative: &Path) -> Result<(Dir, OsString), RuntimeError> {
        let leaf = relative.file_name().ok_or_else(|| {
            RuntimeError::new(ErrorKind::Tool, "the project root cannot be edited")
        })?;
        let parent_relative = relative.parent().unwrap_or_else(|| Path::new(""));
        let parent = self.open_directory(parent_relative).map_err(|err| {
            workspace_io(
                "open parent directory of",
                &self.display_path(relative),
                err,
            )
        })?;
        Ok((parent, leaf.to_owned()))
    }

    fn open_directory(&self, relative: &Path) -> std::io::Result<Dir> {
        if relative.as_os_str().is_empty() {
            self.directory.try_clone()
        } else {
            self.directory.open_dir(relative)
        }
    }

    fn write_temporary(
        &self,
        parent: &Dir,
        temporary: &OsString,
        target: &Path,
        contents: &[u8],
        metadata: Option<&PreservedMetadata>,
    ) -> Result<FileVersion, RuntimeError> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        let mut file = parent
            .open_with(temporary, &options)
            .map_err(|err| workspace_io("create temporary for", &self.display_path(target), err))?;
        let result = (|| {
            file.write_all(contents)?;
            if let Some(metadata) = metadata {
                apply_metadata(&file, metadata)?;
            }
            file.sync_all()?;
            self.trace_commit("temporary-synced");
            let observed = file.metadata()?;
            Ok(version_from(
                &self.display_path(target),
                &file,
                &observed,
                contents,
            ))
        })();
        match result {
            Ok(version) => Ok(version),
            Err(err) => {
                drop(file);
                let _ = parent.remove_file(temporary);
                Err(workspace_io(
                    "prepare replacement for",
                    &self.display_path(target),
                    err,
                ))
            }
        }
    }

    /// Resolves `path` against the root without requiring it to exist, so a
    /// tool may create a new file inside the boundary.
    ///
    /// Existing paths are canonicalized. For a path that does not exist yet,
    /// the nearest existing ancestor is canonicalized and the remaining
    /// components are applied lexically — `..` pops, `.` is dropped — so a
    /// traversal cannot climb out through a not-yet-created directory.
    fn resolve_lexically(&self, path: &str) -> Option<PathBuf> {
        let candidate = {
            let raw = Path::new(path);
            if raw.is_absolute() {
                raw.to_path_buf()
            } else {
                self.root.join(raw)
            }
        };

        if let Ok(canonical) = candidate.canonicalize() {
            return Some(canonical);
        }

        // Walk down from the deepest ancestor that does exist.
        let mut existing = candidate.as_path();
        let mut trailing = Vec::new();
        loop {
            let parent = existing.parent()?;
            trailing.push(existing.file_name()?.to_owned());
            existing = parent;
            if let Ok(canonical) = existing.canonicalize() {
                let mut resolved = canonical;
                for component in trailing.iter().rev() {
                    match Path::new(component).components().next() {
                        Some(Component::ParentDir) => {
                            resolved.pop();
                        }
                        Some(Component::CurDir) => {}
                        _ => resolved.push(component),
                    }
                }
                return Some(resolved);
            }
        }
    }

    #[cfg(test)]
    fn trace_commit(&self, event: &'static str) {
        self.commit_trace.lock().expect("commit trace").push(event);
    }

    #[cfg(not(test))]
    fn trace_commit(&self, _event: &'static str) {}
}

fn walk_directory(
    directory: Dir,
    relative: PathBuf,
    recursive: bool,
    include_hidden: bool,
    limit: usize,
    output: &mut Vec<WorkspaceEntry>,
) -> Result<(), RuntimeError> {
    for entry in directory
        .entries()
        .map_err(|err| workspace_io("list", &relative, err))?
    {
        if output.len() >= limit {
            break;
        }
        let entry = entry.map_err(|err| workspace_io("list", &relative, err))?;
        let name = entry.file_name();
        if !include_hidden {
            let name = name.to_string_lossy();
            if name.starts_with('.') || matches!(name.as_ref(), "target" | "node_modules") {
                continue;
            }
        }
        let child = relative.join(&name);
        let file_type = entry
            .file_type()
            .map_err(|err| workspace_io("inspect", &child, err))?;
        let metadata = entry.metadata().ok();
        output.push(WorkspaceEntry {
            relative_path: child.clone(),
            is_dir: file_type.is_dir(),
            is_file: file_type.is_file(),
            size: metadata.as_ref().map(cap_std::fs::Metadata::len),
        });
        if recursive && file_type.is_dir() && output.len() < limit {
            let child_dir = entry
                .open_dir()
                .map_err(|err| workspace_io("open directory", &child, err))?;
            walk_directory(child_dir, child, true, include_hidden, limit, output)?;
        }
    }
    Ok(())
}

fn read_from_dir(
    directory: &Dir,
    name: &OsString,
    display: &Path,
    max_bytes: usize,
) -> Result<CapabilityRead, RuntimeError> {
    let mut file = directory
        .open(name)
        .map_err(|err| workspace_io("open", display, err))?;
    let metadata = file
        .metadata()
        .map_err(|err| workspace_io("inspect", display, err))?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(
            u64::try_from(max_bytes)
                .unwrap_or(u64::MAX - 1)
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)
        .map_err(|err| workspace_io("read", display, err))?;
    if bytes.len() > max_bytes {
        return Err(RuntimeError::new(
            ErrorKind::Tool,
            format!("`{}` changed to exceed the read limit", display.display()),
        ));
    }
    Ok(CapabilityRead {
        version: version_from(display, &file, &metadata, &bytes),
        bytes,
    })
}

fn current_from_dir(
    directory: &Dir,
    name: &OsString,
    display: &Path,
    max_bytes: usize,
) -> Result<FileVersion, RuntimeError> {
    read_from_dir(directory, name, display, max_bytes).map(|read| read.version)
}

fn rollback_exchange(
    parent: &Dir,
    temporary: &OsString,
    leaf: &OsString,
    published: &FileVersion,
    max_bytes: usize,
) -> Result<(), RuntimeError> {
    let display = Path::new(leaf);
    if current_from_dir(parent, leaf, display, max_bytes)
        .is_ok_and(|current| published.matches(&current))
    {
        exchange(parent, temporary, leaf)
            .map_err(|err| workspace_io("roll back stale edit of", display, err))?;
        parent
            .remove_file(temporary)
            .map_err(|err| workspace_io("clean stale edit of", display, err))?;
        sync_directory(parent).map_err(|err| workspace_io("sync rollback of", display, err))?;
        Ok(())
    } else {
        Err(RuntimeError::new(
            ErrorKind::Tool,
            format!(
                "`{}` changed concurrently; the displaced file was preserved as `{}`",
                display.display(),
                temporary.to_string_lossy()
            ),
        ))
    }
}

fn exchange(directory: &Dir, first: &OsString, second: &OsString) -> std::io::Result<()> {
    Ok(rustix::fs::renameat_with(
        directory,
        first,
        directory,
        second,
        rustix::fs::RenameFlags::EXCHANGE,
    )?)
}

/// Durably commits a directory's entries.
///
/// cap-std opens directory handles with `O_PATH` on Linux, and `fsync` on an
/// `O_PATH` descriptor fails with `EBADF` -- it names a location rather than
/// an open file. Reopening the directory through its own descriptor yields a
/// syncable handle without widening authority: `openat` relative to a
/// directory descriptor stays inside that directory, so this is the same
/// sandbox cap-std already enforces. macOS has no `O_PATH` and would sync the
/// original handle, but it reopens the same way so both platforms commit
/// through one path.
fn sync_directory(directory: &Dir) -> std::io::Result<()> {
    let syncable = rustix::fs::openat(
        directory,
        ".",
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    Ok(rustix::fs::fsync(&syncable)?)
}

fn preserved_metadata(file: &cap_std::fs::File) -> std::io::Result<PreservedMetadata> {
    let metadata = file.metadata()?;
    let std_file = file.try_clone()?.into_std();
    let mut xattrs = Vec::new();
    for name in std_file.list_xattr()? {
        if let Some(value) = std_file.get_xattr(&name)? {
            xattrs.push((name, value));
        }
    }
    Ok(PreservedMetadata {
        permissions: metadata.permissions(),
        uid: metadata.uid(),
        gid: metadata.gid(),
        xattrs,
        #[cfg(target_os = "macos")]
        acl: exacl::getfacl(descriptor_path(&std_file), None)?,
    })
}

fn apply_metadata(file: &cap_std::fs::File, metadata: &PreservedMetadata) -> std::io::Result<()> {
    file.set_permissions(metadata.permissions.clone())?;
    let current = file.metadata()?;
    if current.uid() != metadata.uid || current.gid() != metadata.gid {
        rustix::fs::fchown(
            file,
            Some(rustix::process::Uid::from_raw(metadata.uid)),
            Some(rustix::process::Gid::from_raw(metadata.gid)),
        )?;
    }
    let std_file = file.try_clone()?.into_std();
    for (name, value) in &metadata.xattrs {
        std_file.set_xattr(name, value)?;
    }
    #[cfg(target_os = "macos")]
    exacl::setfacl(&[descriptor_path(&std_file)], &metadata.acl, None)?;
    let applied = file.metadata()?;
    if applied.mode() & 0o7777 != metadata.permissions.mode() & 0o7777
        || applied.uid() != metadata.uid
        || applied.gid() != metadata.gid
    {
        return Err(std::io::Error::other(
            "replacement metadata did not match the original",
        ));
    }
    Ok(())
}

/// Names an open descriptor as a path, for the macOS ACL interface.
///
/// Only macOS needs this: Linux carries ACLs in extended attributes, which are
/// read and written through the descriptor itself.
#[cfg(target_os = "macos")]
fn descriptor_path(file: &std::fs::File) -> PathBuf {
    use std::os::fd::AsRawFd;
    PathBuf::from(format!("/dev/fd/{}", file.as_raw_fd()))
}

/// Inode birth time as Unix nanoseconds, where the platform reports one.
///
/// cap-std fills `Metadata::created` from the platform `stat` struct, which
/// carries a birth time on macOS and the BSDs but not on Linux. Linux exposes
/// it only through `statx`, so ask for it directly rather than accept the
/// `None` that `Metadata::created` always returns there.
#[cfg(target_os = "linux")]
fn created_unix_nanos(file: &cap_std::fs::File) -> Option<String> {
    use std::os::fd::AsFd;

    let stat = rustix::fs::statx(
        file.as_fd(),
        "",
        rustix::fs::AtFlags::EMPTY_PATH,
        rustix::fs::StatxFlags::BTIME,
    )
    .ok()?;
    // The mask reports what the filesystem actually answered; a filesystem
    // without birth time returns success with the bit clear.
    if stat.stx_mask & rustix::fs::StatxFlags::BTIME.bits() == 0 {
        return None;
    }
    Some(
        (i128::from(stat.stx_btime.tv_sec) * 1_000_000_000 + i128::from(stat.stx_btime.tv_nsec))
            .to_string(),
    )
}

#[cfg(not(target_os = "linux"))]
fn created_unix_nanos(file: &cap_std::fs::File) -> Option<String> {
    let created = file.metadata().ok()?.created().ok()?;
    Some(
        match created.duration_since(cap_std::time::SystemClock::UNIX_EPOCH) {
            Ok(duration) => duration.as_nanos().to_string(),
            Err(error) => format!("-{}", error.duration().as_nanos()),
        },
    )
}

fn version_from(
    path: &Path,
    file: &cap_std::fs::File,
    metadata: &cap_std::fs::Metadata,
    bytes: &[u8],
) -> FileVersion {
    let modified_unix_nanos = metadata.modified().ok().map(|modified| {
        match modified.duration_since(cap_std::time::SystemClock::UNIX_EPOCH) {
            Ok(duration) => duration.as_nanos().to_string(),
            Err(error) => format!("-{}", error.duration().as_nanos()),
        }
    });
    let created_unix_nanos = created_unix_nanos(file);
    let digest = Sha256::digest(bytes);
    FileVersion {
        canonical_path: path.to_string_lossy().into_owned(),
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
        modified_unix_nanos,
        created_unix_nanos,
        content_sha256: format!("{digest:x}"),
    }
}

fn stale(path: &Path) -> RuntimeError {
    RuntimeError::new(
        ErrorKind::Tool,
        format!(
            "`{}` changed since it was read; read it again before editing",
            path.display()
        ),
    )
}

fn workspace_io(action: &str, path: &Path, err: std::io::Error) -> RuntimeError {
    RuntimeError::new(
        ErrorKind::Tool,
        format!("cannot {action} `{}`: {err}", path.display()),
    )
}

impl Workspace for ProjectWorkspace {
    fn root(&self) -> &str {
        &self.display
    }

    fn contains(&self, path: &str) -> bool {
        self.resolve_lexically(path)
            .is_some_and(|resolved| resolved.starts_with(&self.root))
    }

    fn resolve(&self, path: &str) -> Result<String, RuntimeError> {
        let resolved = self.resolve_lexically(path).ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::Workspace,
                format!("path `{path}` could not be resolved"),
            )
        })?;
        if resolved.starts_with(&self.root) {
            Ok(resolved.to_string_lossy().into_owned())
        } else {
            Err(RuntimeError::new(
                ErrorKind::Workspace,
                format!("path `{path}` is outside the project `{}`", self.display),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> (tempfile::TempDir, ProjectWorkspace) {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::create_dir_all(dir.path().join("src")).expect("src");
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").expect("a file");
        let workspace = ProjectWorkspace::new(dir.path()).expect("a workspace");
        (dir, workspace)
    }

    #[test]
    fn existing_paths_inside_the_root_are_contained() {
        let (dir, workspace) = project();
        assert!(workspace.contains("src/main.rs"));
        assert!(workspace.contains(dir.path().join("src/main.rs").to_str().unwrap()));
    }

    #[test]
    fn a_new_file_inside_the_root_is_contained() {
        let (_dir, workspace) = project();
        assert!(workspace.contains("src/created_later.rs"));
        assert!(workspace.contains("does/not/exist/yet.txt"));
    }

    #[test]
    fn traversal_out_of_the_root_is_rejected() {
        let (_dir, workspace) = project();
        assert!(!workspace.contains("../escape.txt"));
        assert!(!workspace.contains("src/../../escape.txt"));
        assert!(!workspace.contains("/etc/passwd"));

        let err = workspace.resolve("../escape.txt").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Workspace);
    }

    #[test]
    fn traversal_through_a_missing_directory_is_rejected() {
        let (_dir, workspace) = project();
        // The middle directory does not exist, so the check cannot rely on
        // canonicalize alone.
        assert!(!workspace.contains("not_yet/../../escape.txt"));
    }

    #[test]
    fn resolve_returns_the_canonical_path() {
        let (dir, workspace) = project();
        let resolved = workspace.resolve("src/main.rs").expect("resolved");
        let expected = dir
            .path()
            .canonicalize()
            .unwrap()
            .join("src/main.rs")
            .to_string_lossy()
            .into_owned();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn a_missing_root_is_an_error_rather_than_an_empty_boundary() {
        let err = ProjectWorkspace::new("/definitely/not/a/real/project").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Workspace);
    }

    #[cfg(unix)]
    #[test]
    fn a_component_swapped_to_an_outside_symlink_cannot_escape() {
        use std::os::unix::fs::symlink;

        let (dir, workspace) = project();
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("secret.txt"), "outside").expect("secret");
        std::fs::rename(dir.path().join("src"), dir.path().join("src-old")).expect("swap old");
        symlink(outside.path(), dir.path().join("src")).expect("swap symlink");

        assert!(workspace.read_bounded("src/secret.txt", 1024).is_err());
        assert!(workspace.entries("src", true, true, 100).is_err());
        assert!(workspace.create_new("src/created.txt", b"no").is_err());
        assert!(!outside.path().join("created.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn replacement_preserves_mode_xattrs_acl_and_owner() {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let (dir, workspace) = project();
        let path = dir.path().join("script.sh");
        std::fs::write(&path, "#!/bin/sh\necho old\n").expect("script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o751)).expect("mode");
        xattr::set(&path, "user.smith-test", b"preserved").expect("xattr");
        let before_acl = exacl::getfacl(&path, None).expect("acl");
        let before = std::fs::metadata(&path).expect("metadata");
        let version = workspace.read_bounded(&path, 4096).expect("read").version;

        workspace
            .replace_if_version(&path, b"#!/bin/sh\necho new\n", &version, 4096)
            .expect("replace");

        let after = std::fs::metadata(&path).expect("metadata");
        assert_eq!(after.mode() & 0o7777, 0o751);
        assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
        assert_eq!(
            xattr::get(&path, "user.smith-test").expect("xattr"),
            Some(b"preserved".to_vec())
        );
        assert_eq!(exacl::getfacl(&path, None).expect("acl"), before_acl);
    }

    #[test]
    fn equal_mtime_changed_bytes_are_stale() {
        let (dir, workspace) = project();
        let path = dir.path().join("same.txt");
        std::fs::write(&path, "alpha\n").expect("seed");
        let version = workspace.read_bounded(&path, 1024).expect("read").version;
        let modified = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .expect("mtime");
        std::fs::write(&path, "bravo\n").expect("same-size rewrite");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_modified(modified))
            .expect("restore mtime");

        let err = workspace
            .replace_if_version(&path, b"smith\n", &version, 1024)
            .expect_err("the hash must catch an equal-mtime change");
        assert!(err.message.contains("changed since it was read"), "{err:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "bravo\n");
    }

    #[test]
    fn replaced_inode_is_stale_even_with_identical_bytes_and_mtime() {
        let (dir, workspace) = project();
        let path = dir.path().join("identity.txt");
        std::fs::write(&path, "same\n").expect("seed");
        let version = workspace.read_bounded(&path, 1024).expect("read").version;
        let modified = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .expect("mtime");
        std::fs::remove_file(&path).expect("remove old inode");
        // Linux hands out a freed inode number again immediately, so with the
        // bytes, size and mtime all restored below, birth time is the only
        // field left that separates the two objects. File timestamps advance
        // on the kernel tick rather than continuously, so a replacement made
        // inside one tick is genuinely indistinguishable -- and harmless,
        // since every byte matches what was read. Wait past the tick to
        // exercise the case that is detectable, which is also the only shape a
        // real edit-then-replace race takes.
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&path, "same\n").expect("replacement inode");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_modified(modified))
            .expect("restore mtime");

        assert!(
            workspace
                .replace_if_version(&path, b"smith\n", &version, 1024)
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "same\n");
    }

    #[test]
    fn delete_refuses_a_replaced_target_without_losing_it() {
        let (dir, workspace) = project();
        let path = dir.path().join("delete.txt");
        std::fs::write(&path, "old\n").expect("seed");
        let version = workspace.read_bounded(&path, 1024).expect("read").version;
        std::fs::write(&path, "user edit\n").expect("user edit");

        assert!(workspace.delete_if_version(&path, &version, 1024).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user edit\n");
    }

    #[test]
    fn metadata_copy_failure_is_fail_closed() {
        let (dir, workspace) = project();
        let original = workspace.directory.open("src/main.rs").expect("original");
        let mut metadata = preserved_metadata(&original).expect("metadata");
        metadata
            .xattrs
            .push((OsString::from("invalid\0xattr"), b"value".to_vec()));
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        let temporary = workspace
            .directory
            .open_with("metadata.tmp", &options)
            .expect("temporary");

        assert!(apply_metadata(&temporary, &metadata).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/main.rs")).unwrap(),
            "fn main() {}"
        );
    }

    #[test]
    fn durable_replace_orders_temp_sync_before_publish_and_parent_sync_after_cleanup() {
        let (dir, workspace) = project();
        let path = dir.path().join("durable.txt");
        std::fs::write(&path, "old\n").expect("seed");
        let version = workspace.read_bounded(&path, 1024).expect("read").version;

        workspace
            .replace_if_version(&path, b"new\n", &version, 1024)
            .expect("replace");

        assert_eq!(
            *workspace.commit_trace.lock().expect("trace"),
            [
                "temporary-synced",
                "published-exchange",
                "old-inode-removed",
                "parent-synced"
            ]
        );
    }
}
