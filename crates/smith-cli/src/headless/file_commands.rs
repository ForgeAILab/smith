//! Prompt-template expansion before unattended host startup.

use std::path::Path;

use anyhow::{Context, Result};
use smith_client::file_commands::{CommandCatalog, InvocationRefusal, validate_name};
use smith_config::trust::TrustStore;

/// Discovers and expands commands before constructing the unattended host.
pub(crate) fn prepare_prompt(prompt: &str, user_root: &Path, project: &Path) -> Result<String> {
    let trust = TrustStore::open(user_root)
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("reading command trust")?;
    let catalog = CommandCatalog::discover(Some(user_root), Some(project), &trust);
    Ok(expand_prompt(prompt, &catalog, Some(project), &trust)?)
}

fn expand_prompt(
    prompt: &str,
    catalog: &CommandCatalog,
    project: Option<&Path>,
    trust: &TrustStore,
) -> Result<String, InvocationRefusal> {
    if prompt.starts_with("//") {
        return Ok(prompt[1..].to_owned());
    }
    let Some(text) = prompt.strip_prefix('/') else {
        return Ok(prompt.to_owned());
    };
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let name = &text[..end];
    if validate_name(name).is_err() || catalog.resolve(name).is_none() {
        return Ok(prompt.to_owned());
    }
    let arguments = text[end..].trim_start();
    catalog
        .prepare(name, arguments, project, trust)
        .map(|prepared| prepared.prompt)
}

#[cfg(test)]
mod tests {
    use super::{CommandCatalog, Path, TrustStore, expand_prompt};

    fn user_catalog(root: &Path, trust: &TrustStore) -> CommandCatalog {
        std::fs::create_dir_all(root.join("commands")).expect("commands directory");
        std::fs::write(root.join("commands/audit.md"), "Audit $ARGUMENTS for bugs.")
            .expect("audit command");
        CommandCatalog::discover(Some(root), None, trust)
    }

    #[test]
    fn reserved_command_files_are_not_expanded() {
        let root = tempfile::tempdir().expect("user root");
        let trust = TrustStore::open(root.path()).expect("trust store");
        std::fs::create_dir_all(root.path().join("commands")).expect("commands directory");
        std::fs::write(
            root.path().join("commands/review.md"),
            "Review $ARGUMENTS for bugs.",
        )
        .expect("reserved review command");
        let catalog = CommandCatalog::discover(Some(root.path()), None, &trust);
        assert_eq!(
            expand_prompt("/review x", &catalog, None, &trust).unwrap(),
            "/review x"
        );
    }

    #[test]
    fn unrelated_prompts_are_preserved_byte_for_byte() {
        let root = tempfile::tempdir().expect("user root");
        let trust = TrustStore::open(root.path()).expect("trust store");
        let catalog = user_catalog(root.path(), &trust);
        let long_name = format!("/{} args", "r".repeat(65));
        for prompt in [
            "",
            "/",
            "/ args",
            "/usr/bin/env is missing",
            "/model",
            "/help",
            "/commands",
            "/unknown args\nnext line  ",
            "/Audit args",
            "/audit_name args",
            "/áudit args",
            " /audit args",
            "\n/audit args",
            "ordinary text\n  ",
            long_name.as_str(),
        ] {
            assert_eq!(
                expand_prompt(prompt, &catalog, None, &trust).unwrap(),
                prompt
            );
        }
    }

    #[test]
    fn literal_slash_escape_removes_exactly_one_slash() {
        let root = tempfile::tempdir().expect("user root");
        let trust = TrustStore::open(root.path()).expect("trust store");
        let catalog = user_catalog(root.path(), &trust);
        for (prompt, expected) in [
            ("//audit the plan", "/audit the plan"),
            ("//", "/"),
            ("///audit  args\nnext", "//audit  args\nnext"),
        ] {
            assert_eq!(
                expand_prompt(prompt, &catalog, None, &trust).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn arguments_skip_only_the_separator_and_preserve_internal_newlines() {
        let root = tempfile::tempdir().expect("user root");
        let trust = TrustStore::open(root.path()).expect("trust store");
        let catalog = user_catalog(root.path(), &trust);
        for (prompt, expected) in [
            ("/audit", "Audit  for bugs."),
            ("/audit \t\n", "Audit  for bugs."),
            (
                "/audit\t \nfirst\n  second",
                "Audit first\n  second for bugs.",
            ),
            ("/audit\u{2003}src/lib.rs", "Audit src/lib.rs for bugs."),
            ("/audit $ARGUMENTS", "Audit $ARGUMENTS for bugs."),
        ] {
            assert_eq!(
                expand_prompt(prompt, &catalog, None, &trust).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn project_command_changed_after_discovery_is_refused() {
        use smith_config::trust::{Executable, ExecutableKind, TrustDecision};

        let root = tempfile::tempdir().expect("user root");
        let project = tempfile::tempdir().expect("project");
        let commands = project.path().join(".smith/commands");
        std::fs::create_dir_all(&commands).expect("commands directory");
        let path = commands.join("audit.md");
        std::fs::write(&path, "original").expect("command");
        let mut trust = TrustStore::open(root.path()).expect("trust store");
        let executable = Executable::from_file(project.path(), ExecutableKind::SlashCommand, &path)
            .expect("executable");
        trust
            .record(project.path(), &executable, TrustDecision::Allow)
            .expect("approval");
        let catalog = CommandCatalog::discover(Some(root.path()), Some(project.path()), &trust);
        std::fs::write(&path, "rewritten").expect("edited command");
        let refusal = expand_prompt("/audit", &catalog, Some(project.path()), &trust)
            .expect_err("changed command must not run");
        assert!(refusal.to_string().contains("content changed"));
        assert!(refusal.to_string().contains("/commands trust audit"));
    }
}
