//! Bounded Git inspection for Smith's local change commands.
//!
//! Commands are executed directly as `git` subcommands rather than through a
//! shell, so user aliases and shell expansion cannot change their meaning.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use sha2::{Digest, Sha256};

const MAX_PATCH_BYTES: usize = 512 * 1024;
const MAX_UNTRACKED_FILE_BYTES: u64 = 128 * 1024;

/// A Git-backed project checkout.
#[derive(Debug, Clone)]
pub struct GitChanges {
    project: PathBuf,
    root: PathBuf,
}

/// A bounded inspection result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeView {
    /// Human-readable view title.
    pub title: String,
    /// Patch or structured unavailable/empty content.
    pub content: String,
}

/// A stale-safe selective revert preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevertPreview {
    /// User-facing title.
    pub title: String,
    /// Exact reverse patch.
    pub content: String,
    /// Original scope string.
    pub scope: String,
    /// Fingerprint checked again immediately before mutation.
    pub fingerprint: String,
    /// Origin label; unknown unless an external change ledger proves more.
    pub origin: &'static str,
}

/// Images needed to make a successful revert recoverable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedRevert {
    /// Canonical affected path.
    pub path: PathBuf,
    /// Image before revert.
    pub before: Option<Vec<u8>>,
    /// Image after revert.
    pub after: Option<Vec<u8>>,
    /// Recovery copy created before an untracked file was removed.
    pub recovery_path: Option<PathBuf>,
}

impl GitChanges {
    /// Discovers the enclosing Git worktree without initializing one.
    pub fn discover(project: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let project = project.as_ref().canonicalize().map_err(io_error)?;
        let output = git_output(&project, ["rev-parse", "--show-toplevel"])?;
        if !output.status.success() {
            return Err(unavailable(
                "Git-backed change inspection is unavailable outside a Git worktree",
            ));
        }
        let root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        let root = root.canonicalize().map_err(io_error)?;
        if !project.starts_with(&root) {
            return Err(unavailable("Git reported a worktree outside the project"));
        }
        Ok(Self { project, root })
    }

    /// The repository root Git resolved.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns a short, local status summary.
    pub fn status_summary(&self) -> Result<String, RuntimeError> {
        let output = self.git(["status", "--short", "--untracked-files=all"])?;
        ensure_success(output, "read Git status").map(|text| {
            if text.trim().is_empty() {
                "clean".to_owned()
            } else {
                format!("{} changed path(s)", text.lines().count())
            }
        })
    }

    /// Returns the current branch, or a bounded detached-HEAD label.
    pub fn branch_label(&self) -> Result<String, RuntimeError> {
        let symbolic = self.git(["symbolic-ref", "--short", "--quiet", "HEAD"])?;
        if symbolic.status.success() {
            let branch = bounded_git_label(&String::from_utf8_lossy(&symbolic.stdout));
            if !branch.is_empty() {
                return Ok(branch);
            }
        }
        let detached = self.git(["rev-parse", "--short=12", "HEAD"])?;
        let revision = ensure_success(detached, "read current Git revision")?;
        Ok(format!("detached@{}", bounded_git_label(&revision)))
    }

    /// Hashes one current project-relative path, including absence.
    pub fn path_hash(&self, path: &str) -> Result<String, RuntimeError> {
        validate_relative_path(path)?;
        let image = read_optional(&self.root.join(path))?;
        Ok(fingerprint(path, image.as_deref(), b"path-image"))
    }

    /// Resolves the merge base between `HEAD` and another revision.
    pub fn merge_base(&self, revision: &str) -> Result<String, RuntimeError> {
        validate_revision(revision)?;
        let output = self.git(["merge-base", "HEAD", revision])?;
        ensure_success(output, "resolve merge base").map(|base| base.trim().to_owned())
    }

