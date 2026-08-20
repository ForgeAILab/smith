//! Executable project trust.
//!
//! A project's `.smith/` directory is repository content: anyone who can land a
//! commit can put a hook, an extension, a credential helper, or a shell-valued
//! setting in front of whoever opens the project next. Reading the declarative
//! settings there is safe and happens before any prompt. *Running* something
//! from there is not, and requires that the user said yes to exactly this
//! content.
//!
//! "Exactly this content" is the load-bearing part. A decision binds the
//! canonical, symlink-resolved project path *and* the SHA-256 digest of the
//! artifact. Binding to the path alone would let an approved extension be
//! rewritten in the next commit and then run unattended; binding to the digest
//! alone would let a file carry its approval into an unrelated checkout. Either
//! half changing invalidates the decision, and the answer is
//! [`TrustStatus::Changed`] rather than a silent re-approval.
//!
//! This module runs nothing, reads no project configuration, and prompts for
//! nothing. It records decisions and reports them; asking a human belongs to
//! whichever surface has one. That keeps the policy pure, which is the only
//! way it can be tested exhaustively.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::credential::user_state_root;

/// The persisted trust file's shape version.
const SCHEMA_VERSION: u32 = 1;

/// The file, inside the user state root, that holds every decision.
const FILE_NAME: &str = "trust.json";

/// The kinds of project-supplied authority Smith will not exercise unasked.
///
/// Most variants name something Smith would *run*. [`ExecutableKind::Skill`]
/// names something Smith would *say*: a body the project supplies becomes part
/// of the instructions the model is steered by, which is authority exercised on
/// the project's behalf whether or not a process is spawned.
///
/// Plain declarative settings have no variant here on purpose: they carry no
/// execution, are readable before any decision, and gating them would make
/// Smith prompt for a model name. That reasoning stops at text Smith would
/// adopt as its own instructions, which is why skills are gated and a model
/// name is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutableKind {
    /// A project extension module the host would load and run.
    Extension,
    /// A lifecycle hook Smith would invoke around a turn or a tool.
    Hook,
    /// A program Smith would run to obtain a provider credential.
    CredentialHelper,
    /// A setting whose value Smith would run as a command.
    ShellSetting,
    /// A declared Model Context Protocol server Smith would spawn or dial.
    McpServer,
    /// A project-supplied skill body Smith would activate as privileged
    /// instructions.
    Skill,
}

impl ExecutableKind {
    /// The kind's stable name, as diagnostics and the persisted file spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Extension => "extension",
            Self::Hook => "hook",
            Self::CredentialHelper => "credential helper",
            Self::ShellSetting => "shell setting",
            Self::McpServer => "MCP server",
            Self::Skill => "skill",
        }
    }
}

impl fmt::Display for ExecutableKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A SHA-256 digest of executable content, in lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentDigest(String);

impl ContentDigest {
    /// Digests one span of content: a file's bytes, or a setting's text.
    pub fn of(content: &[u8]) -> Self {
        Self::of_parts([content])
    }

    /// Digests several spans as one artifact, so that an extension's module and
    /// its manifest are approved together and invalidate together.
    ///
    /// Each part is length-prefixed, so moving a byte across a part boundary
    /// changes the digest rather than leaving it unchanged.
    pub fn of_parts<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> Self {
        let mut hasher = Sha256::new();
        for part in parts {
            hasher.update((part.len() as u64).to_le_bytes());
            hasher.update(part);
        }
        Self(hex(&hasher.finalize()))
    }

    /// Digests a file's current content.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read, which is not a reason to proceed:
    /// content that cannot be identified cannot be trusted.
    pub fn of_file(path: &Path) -> Result<Self, RuntimeError> {
        let content = fs::read(path).map_err(|err| {
            RuntimeError::config(format!(
                "`{}` cannot be read, so its content cannot be identified: {err}",
                path.display()
            ))
        })?;
        Ok(Self::of(&content))
    }

