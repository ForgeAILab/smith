//! Source parsing and fetching through the caller's git executable.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::manifest::component_path;
use crate::{PluginError, Result, validate_name};

/// A reproducible source recorded with an installed plugin or marketplace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginSource {
    /// A directory copied at preparation time.
    Local {
        /// Absolute source directory (canonicalised when fetched).
        path: PathBuf,
    },
    /// A shallow git checkout, optionally narrowed to one plugin directory.
    Git {
        /// Git URL, including local file URLs used by offline callers.
        url: String,
        /// Optional branch or tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
        /// Optional confined path inside the checkout.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subdirectory: Option<String>,
    },
    /// An entry resolved again through a registered marketplace on update.
    Marketplace {
        /// Entry name.
        plugin: String,
        /// Registered catalogue name.
        marketplace: String,
    },
}

impl PluginSource {
    /// Parses local paths, GitHub shorthand, git URLs, and name@marketplace.
    pub fn parse(source: &str) -> Result<Self> {
        if let Some(github) = source.strip_prefix("github:") {
            let (repo, reference) = split_ref(github);
            let parts: Vec<_> = repo.split('/').collect();
            if parts.len() != 2
                || parts.iter().any(|part| {
                    part.is_empty()
                        || matches!(*part, "." | "..")
                        || part
                            .chars()
                            .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
                })
            {
                return Err(PluginError::Invalid(
                    "GitHub source must be github:owner/repo[#ref]".into(),
                ));
            }
            return Ok(Self::Git {
                url: format!("https://github.com/{repo}.git"),
                reference,
                subdirectory: None,
            });
        }
        if source.starts_with("git@")
            || source.starts_with("https://")
            || source.starts_with("http://")
            || source.starts_with("git://")
            || source.starts_with("ssh://")
            || source.starts_with("file://")
        {
            let (url, reference) = split_ref(source);
            return Ok(Self::Git {
                url: url.into(),
                reference,
                subdirectory: None,
            });
        }
        if source.starts_with("./")
            || source.starts_with("../")
            || Path::new(source).is_absolute()
            || Path::new(source).is_dir()
        {
            return Ok(Self::Local {
                path: PathBuf::from(source),
            });
        }
        if let Some((plugin, marketplace)) = source.split_once('@') {
            validate_name(plugin)?;
            validate_name(marketplace)?;
            return Ok(Self::Marketplace {
                plugin: plugin.into(),
                marketplace: marketplace.into(),
            });
        }
        Err(PluginError::UnsupportedSource {
            kind: source.split(':').next().unwrap_or(source).into(),
        })
    }

    /// Branch or tag recorded separately for display.
    pub fn reference(&self) -> Option<&str> {
        match self {
            Self::Git { reference, .. } => reference.as_deref(),
            _ => None,
        }
    }
}

fn split_ref(source: &str) -> (&str, Option<String>) {
    match source.rsplit_once('#') {
        Some((url, reference)) => (url, Some(reference.into())),
        None => (source, None),
    }
}

/// A fetched directory whose temporary checkout remains alive until dropped.
#[derive(Debug)]
pub struct FetchedSource {
    /// Canonical plugin or marketplace directory.
    pub root: PathBuf,
    /// Full commit ID for a git checkout.
    pub resolved_commit: Option<String>,
    /// Source, with local paths made canonical.
    pub source: PluginSource,
    pub(crate) _temporary: Option<TempDir>,
}

/// Executes git without a shell; an injectable program makes absence testable.
#[derive(Debug, Clone)]
pub struct GitFetcher {
    program: OsString,
}

impl Default for GitFetcher {
    fn default() -> Self {
        Self::new("git")
    }
}

impl GitFetcher {
    /// Uses the named git executable (normally git on PATH).
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// Fetches a local directory or shallow clone; marketplace lookup is separate.
    pub fn fetch(&self, source: &PluginSource) -> Result<FetchedSource> {
        match source {
            PluginSource::Local { path } => {
                let root = path.canonicalize()?;
                if !root.is_dir() {
                    return Err(PluginError::Invalid("source must be a directory".into()));
                }
                Ok(FetchedSource {
                    source: PluginSource::Local { path: root.clone() },
                    root,
                    resolved_commit: None,
                    _temporary: None,
                })
            }
            PluginSource::Git {
                url,
                reference,
                subdirectory,
            } => {
                let temporary = tempfile::tempdir()?;
                let checkout = temporary.path().join("checkout");
                let mut clone = Command::new(&self.program);
                clone.args(["clone", "--depth", "1", "--no-hardlinks"]);
                if let Some(reference) = reference {
                    clone.args(["--branch", reference]);
                }
                clone.arg("--").arg(url).arg(&checkout);
                self.run(&mut clone)?;
                let mut revision = Command::new(&self.program);
                revision
                    .arg("-C")
                    .arg(&checkout)
                    .args(["rev-parse", "HEAD"]);
                let commit = self.run(&mut revision)?.trim().to_owned();
                let root = checkout.canonicalize()?;
                let root = match subdirectory {
                    Some(path) => component_path(
                        &root,
                        &format!("./{}", path.trim_start_matches("./")),
                        false,
                    )?,
                    None => root,
                };
                Ok(FetchedSource {
                    root,
                    source: source.clone(),
                    resolved_commit: Some(commit),
                    _temporary: Some(temporary),
                })
            }
            PluginSource::Marketplace { .. } => Err(PluginError::Invalid(
                "resolve marketplace entries through PluginStore".into(),
            )),
        }
    }

    fn run(&self, command: &mut Command) -> Result<String> {
        let output = command.output().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                PluginError::GitUnavailable {
                    program: self.program.to_string_lossy().into_owned(),
                }
            } else {
                PluginError::Io(error)
            }
        })?;
        if !output.status.success() {
            return Err(PluginError::Git(
                String::from_utf8_lossy(&output.stderr).trim().into(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

pub(crate) fn copy_directory(root: &Path, destination: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    copy(&root, &root, destination, &mut Vec::new())
}

fn copy(root: &Path, path: &Path, destination: &Path, parents: &mut Vec<PathBuf>) -> Result<()> {
    let resolved = crate::manifest::confined(root, path)?;
    if parents.contains(&resolved) {
        return Err(PluginError::Invalid(
            "symlink cycle in plugin directory".into(),
        ));
    }
    let metadata = fs::metadata(&resolved)?;
    if metadata.is_dir() {
        fs::create_dir(destination)?;
        parents.push(resolved.clone());
        let mut entries = fs::read_dir(&resolved)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            // A checkout's VCS metadata is not installed plugin content.
            if !crate::digest::vcs_metadata(&entry.file_name()) {
                copy(
                    root,
                    &entry.path(),
                    &destination.join(entry.file_name()),
                    parents,
                )?;
            }
        }
        parents.pop();
    } else if metadata.is_file() {
        fs::copy(&resolved, destination)?;
    } else {
        return Err(PluginError::Invalid(format!(
            "`{}` is not a regular file or directory",
            path.display()
        )));
    }
    Ok(())
}