    /// Inspects one supported scope.
    pub fn inspect(&self, scope: Option<&str>) -> Result<ChangeView, RuntimeError> {
        let scope = scope.unwrap_or("all");
        let (title, mut content) = match scope {
            "all" => (
                "diff · all uncommitted".to_owned(),
                self.diff(["diff", "--no-ext-diff", "--binary", "HEAD", "--", "."])?,
            ),
            "staged" => (
                "diff · staged".to_owned(),
                self.diff(["diff", "--no-ext-diff", "--binary", "--cached", "--", "."])?,
            ),
            "unstaged" => (
                "diff · unstaged".to_owned(),
                self.diff(["diff", "--no-ext-diff", "--binary", "--", "."])?,
            ),
            "untracked" => ("diff · untracked".to_owned(), self.untracked_patch(None)?),
            "last-turn" => {
                return Err(unavailable(
                    "last-turn diff is unavailable because this session has no attributable change set",
                ));
            }
            revision if revision.starts_with("commit:") => {
                let revision = revision.trim_start_matches("commit:");
                validate_revision(revision)?;
                (
                    format!("diff · commit {revision}"),
                    self.diff([
                        "show",
                        "--format=fuller",
                        "--no-ext-diff",
                        "--binary",
                        revision,
                    ])?,
                )
            }
            revision if revision.starts_with("base:") => {
                let revision = revision.trim_start_matches("base:");
                let base = self.merge_base(revision)?;
                (
                    format!("diff · {revision}...HEAD"),
                    self.diff([
                        "diff",
                        "--no-ext-diff",
                        "--binary",
                        &base,
                        "HEAD",
                        "--",
                        ".",
                    ])?,
                )
            }
            path => {
                let (path, hunk) = parse_revert_scope(path)?;
                let mut tracked =
                    self.diff(["diff", "--no-ext-diff", "--binary", "HEAD", "--", path])?;
                if let Some(index) = hunk {
                    tracked = select_hunk(&tracked, index)?;
                }
                let untracked = self.untracked_patch(Some(path))?;
                let suffix = hunk.map_or_else(String::new, |index| format!("#{index}"));
                (
                    format!("diff · {path}{suffix}"),
                    format!("{tracked}{untracked}"),
                )
            }
        };

        if scope == "all" {
            content.push_str(&self.untracked_patch(None)?);
        }
        if content.trim().is_empty() {
            content = "No changes in this scope.".to_owned();
        }
        Ok(ChangeView { title, content })
    }

    /// Previews one exact file or numbered hunk (`path#N`) for `/revert`.
    pub fn preview_revert(&self, scope: &str) -> Result<RevertPreview, RuntimeError> {
        let (path, hunk) = parse_revert_scope(scope)?;
        let absolute = self.root.join(path);
        let before = read_optional(&absolute)?;
        let untracked = self.is_untracked(path)?;
        let patch = if untracked {
            if hunk.is_some() {
                return Err(unavailable(
                    "untracked files can be reverted only as a whole file",
                ));
            }
            reverse_untracked_patch(&self.untracked_patch(Some(path))?)
        } else {
            let full = self.diff([
                "diff",
                "-R",
                "--no-ext-diff",
                "--binary",
                "HEAD",
                "--",
                path,
            ])?;
            if full.trim().is_empty() {
                return Err(unavailable(format!(
                    "`{path}` has no current change to revert"
                )));
            }
            if full.contains("GIT binary patch") || full.contains("Binary files") {
                return Err(unavailable(
                    "binary changes are visible in /diff but cannot be selectively reverted",
                ));
            }
            match hunk {
                Some(index) => select_hunk(&full, index)?,
                None => full,
            }
        };
        let fingerprint = fingerprint(scope, before.as_deref(), patch.as_bytes());
        Ok(RevertPreview {
            title: format!("revert · {scope}"),
            content: format!("origin: unknown\n\n{patch}"),
            scope: scope.to_owned(),
            fingerprint,
            origin: "unknown",
        })
    }