    /// The lowercase hex form, which is what a consent prompt should show.
    pub fn as_hex(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One digest part, prefixed by what it is, so two parts of different kinds
/// cannot be swapped without changing the digest.
fn tagged(tag: &str, value: &str) -> Vec<u8> {
    let mut part = Vec::with_capacity(tag.len() + value.len() + 1);
    part.extend_from_slice(tag.as_bytes());
    part.push(b':');
    part.extend_from_slice(value.as_bytes());
    part
}

fn hex(bytes: &[u8]) -> String {
    use fmt::Write as _;

    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// One project-supplied artifact whose use requires the user's consent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Executable {
    kind: ExecutableKind,
    label: String,
    digest: ContentDigest,
}

impl Executable {
    /// Describes an artifact the caller has already digested — an extension
    /// spanning a module and a manifest, for instance.
    ///
    /// `label` identifies the artifact within its project across content
    /// changes, so that a rewritten extension is recognizably the same
    /// extension with a new digest rather than an unrelated one.
    pub fn new(kind: ExecutableKind, label: impl Into<String>, digest: ContentDigest) -> Self {
        Self {
            kind,
            label: label.into(),
            digest,
        }
    }

    /// Describes a file inside `project`, labelled by its project-relative path.
    ///
    /// # Errors
    ///
    /// Fails when either path cannot be canonicalized, and when the file
    /// resolves outside the project root: a symlink in `.smith/` aimed at a
    /// script elsewhere on the machine is not project content and must not
    /// inherit the project's trust.
    pub fn from_file(
        project: &Path,
        kind: ExecutableKind,
        path: &Path,
    ) -> Result<Self, RuntimeError> {
        let root = canonical_root(project)?;
        let resolved = path.canonicalize().map_err(|err| {
            RuntimeError::config(format!("`{}` is unusable: {err}", path.display()))
        })?;
        let relative = resolved.strip_prefix(&root).map_err(|_| {
            RuntimeError::new(
                ErrorKind::Workspace,
                format!(
                    "`{}` resolves outside the project `{}`, so the project's trust cannot cover it",
                    path.display(),
                    root.display()
                ),
            )
        })?;

        Ok(Self::new(
            kind,
            join_with_slashes(relative),
            ContentDigest::of_file(&resolved)?,
        ))
    }

    /// Describes a shell-valued setting, labelled by its configuration key.
    ///
    /// The command's text is digested, never stored: showing it belongs to the
    /// prompt, which already has the resolved configuration in hand.
    pub fn from_setting(key: impl Into<String>, command: &str) -> Self {
        Self::new(
            ExecutableKind::ShellSetting,
            key,
            ContentDigest::of(command.as_bytes()),
        )
    }

    /// Describes a declared MCP server by the local invocation Smith would
    /// perform, labelled by the server's name.
    ///
    /// The digest covers the *fully resolved* command, its arguments, and the
    /// **names** of the environment variables the server would be given. Three
    /// consequences, each deliberate:
    ///
    /// - Changing the command or an argument is a different program, so the
    ///   earlier decision authorizes nothing.
    /// - Rotating a credential changes no name, so a trusted server keeps
    ///   running. A value-derived digest would re-prompt on every rotation and
    ///   would be a secret-derived identity besides.
    /// - Adding a variable changes what the server can see, so it re-prompts
    ///   even when the command is untouched.
    ///
    /// Each part is tagged before it is length-prefixed, so an argument and a
    /// variable name spelled alike cannot be exchanged for one another.
    pub fn from_mcp_command<A, E>(
        name: impl Into<String>,
        command: &str,
        args: A,
        environment: E,
    ) -> Self
    where
        A: IntoIterator,
        A::Item: AsRef<str>,
        E: IntoIterator,
        E::Item: AsRef<str>,
    {
        let mut parts: Vec<Vec<u8>> = vec![b"stdio".to_vec(), tagged("command", command)];
        parts.extend(args.into_iter().map(|arg| tagged("arg", arg.as_ref())));
        parts.extend(
            environment
                .into_iter()
                .map(|variable| tagged("env", variable.as_ref())),
        );
        Self::new(
            ExecutableKind::McpServer,
            name,
            ContentDigest::of_parts(parts.iter().map(Vec::as_slice)),
        )
    }

    /// Describes a declared remote MCP server by the endpoint Smith would dial
    /// and the header names it would send.
    ///
    /// Header *values* are excluded for the same reason environment values are:
    /// a bearer token is a secret, and rotating one must not look like a
    /// changed server.
    pub fn from_mcp_endpoint<H>(name: impl Into<String>, url: &str, headers: H) -> Self
    where
        H: IntoIterator,
        H::Item: AsRef<str>,
    {
        let mut parts: Vec<Vec<u8>> = vec![b"http".to_vec(), tagged("url", url)];
        parts.extend(
            headers
                .into_iter()
                .map(|header| tagged("header", header.as_ref())),
        );
        Self::new(
            ExecutableKind::McpServer,
            name,
            ContentDigest::of_parts(parts.iter().map(Vec::as_slice)),
        )
    }

    /// What kind of authority this artifact carries.
    pub fn kind(&self) -> ExecutableKind {
        self.kind
    }

    /// The artifact's stable identity within its project.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The exact content a decision would be made about.
    pub fn digest(&self) -> &ContentDigest {
        &self.digest
    }
}

/// The answer a user gave about one artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustDecision {
    /// Smith may run this exact content in this project.
    Allow,
    /// Smith may not.
    Deny,
}

/// What the store knows about one artifact in one project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustStatus {
    /// The user approved exactly this content here.
    Trusted,
    /// Nothing has been decided about it.
    Untrusted,
    /// Something was decided about it, and its content has changed since. The
    /// earlier answer covers content that no longer exists, so it authorizes
    /// nothing.
    Changed,
    /// The user refused exactly this content.
    Denied,
}

