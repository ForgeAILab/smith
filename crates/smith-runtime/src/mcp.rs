//! Connecting declared MCP servers without letting them delay a session.
//!
//! A configured server is a third party: it may take a minute to install
//! itself, it may never start, and it may die in the middle of a turn. None of
//! that may cost the user their prompt, so connection happens *beside* session
//! start rather than in front of it. The supervisor owns that work.
//!
//! # What it decides and what it does not
//!
//! Whether a server may be started at all is [`smith_config::mcp`]'s question,
//! answered from the trust store before anything is spawned. This module asks
//! it, and then only connects what was admitted. An untrusted server contributes
//! no tools, is never spawned, and is reported — including in a non-interactive
//! run, where there is nobody to ask and fail-closed is the only honest answer.
//!
//! # Why the supervisor outlives the runtime
//!
//! Smith's tool registry is sealed when a runtime is composed, so a server that
//! connects afterwards cannot push tools into the live one. The supervisor
//! therefore holds the connections and their tools across session rebuilds:
//! composition reads whatever has connected so far, and a later rebuild — the
//! same safe boundary `/model` and `/agent` already use — picks up the rest.
//! Connections are not re-established each time, which is what makes that
//! boundary cheap enough to cross.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::tool::Tool;
use agent_runtime_mcp::{
    McpClient, McpConnection, McpError, McpServerConfig, McpTool, McpTransport,
};
use async_trait::async_trait;
use smith_config::credential::{CredentialRef, CredentialResolver};
use smith_config::mcp::{McpAdmission, admit};
use smith_config::resolve::{
    AUTHORIZATION_HEADER, McpValue, ResolvedMcp, ResolvedMcpServer, ResolvedMcpTransport, Source,
    Sourced,
};
use smith_config::trust::{TrustStatus, TrustStore};

/// How long a server has to become ready before it is written off.
pub const DEFAULT_STARTUP_TIMEOUT_MS: u64 = 30_000;

/// The longest failure reason any surface will be asked to render.
const MAX_REASON_CHARS: usize = 200;

/// Where one declared server has got to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpState {
    /// `enabled = false`. Nothing is asked and nothing runs.
    Disabled,
    /// The user has not approved this exact invocation, so it is not started.
    NeedsTrust(TrustStatus),
    /// Trusted, and still becoming ready.
    Connecting,
    /// Connected, with this many tools registered.
    Connected {
        /// Tools accepted from the server.
        tools: usize,
    },
    /// Tried and failed. The reason is kept rather than logged and dropped.
    Failed {
        /// A bounded, secret-free explanation.
        reason: String,
    },
}

impl McpState {
    /// Whether this state can still change on its own.
    pub fn is_settled(&self) -> bool {
        !matches!(self, Self::Connecting)
    }

    /// The word a surface uses for this state.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::NeedsTrust(_) => "untrusted",
            Self::Connecting => "connecting",
            Self::Connected { .. } => "connected",
            Self::Failed { .. } => "failed",
        }
    }
}

/// What a surface may say about one declared server.
///
/// There is no field a credential value could occupy: an environment variable
/// is named, never shown, and a failure reason is bounded and comes from the
/// error taxonomy rather than from a formatted configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerReport {
    /// The declared name, which namespaces the server's tools.
    pub name: String,
    /// The transport, spelled as configuration spells it.
    pub transport: &'static str,
    /// Where the winning declaration was written.
    pub source: Source,
    /// Where the server has got to.
    pub state: McpState,
    /// Tools the server advertised and Smith would not register, with reasons.
    pub rejected: Vec<String>,
}

/// Everything the supervisor needs that does not come from configuration.
#[derive(Clone)]
pub struct McpOptions {
    /// The canonical project trust decisions are recorded against.
    pub project: PathBuf,
    /// How a configured credential reference becomes a secret.
    pub credentials: Option<CredentialResolver>,
    /// How long a platform credential lookup may wait for access.
    pub credential_timeout_ms: u64,
    /// How long a server has to become ready.
    pub startup_timeout: Duration,
    /// How much text one remote call may contribute to the transcript.
    pub max_output_bytes: usize,
}