    /// Applies a previously previewed file/hunk only if its fingerprint is
    /// still current.
    pub fn apply_revert(
        &self,
        scope: &str,
        expected_fingerprint: &str,
        recovery_dir: Option<&Path>,
    ) -> Result<AppliedRevert, RuntimeError> {
        let preview = self.preview_revert(scope)?;
        if preview.fingerprint != expected_fingerprint {
            return Err(unavailable(
                "revert refused because the selected change moved after preview",
            ));
        }
        let (path, _) = parse_revert_scope(scope)?;
        let absolute = self.root.join(path);
        let before = read_optional(&absolute)?;
        if self.is_untracked(path)? {
            let recovery_dir = recovery_dir.ok_or_else(|| {
                unavailable("untracked revert requires enabled session recovery storage")
            })?;
            std::fs::create_dir_all(recovery_dir).map_err(io_error)?;
            let destination = recovery_dir.join(format!(
                "{}-{}",
                fingerprint(scope, before.as_deref(), b"recovery"),
                Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("file")
            ));
            std::fs::rename(&absolute, &destination).map_err(io_error)?;
            return Ok(AppliedRevert {
                path: absolute,
                before,
                after: None,
                recovery_path: Some(destination),
            });
        }

        let patch = preview
            .content
            .split_once("\n\n")
            .map_or(preview.content.as_str(), |(_, patch)| patch);
        self.apply_patch(patch, false)?;
        let after = read_optional(&absolute)?;
        Ok(AppliedRevert {
            path: absolute,
            before,
            after,
            recovery_path: None,
        })
    }

    fn diff<const N: usize>(&self, args: [&str; N]) -> Result<String, RuntimeError> {
        let output = self.git(args)?;
        let text = ensure_success(output, "inspect Git changes")?;
        Ok(bound_patch(text))
    }

    fn untracked_patch(&self, selected: Option<&str>) -> Result<String, RuntimeError> {
        let output = self.git([
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            selected.unwrap_or("."),
        ])?;
        let bytes = if output.status.success() {
            output.stdout
        } else {
            return Err(command_error("list untracked files", &output));
        };
        let mut patch = String::new();
        for raw in bytes
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            let path = String::from_utf8_lossy(raw);
            let absolute = self.root.join(path.as_ref());
            let metadata = std::fs::symlink_metadata(&absolute).map_err(io_error)?;
            patch.push_str(&format!(
                "diff --git a/{path} b/{path}\nnew file mode {:06o}\n",
                file_mode(&metadata)
            ));
            if metadata.len() > MAX_UNTRACKED_FILE_BYTES {
                patch.push_str(&format!(
                    "Smith: untracked file is oversized ({} bytes); content omitted\n",
                    metadata.len()
                ));
                continue;
            }
            let bytes = std::fs::read(&absolute).map_err(io_error)?;
            if bytes.contains(&0) {
                patch.push_str("Binary files /dev/null and b/file differ\n");
                continue;
            }
            patch.push_str("--- /dev/null\n");
            patch.push_str(&format!("+++ b/{path}\n"));
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines() {
                patch.push('+');
                patch.push_str(line);
                patch.push('\n');
            }
        }
        Ok(bound_patch(patch))
    }

    fn git<const N: usize>(&self, args: [&str; N]) -> Result<Output, RuntimeError> {
        git_output(&self.project, args)
    }

    fn is_untracked(&self, path: &str) -> Result<bool, RuntimeError> {
        let output = self.git(["ls-files", "--error-unmatch", "--", path])?;
        Ok(!output.status.success() && self.root.join(path).exists())
    }

    fn apply_patch(&self, patch: &str, reverse: bool) -> Result<(), RuntimeError> {
        for check in [true, false] {
            let mut command = Command::new("git");
            command
                .arg("-C")
                .arg(&self.project)
                .args(["apply", "--whitespace=nowarn"])
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_TERMINAL_PROMPT", "0")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if reverse {
                command.arg("--reverse");
            }
            if check {
                command.arg("--check");
            }
            let mut child = command.spawn().map_err(io_error)?;
            child
                .stdin
                .take()
                .ok_or_else(|| unavailable("Git patch input was unavailable"))?
                .write_all(patch.as_bytes())
                .map_err(io_error)?;
            let output = child.wait_with_output().map_err(io_error)?;
            if !output.status.success() {
                return Err(command_error(
                    if check {
                        "validate the selected reverse patch"
                    } else {
                        "apply the selected reverse patch"
                    },
                    &output,
                ));
            }
        }
        Ok(())
    }
}