impl TrustStatus {
    /// Whether Smith may run the artifact without asking again.
    ///
    /// Only [`TrustStatus::Trusted`] says yes, so a status added later is
    /// refused until it is deliberately allowed here.
    pub fn allows_execution(self) -> bool {
        matches!(self, Self::Trusted)
    }
}

/// One persisted decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustRecord {
    /// The kind of authority that was decided.
    pub kind: ExecutableKind,
    /// The artifact's identity within the project.
    pub label: String,
    /// The content the decision was made about.
    pub digest: ContentDigest,
    /// The answer.
    pub decision: TrustDecision,
}

/// The persisted file, kept separate from the in-memory store so its shape can
/// carry a version the store does not have to.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustFile {
    version: u32,
    #[serde(default)]
    projects: BTreeMap<String, Vec<TrustRecord>>,
}

/// Every executable-trust decision the user has made.
///
/// The root is injected rather than discovered, so a test never reads or writes
/// the developer's real `~/.smith`.
#[derive(Debug, Clone)]
pub struct TrustStore {
    path: PathBuf,
    projects: BTreeMap<String, Vec<TrustRecord>>,
}

impl TrustStore {
    /// Opens the store kept under `root`, which is `~/.smith` in production.
    ///
    /// Nothing is created until a decision is recorded: opening a project must
    /// not write to the user's state.
    ///
    /// # Errors
    ///
    /// Fails when a trust file exists but cannot be read or understood. That is
    /// deliberately loud: silently continuing with an empty store would either
    /// re-prompt for everything or, worse, look like a fresh machine.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, RuntimeError> {
        let path = root.into().join(FILE_NAME);
        let projects = match fs::read(&path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(err) => {
                return Err(RuntimeError::config(format!(
                    "the trust file `{}` cannot be read: {err}",
                    path.display()
                )));
            }
            Ok(bytes) => {
                let file: TrustFile = serde_json::from_slice(&bytes).map_err(|err| {
                    RuntimeError::config(format!(
                        "the trust file `{}` is not readable as trust decisions: {err}",
                        path.display()
                    ))
                })?;
                if file.version != SCHEMA_VERSION {
                    return Err(RuntimeError::config(format!(
                        "the trust file `{}` has version {}, which this Smith does not understand",
                        path.display(),
                        file.version
                    )));
                }
                file.projects
            }
        };

        Ok(Self { path, projects })
    }

    /// Opens the store under `~/.smith`.
    ///
    /// # Errors
    ///
    /// Fails when there is no home directory, or for the reasons
    /// [`TrustStore::open`] fails.
    pub fn discover() -> Result<Self, RuntimeError> {
        Self::open(user_state_root()?)
    }

    /// Where decisions are persisted.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What is known about `executable` in `project`.
    ///
    /// # Errors
    ///
    /// Fails when `project` cannot be canonicalized. A project root that cannot
    /// be resolved cannot be matched against a recorded one, and guessing would
    /// mean answering about a different directory.
    pub fn status(
        &self,
        project: &Path,
        executable: &Executable,
    ) -> Result<TrustStatus, RuntimeError> {
        let key = project_key(project)?;
        let Some(record) = self
            .projects
            .get(&key)
            .and_then(|records| find(records, executable))
        else {
            return Ok(TrustStatus::Untrusted);
        };

        if record.digest != executable.digest {
            return Ok(TrustStatus::Changed);
        }
        Ok(match record.decision {
            TrustDecision::Allow => TrustStatus::Trusted,
            TrustDecision::Deny => TrustStatus::Denied,
        })
    }

    /// Records `decision` about `executable` in `project`, replacing whatever
    /// was decided about the same artifact before.
    ///
    /// # Errors
    ///
    /// Fails when `project` cannot be canonicalized, or when the decision
    /// cannot be persisted. An unpersisted decision is not reported as made:
    /// the next run would have executed on the strength of it.
    pub fn record(
        &mut self,
        project: &Path,
        executable: &Executable,
        decision: TrustDecision,
    ) -> Result<(), RuntimeError> {
        let key = project_key(project)?;
        let record = TrustRecord {
            kind: executable.kind,
            label: executable.label.clone(),
            digest: executable.digest.clone(),
            decision,
        };
        let records = self.projects.entry(key).or_default();
        match records
            .iter_mut()
            .find(|existing| existing.kind == record.kind && existing.label == record.label)
        {
            Some(existing) => *existing = record,
            None => records.push(record),
        }
        self.persist()
    }

    /// Every decision recorded for `project`.
    ///
    /// # Errors
    ///
    /// Fails when `project` cannot be canonicalized.
    pub fn records(&self, project: &Path) -> Result<&[TrustRecord], RuntimeError> {
        let key = project_key(project)?;
        Ok(self
            .projects
            .get(&key)
            .map_or(&[][..], |records| records.as_slice()))
    }

    /// Forgets every decision recorded for `project`, so the next run asks
    /// again.
    ///
    /// # Errors
    ///
    /// Fails when `project` cannot be canonicalized, or when the change cannot
    /// be persisted.
    pub fn forget(&mut self, project: &Path) -> Result<(), RuntimeError> {
        let key = project_key(project)?;
        self.projects.remove(&key);
        self.persist()
    }

    /// Writes the whole file, then moves it into place.
    ///
    /// A partial trust file is worse than none — it would read as a set of
    /// decisions the user never made — so the rename is what publishes it.
    fn persist(&self) -> Result<(), RuntimeError> {
        let file = TrustFile {
            version: SCHEMA_VERSION,
            projects: self.projects.clone(),
        };
        let body = serde_json::to_vec_pretty(&file).map_err(|err| {
            RuntimeError::config(format!("trust decisions cannot be written: {err}"))
        })?;

        let parent = self.path.parent().ok_or_else(|| {
            RuntimeError::config(format!(
                "`{}` has no parent directory to write into",
                self.path.display()
            ))
        })?;
        let temporary = self.path.with_extension("json.writing");
        create_private_dir(parent)
            .and_then(|()| write_private(&temporary, &body))
            .and_then(|()| fs::rename(&temporary, &self.path))
            .map_err(|err| {
                RuntimeError::config(format!(
                    "trust decisions cannot be saved to `{}`: {err}",
                    self.path.display()
                ))
            })
    }
}

