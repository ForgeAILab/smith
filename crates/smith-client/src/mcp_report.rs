//! Local `/mcp` results and their plain-text rendering.
//!
//! Server state, rejected tools, and credential references travel as data.
//! Trust confirmation remains a separate approval surface.

/// The result of listing MCP servers or recording a trust decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpReport {
    /// No servers are declared; the initial command also offers setup guidance.
    Empty {
        /// Whether to include the existing configuration-table hint.
        guidance: bool,
    },
    /// The MCP context is unavailable when a confirmed decision is applied.
    Unavailable,
    /// The host could not prepare or record a trust decision.
    Error(String),
    /// Declared servers in the supervisor's existing order.
    Servers(Vec<McpServer>),
    /// The exact invocation was trusted and admitted for connection.
    Trusted {
        /// Declared server name.
        server: String,
        /// Content identity covered by the recorded decision.
        digest: String,
    },
}

impl McpReport {
    /// The existing message when no servers are declared.
    pub const EMPTY_MESSAGE: &str = "no MCP servers are declared";
    /// The existing guidance when the command has no MCP context.
    pub const EMPTY_GUIDANCE: &str =
        "no MCP servers are declared; add an `[mcp.servers.<name>]` table";

    /// The existing acknowledgement of a recorded trust decision.
    pub fn trusted_value(server: &str, digest: &str) -> String {
        format!(
            "`{server}` is trusted at {digest} and is connecting; its tools join at the next safe boundary"
        )
    }
}

/// One declared server's state and secret-free configuration provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    /// Declared server name.
    pub name: String,
    /// Transport, spelled as configuration spells it.
    pub transport: String,
    /// Winning declaration's existing source display value.
    pub source: String,
    /// Connection or admission state.
    pub state: McpServerState,
    /// Refused tool explanations, in supervisor order.
    pub rejected: Vec<String>,
    /// Environment variables followed by headers; values are never included.
    pub values: Vec<McpValue>,
}

/// Connection and trust outcomes, independent of display labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerState {
    /// Connected with this many accepted tools.
    Connected {
        /// Number of registered tools.
        tools: usize,
    },
    /// The server is still starting.
    Connecting,
    /// Configuration disabled the server.
    Disabled,
    /// The invocation changed since the last trust decision.
    Changed,
    /// The user refused the invocation.
    Denied,
    /// The invocation needs approval.
    Untrusted,
    /// Connection failed with a bounded, secret-free reason.
    Failed {
        /// Supervisor-owned failure explanation.
        reason: String,
    },
}

impl McpServerState {
    /// The existing state value, including any trust guidance.
    pub fn render_value(&self, server: &str) -> String {
        match self {
            Self::Connected { tools } => format!(
                "connected · {tools} tool{}",
                if *tools == 1 { "" } else { "s" },
            ),
            Self::Connecting => "connecting".to_owned(),
            Self::Disabled => "disabled".to_owned(),
            Self::Changed => {
                format!("untrusted · its command changed — run `/mcp trust {server}`")
            }
            Self::Denied => {
                format!("refused · you declined it — run `/mcp trust {server}` to reconsider")
            }
            Self::Untrusted => {
                format!("untrusted · needs approval — run `/mcp trust {server}`")
            }
            Self::Failed { reason } => format!("failed · {reason}"),
        }
    }
}

/// The semantic location of a server's named value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpValueKind {
    /// Environment variable for a local process.
    Environment,
    /// Header sent to a remote endpoint.
    Header,
}

impl McpValueKind {
    /// The existing label for this value kind.
    pub fn label(self) -> &'static str {
        match self {
            Self::Environment => "env",
            Self::Header => "header",
        }
    }
}

/// A variable or header with only its credential reference, never its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpValue {
    /// Environment variable or header.
    pub kind: McpValueKind,
    /// Variable or header name.
    pub name: String,
    /// Credential reference; absent for a withheld literal.
    pub credential: Option<String>,
}

/// Renders the transcript body for the existing plain-text capture surface.
/// No terminal libraries or title/label parsing are involved.
pub fn render_plain(report: &McpReport) -> String {
    let servers = match report {
        McpReport::Empty { guidance: true } => return McpReport::EMPTY_GUIDANCE.to_owned(),
        McpReport::Empty { guidance: false } | McpReport::Unavailable => {
            return McpReport::EMPTY_MESSAGE.to_owned();
        }
        McpReport::Error(error) => return error.clone(),
        McpReport::Trusted { server, digest } => return McpReport::trusted_value(server, digest),
        McpReport::Servers(servers) => servers,
    };
    let mut lines = Vec::new();
    for server in servers {
        lines.push(format!(
            "{} · {} · {} · {}",
            server.name,
            server.transport,
            server.state.render_value(&server.name),
            server.source,
        ));
        for rejected in &server.rejected {
            lines.push(format!("  refused a tool: {rejected}"));
        }
        for value in &server.values {
            lines.push(format!(
                "  {} {} ← {}",
                value.kind.label(),
                value.name,
                value.credential.as_deref().unwrap_or("value withheld"),
            ));
        }
    }
    lines.join("\n")
}