fn bounded_git_label(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(80)
        .collect()
}

fn parse_revert_scope(scope: &str) -> Result<(&str, Option<usize>), RuntimeError> {
    let (path, hunk) = match scope.rsplit_once('#') {
        Some((path, number))
            if !path.is_empty() && number.chars().all(|ch| ch.is_ascii_digit()) =>
        {
            let index = number
                .parse::<usize>()
                .map_err(|_| unavailable("hunk number is invalid"))?;
            if index == 0 {
                return Err(unavailable("hunk numbers start at 1"));
            }
            (path, Some(index))
        }
        _ => (scope, None),
    };
    if path.is_empty() {
        return Err(unavailable("usage: /revert FILE or /revert FILE#HUNK"));
    }
    validate_relative_path(path)?;
    Ok((path, hunk))
}

fn select_hunk(patch: &str, wanted: usize) -> Result<String, RuntimeError> {
    let lines = patch.lines().collect::<Vec<_>>();
    let first_hunk = lines
        .iter()
        .position(|line| line.starts_with("@@ "))
        .ok_or_else(|| unavailable("selected file has no textual hunks"))?;
    let starts = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.starts_with("@@ ").then_some(index))
        .collect::<Vec<_>>();
    let Some(&start) = starts.get(wanted - 1) else {
        return Err(unavailable(format!(
            "hunk {wanted} does not exist; this file has {} hunk(s)",
            starts.len()
        )));
    };
    let end = starts.get(wanted).copied().unwrap_or(lines.len());
    let mut selected = lines[..first_hunk].join("\n");
    selected.push('\n');
    selected.push_str(&lines[start..end].join("\n"));
    selected.push('\n');
    Ok(selected)
}

fn reverse_untracked_patch(patch: &str) -> String {
    let mut output = Vec::new();
    let mut skipped_null_header = false;
    for line in patch.lines() {
        if line.starts_with("new file mode ") {
            output.push(line.replacen("new file mode", "deleted file mode", 1));
        } else if line == "--- /dev/null" {
            skipped_null_header = true;
        } else if skipped_null_header {
            if let Some(path) = line.strip_prefix("+++ b/") {
                output.push(format!("--- a/{path}"));
                output.push("+++ /dev/null".to_owned());
                skipped_null_header = false;
            }
        } else if let Some(added) = line.strip_prefix('+') {
            output.push(format!("-{added}"));
        } else {
            output.push(line.to_owned());
        }
    }
    output.join("\n") + "\n"
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, RuntimeError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(error)),
    }
}

fn fingerprint(scope: &str, image: Option<&[u8]>, patch: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(scope.as_bytes());
    digest.update([0]);
    match image {
        Some(image) => digest.update(image),
        None => digest.update(b"[absent]"),
    }
    digest.update([0]);
    digest.update(patch);
    format!("{:x}", digest.finalize())
}

fn git_output<const N: usize>(directory: &Path, args: [&str; N]) -> Result<Output, RuntimeError> {
    Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(io_error)
}

fn ensure_success(output: Output, action: &str) -> Result<String, RuntimeError> {
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(command_error(action, &output))
    }
}

fn command_error(action: &str, output: &Output) -> RuntimeError {
    let detail = String::from_utf8_lossy(&output.stderr);
    unavailable(format!("{action} failed: {}", detail.trim()))
}

fn validate_relative_path(path: &str) -> Result<(), RuntimeError> {
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(unavailable("change scope must be a project-relative path"));
    }
    Ok(())
}