fn find<'a>(records: &'a [TrustRecord], executable: &Executable) -> Option<&'a TrustRecord> {
    records
        .iter()
        .find(|record| record.kind == executable.kind && record.label == executable.label)
}

/// The canonical project root, which is what a decision is bound to.
fn canonical_root(project: &Path) -> Result<PathBuf, RuntimeError> {
    project.canonicalize().map_err(|err| {
        RuntimeError::config(format!(
            "the project `{}` cannot be resolved: {err}",
            project.display()
        ))
    })
}

fn project_key(project: &Path) -> Result<String, RuntimeError> {
    Ok(canonical_root(project)?.to_string_lossy().into_owned())
}

/// A project-relative path in one stable spelling, so a record written on one
/// platform still matches on another.
fn join_with_slashes(relative: &Path) -> String {
    relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

/// Creates the file with owner-only permissions from the start, rather than
/// tightening them afterwards and leaving a window where they were not.
#[cfg(unix)]
fn write_private(path: &Path, body: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(body)?;
    file.sync_all()
}

#[cfg(not(unix))]
fn write_private(path: &Path, body: &[u8]) -> std::io::Result<()> {
    fs::write(path, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_is_stable_and_content_addressed() {
        assert_eq!(ContentDigest::of(b"hook"), ContentDigest::of(b"hook"));
        assert_ne!(ContentDigest::of(b"hook"), ContentDigest::of(b"hook "));
        assert_eq!(ContentDigest::of(b"").as_hex().len(), 64);
    }

    #[test]
    fn parts_cannot_be_rearranged_without_changing_the_digest() {
        // Without length prefixes these two would hash identically, and an
        // extension could move code from its manifest into its module while
        // keeping its approval.
        let split = ContentDigest::of_parts([b"ab".as_slice(), b"c".as_slice()]);
        let moved = ContentDigest::of_parts([b"a".as_slice(), b"bc".as_slice()]);
        assert_ne!(split, moved);
    }

    #[test]
    fn only_a_trusted_status_authorizes_execution() {
        assert!(TrustStatus::Trusted.allows_execution());
        for refused in [
            TrustStatus::Untrusted,
            TrustStatus::Changed,
            TrustStatus::Denied,
        ] {
            assert!(!refused.allows_execution(), "{refused:?}");
        }
    }

    #[test]
    fn a_shell_setting_is_identified_by_its_key_and_digested_by_its_command() {
        let setting = Executable::from_setting("hooks.pre_tool", "echo hi");
        assert_eq!(setting.kind(), ExecutableKind::ShellSetting);
        assert_eq!(setting.label(), "hooks.pre_tool");
        assert_eq!(setting.digest(), &ContentDigest::of(b"echo hi"));
        // The command itself is not carried anywhere a diagnostic could print.
        assert!(!format!("{setting:?}").contains("echo hi"));
    }
}
