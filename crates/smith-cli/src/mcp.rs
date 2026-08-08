//! The `/mcp` surface: what is declared, what it is doing, and what it needs.
//!
//! A declared server that contributes nothing is the failure this surface
//! exists to prevent. Every declared server appears here whatever state it is
//! in — connected, still starting, switched off, refused for want of trust, or
//! broken — with the reason attached rather than logged somewhere the user will
//! never look.
//!
//! Nothing here can print a secret. A server's environment is rendered as
//! variable names and, where one is used, the *reference* a value is drawn
//! from; a value written literally is reported as withheld, because Smith
//! cannot tell a server's secrets from its settings.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use smith_config::mcp::{self, McpConfirmation};
use smith_config::resolve::{ResolvedConfig, ResolvedMcpServer};
use smith_config::trust::{TrustDecision, TrustStatus, TrustStore};
use smith_runtime::mcp::{McpOptions, McpState, McpSupervisor};

/// Everything the `/mcp` command needs, owned for the life of the process.
///
/// It outlives one session on purpose: a server connected before a `/model`
/// switch is still connected after it, and re-dialing every server at each
/// rebuild would make switching models cost an `npx` install.
pub(super) struct McpContext {
    supervisor: Arc<McpSupervisor>,
    servers: BTreeMap<String, ResolvedMcpServer>,
    trust: Mutex<TrustStore>,
    project: PathBuf,
}

impl std::fmt::Debug for McpContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpContext")
            .field("servers", &self.servers.keys().collect::<Vec<_>>())
            .field("project", &self.project)
            .finish()
    }
}

impl McpContext {
    /// Plans every declared server and starts connecting the trusted ones.
    ///
    /// Returns `None` when nothing is declared: a project with no `[mcp]` table
    /// must behave exactly as it did before this existed, down to spawning no
    /// task and opening no trust file.
    pub(super) fn start(
        config: &ResolvedConfig,
        user_dir: &Path,
        project: &Path,
        credentials: Option<smith_config::credential::CredentialResolver>,
    ) -> anyhow::Result<Option<Arc<Self>>> {
        if config.mcp.servers.is_empty() {
            return Ok(None);
        }
        let trust = TrustStore::open(user_dir).map_err(|error| anyhow::anyhow!("{error}"))?;
        let options = McpOptions::new(project)
            .with_credentials(credentials)
            .with_max_output_bytes(
                usize::try_from(config.limits.tool_output_limit_bytes.value).unwrap_or(usize::MAX),
            );
        let supervisor = McpSupervisor::plan(&config.mcp, &trust, options)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        supervisor.connect();

        Ok(Some(Arc::new(Self {
            supervisor,
            servers: config.mcp.servers.clone(),
            trust: Mutex::new(trust),
            project: project.to_path_buf(),
        })))
    }

    /// The supervisor a runtime composes its remote tools from.
    pub(super) fn supervisor(&self) -> Arc<McpSupervisor> {
        Arc::clone(&self.supervisor)
    }

    /// Every declared server, with its state and — where it has one — its tool
    /// count, its configuration source, and a bounded failure reason.
    pub(super) fn render_list(&self) -> String {
        let mut lines = Vec::new();
        for report in self.supervisor.reports() {
            let state = match &report.state {
                McpState::Connected { tools } => {
                    format!("connected · {tools} tool{}", plural(*tools))
                }
                McpState::Connecting => "connecting".to_owned(),
                McpState::Disabled => "disabled".to_owned(),
                McpState::NeedsTrust(TrustStatus::Changed) => format!(
                    "untrusted · its command changed — run `/mcp trust {}`",
                    report.name
                ),
                McpState::NeedsTrust(TrustStatus::Denied) => format!(
                    "refused · you declined it — run `/mcp trust {}` to reconsider",
                    report.name
                ),
                McpState::NeedsTrust(_) => {
                    format!(
                        "untrusted · needs approval — run `/mcp trust {}`",
                        report.name
                    )
                }
                McpState::Failed { reason } => format!("failed · {reason}"),
            };
            lines.push(format!(
                "{} · {} · {state} · {}",
                report.name, report.transport, report.source
            ));
            for rejected in &report.rejected {
                lines.push(format!("  refused a tool: {rejected}"));
            }
            if let Some(server) = self.servers.get(&report.name) {
                lines.extend(
                    environment_lines(server)
                        .into_iter()
                        .map(|line| format!("  {line}")),
                );
            }
        }
        if lines.is_empty() {
            return "no MCP servers are declared".to_owned();
        }
        lines.join("\n")
    }

