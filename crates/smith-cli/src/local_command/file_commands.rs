//! Host-owned file-command discovery, invocation, and trust decisions.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use smith_client::commands::CommandsAction;
use smith_client::commands_report::CommandsReport;
use smith_client::file_commands::{CommandCatalog, CommandLayer};
use smith_client::local_result::LocalResult;
use smith_config::trust::{Executable, ExecutableKind, TrustDecision, TrustStatus, TrustStore};
use smith_tui::app::{App, PreparedSubmission};

use super::CommandReport;

/// The interactive host retains the catalog and its discovery roots.
#[derive(Debug)]
pub(crate) struct CommandContext {
    user_root: PathBuf,
    project: PathBuf,
    catalog: Mutex<CommandCatalog>,
}

impl CommandContext {
    pub(crate) fn discover(user_root: &Path, project: &Path) -> Result<Self, String> {
        let trust = TrustStore::open(user_root).map_err(|error| error.message)?;
        Ok(Self {
            user_root: user_root.to_path_buf(),
            project: project.to_path_buf(),
            catalog: Mutex::new(CommandCatalog::discover(
                Some(user_root),
                Some(project),
                &trust,
            )),
        })
    }

    pub(crate) fn catalog(&self) -> CommandCatalog {
        self.catalog.lock().expect("command catalog").clone()
    }

    fn trust_store(&self) -> Result<TrustStore, String> {
        TrustStore::open(&self.user_root).map_err(|error| error.message)
    }

    pub(crate) fn reload(&self) -> Result<CommandCatalog, String> {
        let trust = self.trust_store()?;
        let catalog = CommandCatalog::discover(Some(&self.user_root), Some(&self.project), &trust);
        *self.catalog.lock().expect("command catalog") = catalog.clone();
        Ok(catalog)
    }

    /// File reads and trust checks happen here, before ordinary input handling.
    pub(crate) fn prepare(
        &self,
        app: &App,
        typed: String,
        name: &str,
        arguments: &str,
    ) -> Result<PreparedSubmission, String> {
        let trust = self.trust_store()?;
        let prepared = self
            .catalog()
            .prepare(name, arguments, Some(&self.project), &trust)
            .map_err(|refusal| refusal.to_string())?;
        app.prepare_file_command_submission(typed, &prepared.prompt)
    }

    fn decide(&self, name: &str) -> Result<(Executable, TrustStatus), String> {
        let catalog = self.catalog();
        let entry = catalog.entries().iter()
            .find(|entry| entry.command.name == name && entry.command.layer == CommandLayer::Project)
            .ok_or_else(|| {
                if catalog.entries().iter().any(|entry| entry.command.name == name) {
                    format!("`/{name}` is a user command; only project commands require trust")
                } else {
                    format!("`/{name}` is not a discovered project command; run `/commands` to list them")
                }
            })?;
        let executable = Executable::from_file(
            &self.project,
            ExecutableKind::SlashCommand,
            &entry.command.path,
        )
        .map_err(|error| error.message)?;
        let status = self
            .trust_store()?
            .status(&self.project, &executable)
            .map_err(|error| error.message)?;
        if status == TrustStatus::Trusted {
            return Err(format!(
                "`/{name}` is already trusted at {}",
                executable.digest().as_hex()
            ));
        }
        Ok((executable, status))
    }

    pub(crate) fn confirmation(&self, name: &str) -> Result<(String, String), String> {
        let (executable, status) = self.decide(name)?;
        let digest = executable.digest().as_hex();
        let content = format!(
            "command `/{name}`\n  {}\n  content {digest}\n  status {}\n\nRunning it submits this file's prompt as your own input. The decision covers exactly this content.",
            executable.label(),
            match status {
                TrustStatus::Untrusted => "nothing has been decided about this content",
                TrustStatus::Changed => "approved earlier at different content",
                TrustStatus::Denied => "declined earlier",
                TrustStatus::Trusted => unreachable!("trusted commands need no confirmation"),
            }
        );
        Ok((content, digest.to_owned()))
    }

    pub(crate) fn trust(&self, name: &str, digest: &str) -> Result<CommandsReport, String> {
        let (executable, _) = self.decide(name)?;
        if executable.digest().as_hex() != digest {
            return Err(format!(
                "`/{name}` changed during confirmation; run `/commands trust {name}` again"
            ));
        }
        self.trust_store()?
            .record(&self.project, &executable, TrustDecision::Allow)
            .map_err(|error| error.message)?;
        self.reload()?;
        Ok(CommandsReport::Trusted {
            name: name.to_owned(),
            digest: digest.to_owned(),
        })
    }
}

pub(crate) fn command(
    context: &CommandContext,
    app: &mut App,
    action: CommandsAction,
) -> CommandReport {
    let report = match action {
        CommandsAction::List => CommandsReport::from_catalog(&context.catalog()),
        CommandsAction::Trust(name) => match context.confirmation(&name) {
            Ok((content, digest)) => {
                return CommandReport::CommandTrust {
                    name,
                    content,
                    digest,
                };
            }
            Err(error) => CommandsReport::Error(error),
        },
        CommandsAction::Reload => match context.reload() {
            Ok(catalog) => {
                let report = CommandsReport::Reloaded {
                    runnable: catalog.runnable().count(),
                    entries: catalog.entries().len(),
                    problems: catalog.problems().to_vec(),
                };
                app.set_command_catalog(catalog);
                report
            }
            Err(error) => CommandsReport::Error(error),
        },
    };
    CommandReport::Show(LocalResult::Commands(Box::new(report)))
}
