//! What a run may do with a declared MCP server, decided before anything runs.
//!
//! A declared server is a repository — or a user file — asking Smith to run a
//! program, or to send a credential to a third party. That is the class of
//! authority [`crate::trust`] exists to gate, so admission is a trust question
//! and not an approval one. The two are deliberately separate:
//!
//! - `approval.mode` answers *may this tool call proceed*. Even `allow-all`
//!   answers only that. It is a statement about calls, made once, by a user who
//!   has not seen the server a repository added since.
//! - Execution trust answers *may this exact invocation be started at all*,
//!   once per content digest, and no approval mode can supply it.
//!
//! Nothing here prompts, spawns, connects, or reads a secret. It reports what a
//! surface must ask and what a run may do, which is what lets the same policy
//! serve the TUI, `smith -p`, and tests without any of them re-deciding it.

use std::path::Path;

use agent_runtime_core::error::RuntimeError;

use crate::resolve::{
    AUTHORIZATION_HEADER, McpValue, ResolvedMcpServer, ResolvedMcpTransport, Source, Sourced,
};
use crate::trust::{ContentDigest, Executable, TrustStatus, TrustStore};

/// What a run may do with one declared server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpAdmission {
    /// The user approved exactly this invocation. It may be connected.
    Connect,
    /// Configuration switched the server off; nothing is asked and nothing runs.
    Disabled,
    /// The invocation is not covered by a decision the user made about it.
    ///
    /// The status distinguishes a server never seen from one whose command
    /// changed and from one that was refused — three different sentences for a
    /// surface, and the same answer for the runtime: do not start it.
    NeedsTrust(TrustStatus),
}

impl McpAdmission {
    /// Whether the run may actually connect.
    pub fn connects(&self) -> bool {
        matches!(self, Self::Connect)
    }
}

/// Decides what may be done with `server` in `project`.
///
/// # Errors
///
/// Fails when `project` cannot be canonicalized, which is the trust store's
/// own rule: a project root that cannot be resolved cannot be matched against a
/// recorded decision, and guessing would answer about a different directory.
pub fn admit(
    server: &ResolvedMcpServer,
    store: &TrustStore,
    project: &Path,
) -> Result<McpAdmission, RuntimeError> {
    if !server.enabled.value {
        return Ok(McpAdmission::Disabled);
    }
    let status = store.status(project, &executable(server))?;
    if status.allows_execution() {
        Ok(McpAdmission::Connect)
    } else {
        Ok(McpAdmission::NeedsTrust(status))
    }
}

/// The trust identity of a server's fully resolved invocation.
pub fn executable(server: &ResolvedMcpServer) -> Executable {
    match &server.transport {
        ResolvedMcpTransport::Stdio { command, args } => Executable::from_mcp_command(
            server.name.clone(),
            &command.value,
            args.as_ref().map_or(&[][..], |args| &args.value),
            server.env.keys(),
        ),
        // Header *names* only, and for the same reason environment names are:
        // a bearer token is a secret, so a rotated credential must not read as
        // a changed server, while gaining a header must.
        ResolvedMcpTransport::StreamableHttp { url, .. } => Executable::from_mcp_endpoint(
            server.name.clone(),
            &url.value,
            server.transport.header_names(),
        ),
    }
}

/// Everything a confirmation surface must show before recording a decision.
///
/// Values are absent by construction rather than by discipline: an environment
/// variable appears as its name and, when it names one, its credential
/// reference. There is no field a secret could be placed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConfirmation {
    /// The declared server name.
    pub server: String,
    /// The transport, spelled as configuration spells it.
    pub transport: &'static str,
    /// The resolved command, or the endpoint for a remote server.
    pub target: String,
    /// The resolved arguments, in order.
    pub args: Vec<String>,
    /// The environment a local server would be given, values withheld.
    pub environment: Vec<McpValueSummary>,
    /// The headers a remote server would be sent, values withheld.
    pub headers: Vec<McpValueSummary>,
    /// The content identity the decision would be recorded against.
    pub digest: ContentDigest,
    /// Where the winning declaration was written.
    pub source: Source,
    /// What is already known about this invocation.
    pub status: TrustStatus,
}

/// One environment variable or header, named rather than shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpValueSummary {
    /// The variable or header name, which is part of the trusted identity.
    pub name: String,
    /// The credential this variable draws from, when it names one.
    ///
    /// `None` means a literal was written. The literal is not reproduced here
    /// under any circumstances: Smith cannot tell a server's secrets from its
    /// settings, so it treats every literal as the first.
    pub credential: Option<String>,
}

/// Describes one server exactly as a confirmation prompt should render it.
pub fn confirmation(server: &ResolvedMcpServer, status: TrustStatus) -> McpConfirmation {
    let (target, headers) = match &server.transport {
        ResolvedMcpTransport::Stdio { command, .. } => (command.value.clone(), Vec::new()),
        ResolvedMcpTransport::StreamableHttp {
            url,
            credential,
            headers,
        } => {
            // A declared `credential` is shown as the header it will actually
            // be sent under, so the prompt describes the request rather than
            // the configuration that produced it.
            let mut summaries: Vec<McpValueSummary> = credential
                .iter()
                .map(|reference| McpValueSummary {
                    name: AUTHORIZATION_HEADER.to_owned(),
                    credential: Some(reference.value.clone()),
                })
                .collect();
            summaries.extend(headers.iter().map(summarize));
            (url.value.clone(), summaries)
        }
    };
    McpConfirmation {
        server: server.name.clone(),
        transport: server.transport.as_str(),
        target,
        args: server.transport.args().to_vec(),
        environment: server.env.iter().map(summarize).collect(),
        headers,
        digest: executable(server).digest().clone(),
        source: server.source.clone(),
        status,
    }
}

fn summarize((name, value): (&String, &Sourced<McpValue>)) -> McpValueSummary {
    McpValueSummary {
        name: name.clone(),
        credential: value.value.credential().map(str::to_owned),
    }
}