fn validate_revision(revision: &str) -> Result<(), RuntimeError> {
    if revision.is_empty()
        || revision.starts_with('-')
        || revision.chars().any(char::is_whitespace)
        || revision.contains(['\0', '\n', '\r'])
    {
        return Err(unavailable("Git revision is not valid"));
    }
    Ok(())
}

fn bound_patch(mut patch: String) -> String {
    if patch.len() <= MAX_PATCH_BYTES {
        return patch;
    }
    let mut end = MAX_PATCH_BYTES;
    while !patch.is_char_boundary(end) {
        end -= 1;
    }
    patch.truncate(end);
    patch.push_str("\nSmith: patch truncated at the 512 KiB display limit.\n");
    patch
}

#[cfg(unix)]
fn file_mode(metadata: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o111 == 0 {
        0o100644
    } else {
        0o100755
    }
}

#[cfg(not(unix))]
fn file_mode(_metadata: &std::fs::Metadata) -> u32 {
    0o100644
}

fn io_error(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::new(ErrorKind::Workspace, error.to_string())
}

fn unavailable(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(ErrorKind::Workspace, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success());
    }

    fn repository() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp");
        git(dir.path(), &["init", "-q"]);
        git(
            dir.path(),
            &["config", "user.email", "smith@example.invalid"],
        );
        git(dir.path(), &["config", "user.name", "Smith Test"]);
        std::fs::write(dir.path().join("tracked.txt"), "before\n").expect("write");
        git(dir.path(), &["add", "tracked.txt"]);
        git(dir.path(), &["commit", "-qm", "initial"]);
        dir
    }

    #[test]
    fn all_scope_includes_tracked_and_untracked_content() {
        let dir = repository();
        std::fs::write(dir.path().join("tracked.txt"), "after\n").expect("write");
        std::fs::write(dir.path().join("new.txt"), "new\n").expect("write");
        let changes = GitChanges::discover(dir.path()).expect("repo");
        let view = changes.inspect(None).expect("diff");
        assert!(view.content.contains("-before"), "{}", view.content);
        assert!(view.content.contains("+new"), "{}", view.content);
    }

    #[test]
    fn outside_git_fails_without_initializing_a_repository() {
        let dir = tempfile::tempdir().expect("temp");
        let error = GitChanges::discover(dir.path()).unwrap_err();
        assert!(error.message.contains("outside a Git worktree"));
        assert!(!dir.path().join(".git").exists());
    }

    #[test]
    fn parent_traversal_is_refused() {
        let dir = repository();
        let changes = GitChanges::discover(dir.path()).expect("repo");
        assert!(changes.inspect(Some("../outside")).is_err());
    }

    #[test]
    fn tracked_file_revert_leaves_unselected_changes_untouched() {
        let dir = repository();
        std::fs::write(dir.path().join("tracked.txt"), "after\n").expect("write");
        std::fs::write(dir.path().join("other.txt"), "other\n").expect("other");
        let changes = GitChanges::discover(dir.path()).expect("repo");
        let preview = changes.preview_revert("tracked.txt").expect("preview");
        let applied = changes
            .apply_revert("tracked.txt", &preview.fingerprint, None)
            .expect("apply");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("tracked.txt")).expect("tracked"),
            "before\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("other.txt")).expect("other"),
            "other\n"
        );
        assert_eq!(applied.before.as_deref(), Some(&b"after\n"[..]));
        assert_eq!(applied.after.as_deref(), Some(&b"before\n"[..]));
    }

    #[test]
    fn stale_revert_preview_fails_closed() {
        let dir = repository();
        std::fs::write(dir.path().join("tracked.txt"), "after\n").expect("write");
        let changes = GitChanges::discover(dir.path()).expect("repo");
        let preview = changes.preview_revert("tracked.txt").expect("preview");
        std::fs::write(dir.path().join("tracked.txt"), "user edit\n").expect("edit");
        assert!(
            changes
                .apply_revert("tracked.txt", &preview.fingerprint, None)
                .unwrap_err()
                .message
                .contains("moved after preview")
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("tracked.txt")).expect("read"),
            "user edit\n"
        );
    }

    #[test]
    fn one_selected_hunk_reverts_without_touching_the_other() {
        let dir = repository();
        let original = (1..=12)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        std::fs::write(dir.path().join("tracked.txt"), &original).expect("original");
        git(dir.path(), &["add", "tracked.txt"]);
        git(dir.path(), &["commit", "-qm", "long file"]);
        let changed = original
            .replace("line 2\n", "line two\n")
            .replace("line 11\n", "line eleven\n");
        std::fs::write(dir.path().join("tracked.txt"), changed).expect("changed");
        let changes = GitChanges::discover(dir.path()).expect("repo");
        let preview = changes.preview_revert("tracked.txt#1").expect("preview");
        changes
            .apply_revert("tracked.txt#1", &preview.fingerprint, None)
            .expect("apply");
        let current = std::fs::read_to_string(dir.path().join("tracked.txt")).expect("read");
        assert!(current.contains("line 2\n"), "{current}");
        assert!(current.contains("line eleven\n"), "{current}");
    }

    #[test]
    fn untracked_revert_moves_the_file_to_recovery_storage() {
        let dir = repository();
        let path = dir.path().join("new.txt");
        std::fs::write(&path, "recover me\n").expect("write");
        let recovery = dir.path().join(".smith-test-recovery");
        let changes = GitChanges::discover(dir.path()).expect("repo");
        let preview = changes.preview_revert("new.txt").expect("preview");
        let applied = changes
            .apply_revert("new.txt", &preview.fingerprint, Some(&recovery))
            .expect("apply");
        assert!(!path.exists());
        assert_eq!(applied.after, None);
        assert!(
            std::fs::read_dir(recovery)
                .expect("recovery")
                .next()
                .is_some()
        );
    }

    #[test]
    fn mixed_staged_and_unstaged_scopes_remain_distinct() {
        let dir = repository();
        std::fs::write(dir.path().join("tracked.txt"), "staged\n").expect("staged");
        git(dir.path(), &["add", "tracked.txt"]);
        std::fs::write(dir.path().join("tracked.txt"), "unstaged\n").expect("unstaged");
        let changes = GitChanges::discover(dir.path()).expect("repo");
        let staged = changes.inspect(Some("staged")).expect("staged view");
        let unstaged = changes.inspect(Some("unstaged")).expect("unstaged view");
        assert!(staged.content.contains("+staged"), "{}", staged.content);
        assert!(
            unstaged.content.contains("+unstaged"),
            "{}",
            unstaged.content
        );
    }

    #[test]
    fn oversized_and_binary_untracked_files_are_named_without_dumping_content() {
        let dir = repository();
        std::fs::write(dir.path().join("binary.bin"), [0, 1, 2]).expect("binary");
        std::fs::write(
            dir.path().join("large.txt"),
            vec![b'x'; usize::try_from(MAX_UNTRACKED_FILE_BYTES + 1).expect("size")],
        )
        .expect("large");
        let changes = GitChanges::discover(dir.path()).expect("repo");
        let view = changes.inspect(Some("untracked")).expect("view");
        assert!(view.content.contains("Binary files"), "{}", view.content);
        assert!(view.content.contains("oversized"), "{}", view.content);
    }

    #[test]
    fn a_deleted_tracked_file_can_be_restored() {
        let dir = repository();
        std::fs::remove_file(dir.path().join("tracked.txt")).expect("remove");
        let changes = GitChanges::discover(dir.path()).expect("repo");
        let preview = changes.preview_revert("tracked.txt").expect("preview");
        let applied = changes
            .apply_revert("tracked.txt", &preview.fingerprint, None)
            .expect("restore");
        assert_eq!(applied.before, None);
        assert_eq!(applied.after.as_deref(), Some(&b"before\n"[..]));
    }
}
