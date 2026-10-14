//! Resolving a configured agent and building a backend for each run.

use std::path::{Path, PathBuf};

use agent_runtime::agent::external::ExternalAgentBackend;

use super::{CliAgentBackend, CliAgentKind, CliAgentPlan, CliAgentSettings, TurnExecution};

impl CliAgentPlan {
    /// Builds the backend that runs turns in `cwd`.
    ///
    /// `allow_own_tools` only narrows: a read-only run may withhold the CLI's
    /// own tools, but no caller can grant what the owner did not configure.
    pub(crate) fn backend(
        &self,
        cwd: PathBuf,
        allow_own_tools: bool,
    ) -> std::sync::Arc<dyn ExternalAgentBackend> {
        std::sync::Arc::new(CliAgentBackend::new(
            self.kind,
            self.settings(cwd, allow_own_tools),
        ))
    }

    fn settings(&self, cwd: PathBuf, allow_own_tools: bool) -> CliAgentSettings {
        CliAgentSettings {
            executable: self.executable.clone(),
            model: Some(self.model.clone()),
            args: self.args.clone(),
            cwd,
            env: self.env.clone(),
            allow_own_tools: self.allow_own_tools && allow_own_tools,
            instructions: None,
            relay_executable: self.relay_executable.clone(),
            // Each turn binds its own endpoint from the runtime's authorized view.
            bridge: None,
        }
    }

    /// Stable label naming what this plan executes, for policy fingerprints.
    pub(crate) fn label(&self) -> String {
        format!("cli:{}/{}", self.kind.as_str(), self.model)
    }
}

impl TurnExecution {
    /// Stable label naming what executes a turn, for policy fingerprints.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Provider => "provider".to_owned(),
            Self::InstalledAgent(plan) => plan.label(),
            Self::MissingProgram { kind, .. } => format!("cli-missing:{kind}"),
        }
    }
}

/// Reads what a resolved configuration says should execute its turns.
pub(crate) fn turn_execution(
    config: &smith_config::resolve::ResolvedConfig,
    smith_executable: Option<&Path>,
) -> TurnExecution {
    config
        .harness
        .as_ref()
        .map_or(TurnExecution::Provider, |harness| {
            selection(harness, smith_executable)
        })
}

/// Resolves one installed-agent selection against this machine.
fn selection(
    harness: &smith_config::resolve::ResolvedHarness,
    smith_executable: Option<&Path>,
) -> TurnExecution {
    let Some(kind) = CliAgentKind::parse(&harness.kind.value) else {
        return TurnExecution::Provider;
    };
    // Selecting `cli/claude-code/sonnet` should just work, so the executable
    // is found on PATH unless the owner named one.
    let executable = match &harness.executable {
        Some(configured) => PathBuf::from(&configured.value),
        None => match discover_program(&harness.program) {
            Some(executable) => executable,
            None => {
                return TurnExecution::MissingProgram {
                    model: smith_config::cli_agents::cli_model_id(
                        &harness.kind.value,
                        &harness.model.value,
                    ),
                    kind: harness.kind.value.clone(),
                    program: harness.program.clone(),
                };
            }
        },
    };
    TurnExecution::InstalledAgent(CliAgentPlan {
        kind,
        executable,
        model: harness.model.value.clone(),
        args: harness.args.clone(),
        env: harness.env.clone(),
        allow_own_tools: harness.allow_own_tools.value,
        relay_executable: smith_executable.map(Path::to_owned),
    })
}