impl std::fmt::Debug for McpOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpOptions")
            .field("project", &self.project)
            .field("has_credentials", &self.credentials.is_some())
            .field("credential_timeout_ms", &self.credential_timeout_ms)
            .field("startup_timeout", &self.startup_timeout)
            .field("max_output_bytes", &self.max_output_bytes)
            .finish()
    }
}

impl McpOptions {
    /// Options for `project`, with Smith's defaults for everything else.
    pub fn new(project: impl Into<PathBuf>) -> Self {
        Self {
            project: project.into(),
            credentials: None,
            credential_timeout_ms: 10_000,
            startup_timeout: Duration::from_millis(DEFAULT_STARTUP_TIMEOUT_MS),
            max_output_bytes: agent_runtime_mcp::config::DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    /// Supplies the resolver that turns a credential reference into a secret.
    pub fn with_credentials(mut self, credentials: Option<CredentialResolver>) -> Self {
        self.credentials = credentials;
        self
    }

    /// Bounds how much text one remote call contributes to the transcript.
    pub fn with_max_output_bytes(mut self, bytes: usize) -> Self {
        self.max_output_bytes = bytes;
        self
    }
}

/// One connected server, as the supervisor records it.
pub struct McpConnected {
    /// The tools Smith will register.
    pub tools: Vec<Arc<dyn Tool>>,
    /// Advertised tools that were refused, and why.
    pub rejected: Vec<McpError>,
    /// The live connection, when the connector has one to hand back.
    pub connection: Option<Arc<McpConnection>>,
}

/// How the supervisor reaches a server.
///
/// Injected rather than assumed so a test can drive the whole lifecycle — slow,
/// failing, connected — without a child process, and so the production path
/// stays exactly one implementation of one trait.
#[async_trait]
pub trait McpConnector: Send + Sync + std::fmt::Debug {
    /// Connects, lists, and binds one server's tools.
    async fn connect(&self, config: &McpServerConfig) -> Result<McpConnected, McpError>;
}

/// The production connector: the shared package's client.
#[derive(Debug, Default, Clone)]
pub struct ClientConnector;

#[async_trait]
impl McpConnector for ClientConnector {
    async fn connect(&self, config: &McpServerConfig) -> Result<McpConnected, McpError> {
        let (connection, bindings, rejected) = McpClient::new().connect_and_bind(config).await?;
        let connection = Arc::new(connection);
        let tools = bindings
            .into_iter()
            .map(|binding| {
                Arc::new(McpTool::new(
                    connection.clone(),
                    binding,
                    config.request_timeout,
                    config.max_output_bytes,
                )) as Arc<dyn Tool>
            })
            .collect();
        Ok(McpConnected {
            tools,
            rejected,
            connection: Some(connection),
        })
    }
}

#[derive(Debug)]
struct ServerSlot {
    report: McpServerReport,
    declaration: Option<ResolvedMcpServer>,
    tools: Vec<Arc<dyn Tool>>,
    connection: Option<Arc<McpConnection>>,
}

/// Every declared server's connection, held across session rebuilds.
pub struct McpSupervisor {
    connector: Arc<dyn McpConnector>,
    options: McpOptions,
    servers: Mutex<BTreeMap<String, ServerSlot>>,
    changes: tokio::sync::watch::Sender<u64>,
}

impl std::fmt::Debug for McpSupervisor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpSupervisor")
            .field("options", &self.options)
            .field("generation", &self.generation())
            .finish()
    }
}

impl McpSupervisor {
    /// Decides what may be connected, without connecting anything.
    ///
    /// # Errors
    ///
    /// Fails only when the project cannot be canonicalized, which is the trust
    /// store's rule: a root that cannot be resolved cannot be matched against a
    /// recorded decision.
    pub fn plan(
        mcp: &ResolvedMcp,
        trust: &TrustStore,
        options: McpOptions,
    ) -> Result<Arc<Self>, RuntimeError> {
        Self::planned_with(mcp, trust, options, Arc::new(ClientConnector))
    }

