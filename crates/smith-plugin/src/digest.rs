//! Portable file identities shared by source copies and installed verification.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use smith_config::trust::ContentDigest;

use crate::manifest::{confined, relative};
use crate::{PluginError, Result};

pub(crate) fn vcs_metadata(name: &OsStr) -> bool {
    [".git", ".hg", ".svn", ".bzr"]
        .iter()
        .any(|vcs| name == OsStr::new(vcs))
}

pub(crate) fn directory_digest(root: &Path, name: &str) -> Result<ContentDigest> {
    let mut files = BTreeMap::new();
    collect(root, root, &mut Vec::new(), &mut files)?;
    let mut parts = vec![b"smith-plugin-v2".to_vec(), name.as_bytes().to_vec()];
    for (path, bytes) in files {
        parts.push(path.into_bytes());
        parts.push(bytes);
    }
    Ok(ContentDigest::of_parts(parts.iter().map(Vec::as_slice)))
}

fn collect(
    root: &Path,
    path: &Path,
    parents: &mut Vec<PathBuf>,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let resolved = confined(root, path)?;
    let metadata = fs::metadata(&resolved)?;
    if metadata.is_dir() {
        if parents.contains(&resolved) {
            return Err(PluginError::Invalid(
                "symlink cycle in plugin directory".into(),
            ));
        }
        parents.push(resolved);
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if !vcs_metadata(&entry.file_name()) {
                collect(root, &entry.path(), parents, files)?;
            }
        }
        parents.pop();
    } else if metadata.is_file() {
        // Keep the logical path of a confined symlink: copying materializes it
        // there, so inspecting a source and its installed copy gives one hash.
        let relative = relative(root, path)?;
        if relative.contains('\\') {
            return Err(PluginError::Invalid(
                "plugin paths must use portable separators".into(),
            ));
        }
        files.insert(relative, fs::read(resolved)?);
    } else {
        return Err(PluginError::Invalid(format!(
            "`{}` is not a regular file or directory",
            path.display()
        )));
    }
    Ok(())
}