/// Finds an installed program on `PATH`.
///
/// A plain lookup rather than a shell resolution: the same rule the rest of
/// Smith's process boundaries follow, so nothing a shell would expand can
/// change which binary runs.
fn discover_program(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smith_config::resolve::{ResolvedHarness, Source, Sourced};
    use std::collections::BTreeMap;

    fn settings(allow_own_tools: bool) -> CliAgentSettings {
        CliAgentSettings {
            executable: PathBuf::from("/usr/local/bin/agent"),
            model: Some("a-model".to_owned()),
            args: Vec::new(),
            cwd: PathBuf::from("/repo"),
            env: BTreeMap::new(),
            allow_own_tools,
            instructions: None,
            bridge: None,
            relay_executable: None,
        }
    }

    fn harness(program: &str, executable: Option<&str>, allow_own_tools: bool) -> ResolvedHarness {
        ResolvedHarness {
            kind: Sourced::new("claude-code".to_owned(), Source::built_in("model")),
            program: program.to_owned(),
            model: Sourced::new("sonnet".to_owned(), Source::built_in("model")),
            executable: executable
                .map(|path| Sourced::new(path.to_owned(), Source::built_in("harness.executable"))),
            args: Vec::new(),
            allow_own_tools: Sourced::new(
                allow_own_tools,
                Source::built_in("harness.allow_own_tools"),
            ),
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn an_agent_whose_program_is_not_installed_is_named_rather_than_run_on_the_provider() {
        let execution = selection(&harness("smith-no-such-agent-program", None, false), None);
        let TurnExecution::MissingProgram {
            model,
            kind,
            program,
        } = execution
        else {
            panic!("a missing program must not resolve to a runnable plan: {execution:?}");
        };
        assert_eq!(model, "cli/claude-code/sonnet");
        assert_eq!(kind, "claude-code");
        assert_eq!(program, "smith-no-such-agent-program");
    }

    #[test]
    fn a_plan_narrows_the_cli_s_own_tools_but_never_widens_them() {
        let TurnExecution::InstalledAgent(permitted) = selection(
            &harness("claude", Some("/usr/local/bin/claude"), true),
            None,
        ) else {
            panic!("a configured executable needs no PATH lookup");
        };
        let TurnExecution::InstalledAgent(withheld) = selection(
            &harness("claude", Some("/usr/local/bin/claude"), false),
            None,
        ) else {
            panic!("a configured executable needs no PATH lookup");
        };

        let allow_list = |plan: &CliAgentPlan, write_capable: bool| {
            let backend = CliAgentBackend::new(
                plan.kind,
                CliAgentSettings {
                    allow_own_tools: plan.allow_own_tools && write_capable,
                    ..settings(false)
                },
            );
            let args = backend.argv("hello", None).expect("valid invocation paths");
            args.iter().any(|arg| arg == "--permission-mode")
        };

        assert!(
            allow_list(&permitted, true),
            "a write-capable run keeps the tools the owner configured"
        );
        assert!(
            !allow_list(&permitted, false),
            "a read-only run must not receive the CLI's own tools"
        );
        assert!(
            !allow_list(&withheld, true),
            "no caller may grant tools the owner did not configure"
        );
    }

    fn backend_from_plan(kind: CliAgentKind, smith: Option<&Path>) -> CliAgentBackend {
        let mut harness = harness("agent", Some("/usr/local/bin/agent"), false);
        harness.kind.value = kind.as_str().to_owned();
        let TurnExecution::InstalledAgent(plan) = selection(&harness, smith) else {
            panic!("a configured executable needs no PATH lookup");
        };
        CliAgentBackend::new(plan.kind, plan.settings(PathBuf::from("/repo"), true))
    }

    #[test]
    fn a_plan_without_a_smith_executable_is_unbridged_and_spawns_directly() {
        for kind in [CliAgentKind::ClaudeCode, CliAgentKind::Codex] {
            let backend = backend_from_plan(kind, None);
            assert!(
                backend
                    .bridge_invocation(Path::new("/tmp/turn.sock"))
                    .is_none()
            );
            let args = backend.argv("hello", None).unwrap();
            let expected = match kind {
                CliAgentKind::ClaudeCode => vec![
                    "-p",
                    "hello",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--allowedTools",
                    "",
                    "--model",
                    "sonnet",
                ],
                CliAgentKind::Codex => vec![
                    "exec",
                    "--json",
                    "--skip-git-repo-check",
                    "-C",
                    "/repo",
                    "--sandbox",
                    "read-only",
                    "-m",
                    "sonnet",
                    "hello",
                ],
            };
            assert_eq!(args, expected);
            let command = backend.turn_command(&args);
            assert_eq!(command.as_std().get_program(), "/usr/local/bin/agent");
            assert_eq!(
                command.as_std().get_args().collect::<Vec<_>>(),
                args.iter().map(std::ffi::OsStr::new).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn the_host_smith_executable_is_used_for_both_relay_and_supervisor() {
        let smith = Path::new("/host supplied/smith");
        for kind in [CliAgentKind::ClaudeCode, CliAgentKind::Codex] {
            let mut backend = backend_from_plan(kind, Some(smith));
            backend.settings.bridge = backend.bridge_invocation(Path::new("/tmp/turn.sock"));
            let args = backend.argv("hello", None).unwrap();
            match kind {
                CliAgentKind::ClaudeCode => {
                    let config = args
                        .windows(2)
                        .find(|pair| pair[0] == "--mcp-config")
                        .unwrap();
                    let config: serde_json::Value = serde_json::from_str(&config[1]).unwrap();
                    assert_eq!(
                        config["mcpServers"]["smith"]["command"],
                        smith.to_str().unwrap()
                    );
                    assert_eq!(
                        config["mcpServers"]["smith"]["args"],
                        serde_json::json!(["mcp-relay", "/tmp/turn.sock"])
                    );
                }
                CliAgentKind::Codex => {
                    assert!(
                        args.windows(2).any(|pair| pair
                            == ["-c", "mcp_servers.smith.command=\"/host supplied/smith\""])
                    );
                    assert!(args.windows(2).any(|pair| pair
                        == [
                            "-c",
                            "mcp_servers.smith.args=[\"mcp-relay\",\"/tmp/turn.sock\"]"
                        ]));
                }
            }
            let command = backend.turn_command(&args);
            assert_eq!(command.as_std().get_program(), smith.as_os_str());
            let expected = ["harness-supervise", "--", "/usr/local/bin/agent"]
                .into_iter()
                .map(std::ffi::OsStr::new)
                .chain(args.iter().map(std::ffi::OsStr::new))
                .collect::<Vec<_>>();
            assert_eq!(command.as_std().get_args().collect::<Vec<_>>(), expected);
        }
    }
}