    /// Renders what the user is being asked to authorize, without recording
    /// anything.
    pub(super) fn confirmation(&self, name: &str) -> Result<String, String> {
        let server = self.declared(name)?;
        let status = self
            .trust
            .lock()
            .expect("trust store")
            .status(&self.project, &mcp::executable(server))
            .map_err(|error| error.to_string())?;
        Ok(render_confirmation(&mcp::confirmation(server, status)))
    }

    /// Records the user's decision and lets the server connect.
    ///
    /// The decision is persisted before the server is dialed: a spawn that
    /// happened on the strength of an unsaved decision would run again
    /// unattended on the next start.
    pub(super) fn trust(&self, name: &str) -> Result<String, String> {
        let server = self.declared(name)?;
        let executable = mcp::executable(server);
        self.trust
            .lock()
            .expect("trust store")
            .record(&self.project, &executable, TrustDecision::Allow)
            .map_err(|error| error.to_string())?;
        self.supervisor.admit_now(server);
        Ok(format!(
            "`{name}` is trusted at {} and is connecting; its tools join at the next safe boundary",
            executable.digest()
        ))
    }

    fn declared(&self, name: &str) -> Result<&ResolvedMcpServer, String> {
        self.servers.get(name).ok_or_else(|| {
            let known = self
                .servers
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            if known.is_empty() {
                "no MCP servers are declared".to_owned()
            } else {
                format!("`{name}` is not a declared MCP server; declared: {known}")
            }
        })
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// One line per environment variable and header: its name, and where its
/// value comes from. Never the value.
fn environment_lines(server: &ResolvedMcpServer) -> Vec<String> {
    let confirmation = mcp::confirmation(server, TrustStatus::Untrusted);
    confirmation
        .environment
        .iter()
        .map(|value| ("env", value))
        .chain(confirmation.headers.iter().map(|value| ("header", value)))
        .map(|(kind, value)| match &value.credential {
            Some(reference) => format!("{kind} {} ← {reference}", value.name),
            None => format!("{kind} {} ← value withheld", value.name),
        })
        .collect()
}

fn render_confirmation(confirmation: &McpConfirmation) -> String {
    let mut lines = vec![
        format!(
            "MCP server `{}` ({})",
            confirmation.server, confirmation.transport
        ),
        format!("declared at {}", confirmation.source),
        match confirmation.status {
            TrustStatus::Changed => {
                "its content changed since you last decided about it".to_owned()
            }
            TrustStatus::Denied => "you refused this exact content before".to_owned(),
            TrustStatus::Trusted => {
                "already trusted; confirming again renews the decision".to_owned()
            }
            TrustStatus::Untrusted => "nothing has been decided about this content".to_owned(),
        },
        String::new(),
        match confirmation.transport {
            "http" => format!("endpoint {}", confirmation.target),
            _ => format!("command  {}", confirmation.target),
        },
    ];
    if !confirmation.args.is_empty() {
        lines.push(format!("args     {}", confirmation.args.join(" ")));
    }
    for variable in &confirmation.environment {
        lines.push(match &variable.credential {
            Some(reference) => format!("env      {} ← {reference}", variable.name),
            None => format!("env      {} ← value withheld", variable.name),
        });
    }
    for header in &confirmation.headers {
        lines.push(match &header.credential {
            Some(reference) => format!("header   {} ← {reference}", header.name),
            None => format!("header   {} ← value withheld", header.name),
        });
    }
    lines.push(format!("identity {}", confirmation.digest));
    lines.push(String::new());
    lines.push(match confirmation.transport {
        "http" => "Trusting this sends the credentials above to that endpoint whenever this \
                   project is opened, until the declaration changes."
            .to_owned(),
        _ => "Trusting this runs the command above whenever this project is opened, until its \
              content changes."
            .to_owned(),
    });
    lines.join("\n")
}