    /// [`Self::plan`], reaching servers through `connector`.
    pub fn planned_with(
        mcp: &ResolvedMcp,
        trust: &TrustStore,
        options: McpOptions,
        connector: Arc<dyn McpConnector>,
    ) -> Result<Arc<Self>, RuntimeError> {
        let mut servers = BTreeMap::new();
        for server in mcp.servers.values() {
            let admission = admit(server, trust, &options.project)?;
            let state = match admission {
                McpAdmission::Connect => McpState::Connecting,
                McpAdmission::Disabled => McpState::Disabled,
                McpAdmission::NeedsTrust(status) => McpState::NeedsTrust(status),
            };
            servers.insert(
                server.name.clone(),
                ServerSlot {
                    report: McpServerReport {
                        name: server.name.clone(),
                        transport: server.transport.as_str(),
                        source: server.source.clone(),
                        state,
                        rejected: Vec::new(),
                    },
                    declaration: admission.connects().then(|| server.clone()),
                    tools: Vec::new(),
                    connection: None,
                },
            );
        }

        Ok(Arc::new(Self {
            connector,
            options,
            servers: Mutex::new(servers),
            changes: tokio::sync::watch::Sender::new(0),
        }))
    }

    /// A supervisor with nothing declared, for a host that configures none.
    pub fn empty(project: impl Into<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            connector: Arc::new(ClientConnector),
            options: McpOptions::new(project),
            servers: Mutex::new(BTreeMap::new()),
            changes: tokio::sync::watch::Sender::new(0),
        })
    }

    /// Starts connecting every admitted server and returns immediately.
    ///
    /// Returning before the first server is ready is the point: a session that
    /// waited here would hand a third party the power to delay the prompt.
    pub fn connect(self: &Arc<Self>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            // No executor means no session about to run either. Leaving the
            // servers `Connecting` would be a lie, so they are reported as
            // failed rather than pending forever.
            self.fail_all("no asynchronous runtime is available to connect on");
            return;
        };
        let pending: Vec<ResolvedMcpServer> = {
            let servers = self.servers.lock().expect("supervisor state");
            servers
                .values()
                .filter_map(|slot| slot.declaration.clone())
                .collect()
        };
        for server in pending {
            let supervisor = Arc::clone(self);
            handle.spawn(async move {
                supervisor.connect_one(server).await;
            });
        }
    }

    /// Waits until every server has settled, or `timeout` elapses.
    ///
    /// A one-shot run has no later boundary to pick tools up at, so it waits
    /// here — bounded, and never on the interactive path.
    pub async fn settle(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut changes = self.changes.subscribe();
        while !self.settled() {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return;
            }
            if tokio::time::timeout(remaining, changes.changed())
                .await
                .is_err()
            {
                return;
            }
        }
    }

    /// Whether nothing is still connecting.
    pub fn settled(&self) -> bool {
        self.servers
            .lock()
            .expect("supervisor state")
            .values()
            .all(|slot| slot.report.state.is_settled())
    }

    /// Every declared server's current report, in name order.
    pub fn reports(&self) -> Vec<McpServerReport> {
        self.servers
            .lock()
            .expect("supervisor state")
            .values()
            .map(|slot| slot.report.clone())
            .collect()
    }

    /// One server's current report.
    pub fn report(&self, name: &str) -> Option<McpServerReport> {
        self.servers
            .lock()
            .expect("supervisor state")
            .get(name)
            .map(|slot| slot.report.clone())
    }

    /// Every connected server's tools, in server order.
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        self.servers
            .lock()
            .expect("supervisor state")
            .values()
            .flat_map(|slot| slot.tools.iter().cloned())
            .collect()
    }

    /// How many times the registered tool set has changed.
    ///
    /// A host that composed a runtime at one generation knows it is stale when
    /// this moves.
    pub fn generation(&self) -> u64 {
        *self.changes.borrow()
    }

    /// A receiver that fires whenever the registered tool set changes.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Records a trust decision's effect: the server becomes connectable.
    ///
    /// The caller has already written the decision to the trust store; this is
    /// what makes it take effect without restarting the session.
    pub fn admit_now(self: &Arc<Self>, server: &ResolvedMcpServer) {
        {
            let mut servers = self.servers.lock().expect("supervisor state");
            let Some(slot) = servers.get_mut(&server.name) else {
                return;
            };
            slot.report.state = McpState::Connecting;
            slot.declaration = Some(server.clone());
        }
        let supervisor = Arc::clone(self);
        let server = server.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                supervisor.connect_one(server).await;
            });
        }
    }

    /// Closes every connection this supervisor still owns.
    ///
    /// A connection a live runtime still holds a tool from is left to process
    /// teardown: retiring it underneath an in-flight call would turn a clean
    /// exit into a failed turn.
    pub async fn shutdown(&self) {
        let connections: Vec<Arc<McpConnection>> = {
            let mut servers = self.servers.lock().expect("supervisor state");
            servers
                .values_mut()
                .filter_map(|slot| {
                    slot.tools.clear();
                    slot.connection.take()
                })
                .collect()
        };
        for connection in connections {
            if let Some(connection) = Arc::into_inner(connection) {
                let _ = connection.shutdown().await;
            }
        }
        self.publish();
    }

    async fn connect_one(self: Arc<Self>, server: ResolvedMcpServer) {
        let config = match self.server_config(&server).await {
            Ok(config) => config,
            Err(reason) => {
                self.record_failure(&server.name, reason);
                return;
            }
        };

        match self.connector.connect(&config).await {
            Ok(connected) => {
                let mut servers = self.servers.lock().expect("supervisor state");
                if let Some(slot) = servers.get_mut(&server.name) {
                    slot.report.state = McpState::Connected {
                        tools: connected.tools.len(),
                    };
                    slot.report.rejected = connected
                        .rejected
                        .iter()
                        .map(|error| bounded(error.to_string()))
                        .collect();
                    slot.tools = connected.tools;
                    slot.connection = connected.connection;
                }
                drop(servers);
                self.publish();
            }
            Err(error) => self.record_failure(&server.name, bounded(error.to_string())),
        }
    }

    /// Turns a declaration into the shared package's configuration, resolving
    /// credential references on the way.
    ///
    /// This is where a reference finally becomes a secret, and it happens here
    /// rather than during resolution because it is behind the trust boundary:
    /// nothing reaches this function that the user has not approved.
    async fn server_config(&self, server: &ResolvedMcpServer) -> Result<McpServerConfig, String> {
        let transport = match &server.transport {
            ResolvedMcpTransport::Stdio { command, args } => McpTransport::Stdio {
                command: command.value.clone(),
                args: args
                    .as_ref()
                    .map(|args| args.value.clone())
                    .unwrap_or_default(),
                env: self.environment(server).await?,
                cwd: None,
            },
            ResolvedMcpTransport::StreamableHttp {
                url,
                credential,
                headers,
            } => McpTransport::StreamableHttp {
                url: url.value.clone(),
                headers: self.headers(credential.as_ref(), headers).await?,
            },
        };
        Ok(McpServerConfig::new(server.name.clone(), transport)
            .with_startup_timeout(self.options.startup_timeout)
            .with_max_output_bytes(self.options.max_output_bytes))
    }

    async fn environment(
        &self,
        server: &ResolvedMcpServer,
    ) -> Result<BTreeMap<String, String>, String> {
        self.resolve_values(&server.env).await
    }

    /// Builds a remote server's headers, turning a declared credential into the
    /// bearer token it is sent as.
    ///
    /// The scheme word is added here rather than being written in
    /// configuration, so a user cannot accidentally send a raw token where a
    /// `Bearer` was meant — or paste `Bearer sk-…` into a repository file and
    /// have it work.
    async fn headers(
        &self,
        credential: Option<&Sourced<String>>,
        declared: &BTreeMap<String, Sourced<McpValue>>,
    ) -> Result<BTreeMap<String, String>, String> {
        let mut headers = self.resolve_values(declared).await?;
        if let Some(credential) = credential {
            let token = self.secret(AUTHORIZATION_HEADER, &credential.value).await?;
            headers.insert(AUTHORIZATION_HEADER.to_owned(), format!("Bearer {token}"));
        }
        Ok(headers)
    }

    async fn resolve_values(
        &self,
        declared: &BTreeMap<String, Sourced<McpValue>>,
    ) -> Result<BTreeMap<String, String>, String> {
        let mut resolved = BTreeMap::new();
        for (name, value) in declared {
            let value = match &value.value {
                McpValue::Literal(literal) => literal.expose().to_owned(),
                McpValue::Credential(reference) => self.secret(name, reference).await?,
            };
            resolved.insert(name.clone(), value);
        }
        Ok(resolved)
    }

    /// Resolves one credential reference, naming the variable and never the
    /// value in any diagnostic it can produce.
    async fn secret(&self, variable: &str, reference: &str) -> Result<String, String> {
        let parsed = CredentialRef::parse(reference).map_err(|error| {
            format!("`{variable}` names a credential that is unusable: {error}")
        })?;
        let resolver =
            self.options.credentials.clone().ok_or_else(|| {
                format!("`{variable}` needs a credential and none can be resolved")
            })?;

        let (sender, receiver) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("smith-mcp-credential".into())
            .spawn(move || {
                let _ = sender.send(resolver.resolve_blocking(&parsed));
            })
            .map_err(|_| format!("`{variable}` could not be resolved: no thread was available"))?;

        let timeout = Duration::from_millis(self.options.credential_timeout_ms.max(1));
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(Ok(secret))) => Ok(secret.expose().to_owned()),
            Ok(Ok(Err(error))) => Err(format!("`{variable}` could not be resolved: {error}")),
            Ok(Err(_)) => Err(format!("`{variable}` could not be resolved")),
            Err(_) => Err(format!(
                "`{variable}` could not be resolved within {}ms",
                self.options.credential_timeout_ms
            )),
        }
    }

    fn record_failure(&self, name: &str, reason: String) {
        {
            let mut servers = self.servers.lock().expect("supervisor state");
            if let Some(slot) = servers.get_mut(name) {
                slot.report.state = McpState::Failed {
                    reason: bounded(reason),
                };
                slot.tools.clear();
                slot.connection = None;
            }
        }
        self.publish();
    }

    fn fail_all(&self, reason: &str) {
        {
            let mut servers = self.servers.lock().expect("supervisor state");
            for slot in servers.values_mut() {
                if matches!(slot.report.state, McpState::Connecting) {
                    slot.report.state = McpState::Failed {
                        reason: reason.to_owned(),
                    };
                }
            }
        }
        self.publish();
    }

    fn publish(&self) {
        self.changes.send_modify(|generation| *generation += 1);
    }
}

/// Trims a reason to something a status line can hold.
fn bounded(reason: String) -> String {
    if reason.chars().count() <= MAX_REASON_CHARS {
        return reason;
    }
    let kept: String = reason.chars().take(MAX_REASON_CHARS).collect();
    format!("{kept}…")
}

/// Whether `project` declares any MCP server at all.
///
/// A project that declares none must behave exactly as it did before this
/// existed, which is easiest to guarantee by not building a supervisor for it.
pub fn is_configured(mcp: &ResolvedMcp) -> bool {
    !mcp.servers.is_empty()
}

/// Opens the trust store a supervisor consults, under `state_root`.
///
/// # Errors
///
/// Fails when a trust file exists and cannot be read, which is deliberately
/// loud: continuing with an empty store would silently re-ask for everything.
pub fn trust_store(state_root: &Path) -> Result<TrustStore, RuntimeError> {
    TrustStore::open(state_root)
}
