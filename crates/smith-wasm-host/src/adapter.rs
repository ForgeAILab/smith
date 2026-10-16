use crate::bindings::{Module as Guest, smith::module::types as w};
use crate::broker::Broker;
use crate::limits::{bounded, identifier};
use crate::{WasmEngine, WasmError, WasmLimits};
use agent_runtime::harness::{
    ComponentDescriptor, ContextContributor, ContextPatch, ContextView, ToolOutputPatch,
    ToolOutputProcessor, ToolOutputView,
};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::tool::{InvocationContext, PreparedToolCall, Tool, ToolOutcome, ToolSpec};
use agent_runtime_registry::RegistryRevision;
use smith_module::{
    Module, ModuleCapability, ModuleContext, ModuleContribution, ModuleError, ModuleHealth,
    ModuleOrigin, Mounted, PipelineComponent, StatusItem, StatusSeverity, StatusSource,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wasmtime::Store;
use wasmtime::component::{Component, Linker};

/// Reserved cache subtree below the private host state directory.
pub const COMPONENT_CACHE_DIRECTORY: &str = "plugins/wasm-cache";

/// Host-owned capability scopes. Alias and host lists never come from guest claims.
#[derive(Debug, Clone)]
pub struct CapabilityGrant {
    /// Granted authority category.
    pub capability: ModuleCapability,
    /// Opaque root aliases (`workspace`, `plugin-data`, `session-history`).
    pub roots: Vec<String>,
    /// Exact canonical DNS hosts.
    pub hosts: Vec<String>,
    /// Explicit user permission to use HTTP instead of HTTPS.
    pub allow_http: bool,
}
/// Installation facts supplied by the plugin host at the sole loading seam.
#[derive(Debug, Clone)]
pub struct WasmModuleConfig {
    /// Plugin recorded as UserManifest provenance.
    pub plugin_name: String,
    /// Manifest requests before user narrowing.
    pub requested_capabilities: Vec<CapabilityGrant>,
    /// Effective grants, never wider than requests.
    pub granted_capabilities: Vec<CapabilityGrant>,
    /// Private host state directory, equal to ModuleContext::user_dir.
    pub host_state_dir: PathBuf,
    /// Host-owned isolated plugin data directory.
    pub data_dir: PathBuf,
    /// Cache directory inside host_state_dir/COMPONENT_CACHE_DIRECTORY.
    /// This reserved subtree is never covered by guest file grants.
    pub cache_dir: PathBuf,
    /// Per-store and per-call bounds.
    pub limits: WasmLimits,
    /// Fresh for each session, retained across composition rebuilds.
    pub session_health: Arc<ModuleHealth>,
    /// All tool names reserved by the host registry at this composition boundary.
    pub reserved_tool_names: BTreeSet<String>,
}

/// Versioned sandbox adapter implementing the ordinary module contract.
pub struct WasmModule {
    engine: WasmEngine,
    component: Component,
    config: WasmModuleConfig,
    descriptor: w::Descriptor,
}
impl std::fmt::Debug for WasmModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmModule")
            .field("id", &self.descriptor.id)
            .finish_non_exhaustive()
    }
}
impl WasmModule {
    /// Loads and describes a component. Each subsequent mount creates a fresh instance.
    /// `origin()` must accompany this module in the composition carrier.
    pub fn load(
        engine: WasmEngine,
        component_bytes: impl AsRef<[u8]>,
        mut config: WasmModuleConfig,
    ) -> Result<Arc<Self>, WasmError> {
        identifier(&config.plugin_name)?;
        if !(1..=1024 * 1024).contains(&config.limits.payload_bytes) {
            return Err(WasmError::MalformedResult(
                "payload limit must be 1..=1 MiB".into(),
            ));
        }
        validate_grants(&config)?;
        private_layout(&mut config)?;
        let component = engine.compile(component_bytes.as_ref(), &config.cache_dir)?;
        let mut versioned = false;
        for (name, _) in component
            .component_type()
            .imports(engine.raw())
            .chain(component.component_type().exports(engine.raw()))
        {
            if name.starts_with("smith:module/") {
                versioned = true;
                let version = name.rsplit('@').next().unwrap_or("");
                let pieces = version.split('.').collect::<Vec<_>>();
                if pieces.len() != 3
                    || pieces[0] != "0"
                    || pieces[1] != "1"
                    || pieces[2].parse::<u32>().is_err()
                {
                    return Err(WasmError::InterfaceVersion { found: name.into() });
                }
            } else if name.starts_with("wasi:") {
                return Err(WasmError::Load(format!("ambient import refused: {name}")));
            }
        }
        if !versioned {
            return Err(WasmError::InterfaceVersion {
                found: "unversioned component".into(),
            });
        }
        // Discovery is outside a session. It has no host capabilities and
        // must not borrow the failure counter of a parent session: children
        // can mount this same immutable component with independent health.
        let mut discovery = config.clone();
        discovery.granted_capabilities.clear();
        discovery.session_health = Arc::new(ModuleHealth::default());
        let mut session = Session::new(&engine, &component, &discovery, None)?;
        let deadline = config.limits.component_deadline;
        let descriptor = session.call(deadline, |guest, store| guest.call_describe(store))?;
        bounded(&descriptor, &config.limits)?;
        identifier(&descriptor.id)?;
        crate::limits::revision(&descriptor.revision)?;
        if format!("wasm/{}/{}", config.plugin_name, descriptor.id).len() > 128 {
            return Err(WasmError::OversizedPayload("module evidence id".into()));
        }
        let requirements: BTreeSet<_> = descriptor.requirements.iter().collect();
        for requirement in &descriptor.requirements {
            identifier(requirement)?;
        }
        if requirements.len() != descriptor.requirements.len() {
            return Err(WasmError::MalformedResult("duplicate requirements".into()));
        }
        Ok(Arc::new(Self {
            engine,
            component,
            config,
            descriptor,
        }))
    }
    /// Provenance and exact requested/granted categories for module evidence/listings.
    pub fn origin(&self) -> ModuleOrigin {
        ModuleOrigin::Sandboxed {
            plugin: self.config.plugin_name.clone(),
            capability_details: Vec::new(),
            withheld_capabilities: Vec::new(),
            requested_capabilities: self
                .config
                .requested_capabilities
                .iter()
                .map(|g| g.capability)
                .collect(),
            granted_capabilities: self
                .config
                .granted_capabilities
                .iter()
                .map(|g| g.capability)
                .collect(),
        }
    }
}
impl Module for WasmModule {
    fn id(&self) -> &str {
        &self.descriptor.id
    }
    fn revision(&self) -> &str {
        &self.descriptor.revision
    }
    fn description(&self) -> &str {
        &self.descriptor.description
    }
    fn default_enabled(&self) -> bool {
        self.descriptor.default_enabled
    }
    fn requirement_ids(&self) -> Vec<String> {
        self.descriptor.requirements.clone()
    }
    fn session_health(&self) -> Option<Arc<ModuleHealth>> {
        Some(self.config.session_health.clone())
    }
    fn mount(&self, context: &ModuleContext) -> Result<Mounted, ModuleError> {
        // Module::mount is synchronous. Use a dedicated blocking thread so even
        // guest imports during mount can enter the host's async transport safely.
        let host_dir = context
            .user_dir
            .canonicalize()
            .map_err(|_| ModuleError("host state directory unavailable".into()))?;
        if host_dir != self.config.host_state_dir {
            return Err(ModuleError(
                "host state directory differs from module configuration".into(),
            ));
        }
        let engine = self.engine.clone();
        let component = self.component.clone();
        let config = self.config.clone();
        let context = context.clone();
        let id = self.id().to_owned();
        let runtime = tokio::runtime::Handle::try_current().ok();
        std::thread::Builder::new()
            .name("smith-wasm-mount".into())
            .spawn(move || mount(engine, component, config, context, id, runtime))
            .map_err(|_| ModuleError("module host worker unavailable".into()))?
            .join()
            .map_err(|_| ModuleError("module host worker failed".into()))?
            .map_err(|e| ModuleError(e.to_string()))
    }
}
fn validate_grants(config: &WasmModuleConfig) -> Result<(), WasmError> {
    for grant in config
        .requested_capabilities
        .iter()
        .chain(&config.granted_capabilities)
    {
        if grant.roots.len() > 32 || grant.hosts.len() > 32 {
            return Err(WasmError::OversizedPayload("grant scopes".into()));
        }
        for root in &grant.roots {
            identifier(root)?;
        }
        for host in &grant.hosts {
            let http_origin = url::Url::parse(host).ok().is_some_and(|url| {
                grant.allow_http
                    && host.len() <= 253
                    && url.scheme() == "http"
                    && url.origin().ascii_serialization() == *host
                    && url.username().is_empty()
                    && url.password().is_none()
            });
            if !http_origin
                && (host.len() > 253
                    || host.is_empty()
                    || host != &host.to_ascii_lowercase()
                    || !host.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b".-".contains(&b)
                    }))
            {
                return Err(WasmError::MalformedResult(
                    "invalid exact host or HTTP origin".into(),
                ));
            }
        }
    }
    for grant in &config.granted_capabilities {
        if matches!(
            grant.capability,
            ModuleCapability::Process
                | ModuleCapability::Credential
                | ModuleCapability::Approval
                | ModuleCapability::Provider
        ) {
            return Err(WasmError::CapabilityDenied(format!(
                "{:?} is never granted in v0.1",
                grant.capability
            )));
        }
        if !config.requested_capabilities.iter().any(|r| {
            r.capability == grant.capability
                && grant.roots.iter().all(|v| r.roots.contains(v))
                && grant.hosts.iter().all(|v| r.hosts.contains(v))
                && (!grant.allow_http || r.allow_http)
        }) {
            return Err(WasmError::CapabilityDenied(
                "grant exceeds manifest request".into(),
            ));
        }
    }
    Ok(())
}
struct Session {
    _engine: WasmEngine,
    guest: Guest,
    store: Store<Broker>,
    health: Arc<ModuleHealth>,
    parent_health: Arc<ModuleHealth>,
    child_session: bool,
    status_ids: Vec<String>,
    cache: Arc<Mutex<BTreeMap<String, StatusItem>>>,
    namespace: String,
    started: Instant,
}
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmSession")
            .field("health", &self.health)
            .finish_non_exhaustive()
    }
}
impl Session {
    fn bind_session(&mut self, session: agent_runtime_core::ids::SessionId) {
        if self.child_session {
            self.health = self.parent_health.child_session(&session);
        }
        self.store.data_mut().session = Some(session);
    }
    fn new(
        engine: &WasmEngine,
        component: &Component,
        config: &WasmModuleConfig,
        context: Option<ModuleContext>,
    ) -> Result<Self, WasmError> {
        let child_session = context
            .as_ref()
            .is_some_and(|context| context.is_child_session);
        let mut store = Store::new(engine.raw(), Broker::new(config, context));
        store.limiter(|broker| broker);
        store.set_epoch_deadline(config.limits.component_deadline.as_millis().max(1) as u64);
        let mut linker = Linker::new(engine.raw());
        Guest::add_to_linker::<_, wasmtime::component::HasSelf<Broker>>(
            &mut linker,
            |broker: &mut Broker| broker,
        )
        .map_err(|e| WasmError::Load(e.to_string()))?;
        let guest = Guest::instantiate(&mut store, component, &linker).map_err(|e| {
            if store.data().memory_denied {
                WasmError::Memory
            } else if e.downcast_ref::<wasmtime::Trap>().is_some() {
                WasmError::guest(e, false)
            } else {
                WasmError::Load(e.to_string())
            }
        })?;
        Ok(Self {
            _engine: engine.clone(),
            guest,
            store,
            health: if child_session {
                Arc::new(ModuleHealth::default())
            } else {
                config.session_health.clone()
            },
            parent_health: config.session_health.clone(),
            child_session,
            status_ids: vec![],
            cache: Arc::new(Mutex::new(BTreeMap::new())),
            namespace: String::new(),
            started: Instant::now(),
        })
    }
    fn enter(&mut self, duration: Duration) {
        self.store
            .set_epoch_deadline(duration.as_millis().max(1) as u64);
        self.store.data_mut().deadline = Instant::now() + duration;
        self.store.data_mut().failure = None;
        self.store.data_mut().memory_denied = false;
    }
    fn call<T>(
        &mut self,
        duration: Duration,
        action: impl FnOnce(&Guest, &mut Store<Broker>) -> wasmtime::Result<T>,
    ) -> Result<T, WasmError> {
        if let Some(reason) = self.health.failure() {
            return Err(WasmError::Disabled(reason));
        }
        self.enter(duration);
        let result = action(&self.guest, &mut self.store)
            .map_err(|e| WasmError::guest(e, self.store.data().memory_denied));
        if let Some(error) = self.store.data_mut().failure.take() {
            return Err(error);
        }
        if Instant::now() >= self.store.data().deadline {
            return Err(WasmError::Deadline);
        }
        result
    }
    fn finish<T>(&mut self, result: Result<T, WasmError>) -> Result<T, WasmError> {
        let status = self.refresh();
        let result = result.and_then(|value| status.map(|()| value));
        match &result {
            Ok(_) => self.health.record_success(),
            Err(error) if !matches!(error, WasmError::Disabled(_)) => {
                tracing::warn!(error = %error, "sandboxed module invocation failed");
                if self.health.record_failure(error.to_string()) {
                    self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
                }
            }
            Err(_) => {}
        }
        result
    }
    fn refresh(&mut self) -> Result<(), WasmError> {
        let mut pending = BTreeMap::new();
        for id in self.status_ids.clone() {
            let item = self.call(self.store.data().limits.status_deadline, |guest, store| {
                guest.call_status(store, &id)
            })?;
            if let Some(item) = item {
                bounded(&item, &self.store.data().limits)?;
                if item.name != id {
                    return Err(WasmError::MalformedResult("invalid status id".into()));
                }
                if item.label.chars().count() > 80 {
                    return Err(WasmError::OversizedPayload("status label".into()));
                }
                let severity = item.severity.map(|s| match s {
                    w::StatusSeverity::Info => StatusSeverity::Info,
                    w::StatusSeverity::Warning => StatusSeverity::Warning,
                    w::StatusSeverity::Error => StatusSeverity::Error,
                });
                let name = format!("{}.{}", self.namespace, id);
                pending.insert(id, StatusItem::new(name, item.label, severity));
            }
        }
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = pending;
        Ok(())
    }
}
fn mount(
    engine: WasmEngine,
    component: Component,
    config: WasmModuleConfig,
    context: ModuleContext,
    id: String,
    runtime: Option<tokio::runtime::Handle>,
) -> Result<Mounted, WasmError> {
    if let Some(reason) = config
        .session_health
        .failure()
        .filter(|_| !context.is_child_session)
    {
        return Err(WasmError::Disabled(reason));
    }
    let mut session = Session::new(&engine, &component, &config, Some(context.clone()))?;
    session.store.data_mut().runtime = runtime;
    session.namespace = format!(
        "wasm_{}_{}_{}",
        config.plugin_name.len(),
        config.plugin_name,
        id
    );
    let input = w::MountContext {
        settings: context
            .settings
            .iter()
            .map(|(name, value)| w::Setting {
                name: name.clone(),
                value: match value {
                    smith_module::SettingValue::String(v) => w::SettingValue::Text(v.clone()),
                    smith_module::SettingValue::Bool(v) => w::SettingValue::Boolean(*v),
                    smith_module::SettingValue::Integer(v) => w::SettingValue::Integer(*v),
                },
            })
            .collect(),
        plugin_data_root: "plugin-data".into(),
        posture: match context.posture {
            smith_module::ModulePosture::ReadOnly => w::Posture::ReadOnly,
            smith_module::ModulePosture::ReadWrite => w::Posture::ReadWrite,
        },
        has_image_binding: context.image_binding.is_some(),
        has_session_history: context.session_history.is_some(),
        semantic_summary_enabled: context.semantic_summary_enabled,
        max_input_tokens: context.max_input_tokens,
        built_in_tools: context.built_in_tools,
        grants: config
            .granted_capabilities
            .iter()
            .map(|g| w::CapabilityGrant {
                capability: crate::projection::capability(g.capability),
                roots: g.roots.clone(),
                hosts: g.hosts.clone(),
            })
            .collect(),
    };
    bounded(&input, &config.limits)?;
    let mounted = session
        .call(config.limits.component_deadline, |guest, store| {
            guest.call_mount(store, &input)
        })?
        .map_err(|error| guest_error(&error, &config.limits))?;
    bounded(&mounted, &config.limits)?;
    let w::Mounted::Contributions(declarations) = mounted else {
        if let w::Mounted::Inactive(reason) = mounted {
            return Ok(Mounted::Inactive { reason });
        }
        unreachable!()
    };
    // Validate the entire set before publishing any executable contribution.
    let mut keys = BTreeSet::new();
    let mut tools = Vec::new();
    let mut components = Vec::new();
    for declaration in declarations {
        match declaration {
            w::Contribution::Tool(tool) => {
                identifier(&tool.name)?;
                if !keys.insert(("tool", tool.name.clone())) {
                    return Err(WasmError::MalformedResult("duplicate tool name".into()));
                }
                let spec = crate::projection::tool_spec(&tool, &session.namespace)?;
                if spec.name.len() > 128 {
                    return Err(WasmError::OversizedPayload("namespaced tool name".into()));
                }
                if config.reserved_tool_names.contains(&tool.name)
                    || config.reserved_tool_names.contains(&spec.name)
                    || BUILTIN_NAMES.contains(&tool.name.as_str())
                {
                    return Err(WasmError::MalformedResult(format!(
                        "tool collision: {}",
                        tool.name
                    )));
                }
                tools.push((tool.name, spec));
            }
            w::Contribution::StatusItem(id) => {
                identifier(&id)?;
                if !keys.insert(("status", id.clone())) {
                    return Err(WasmError::MalformedResult("duplicate status id".into()));
                }
                session.status_ids.push(id);
            }
            w::Contribution::ContextContributor(spec) => components.push((true, spec)),
            w::Contribution::ToolOutputProcessor(spec) => components.push((false, spec)),
        }
    }
    let local_ids = components
        .iter()
        .map(|(phase, spec)| (*phase, spec.id.clone()))
        .collect::<BTreeSet<_>>();
    let components = components
        .into_iter()
        .map(|(context, spec)| {
            identifier(&spec.id)?;
            crate::limits::revision(&spec.revision)?;
            if spec.id.starts_with("runtime.core.")
                || !keys.insert((if context { "context" } else { "output" }, spec.id.clone()))
            {
                return Err(WasmError::MalformedResult(
                    "protected/duplicate component id".into(),
                ));
            }
            let mut descriptor = ComponentDescriptor::new(
                format!("{}.{}", session.namespace, spec.id),
                RegistryRevision::new(&spec.revision),
            );
            for id in &spec.before {
                identifier(id)?;
                descriptor = descriptor.before(if local_ids.contains(&(context, id.clone())) {
                    format!("{}.{}", session.namespace, id)
                } else {
                    id.clone()
                });
            }
            for id in &spec.after {
                identifier(id)?;
                descriptor = descriptor.after(if local_ids.contains(&(context, id.clone())) {
                    format!("{}.{}", session.namespace, id)
                } else {
                    id.clone()
                });
            }
            Ok((context, spec.id, descriptor))
        })
        .collect::<Result<Vec<_>, WasmError>>()?;
    session.refresh()?;
    session.health.record_success();
    let health = session.health.clone();
    let cache = session.cache.clone();
    let namespace = session.namespace.clone();
    let status_ids = session.status_ids.clone();
    let session = Arc::new(Mutex::new(session));
    let mut contributions = Vec::new();
    for (id, spec) in tools {
        contributions.push(ModuleContribution::Tool(Arc::new(GuestTool {
            session: session.clone(),
            id,
            spec,
        })));
    }
    for (context, id, descriptor) in components {
        let component = Arc::new(GuestPipeline {
            session: session.clone(),
            id,
            descriptor,
        });
        contributions.push(ModuleContribution::Pipeline(if context {
            PipelineComponent::ContextContributor(component)
        } else {
            PipelineComponent::ToolOutputProcessor(component)
        }));
    }
    for id in status_ids {
        contributions.push(ModuleContribution::StatusItem {
            name: format!("{namespace}.{id}"),
            source: Arc::new(CachedStatus {
                _session: session.clone(),
                cache: cache.clone(),
                id,
            }),
        });
    }
    Ok(Mounted::TrackedContributions {
        contributions,
        health,
    })
}
// Defense in depth for callers that have not yet supplied the registry catalog.
const BUILTIN_NAMES: &[&str] = &[
    "read",
    "write",
    "edit",
    "bash",
    "shell",
    "search",
    "task_output",
    "task_stop",
    "read_image",
    "generate_image",
    "image",
    "artifact.read",
    "agent",
    "advisor",
    "glob",
    "grep",
    "list",
    "fetch",
    "web_fetch",
    "questionnaire",
    "write_todos",
    "get_goal",
    "create_goal",
    "update_goal",
];
#[derive(Debug)]
struct CachedStatus {
    _session: Arc<Mutex<Session>>,
    cache: Arc<Mutex<BTreeMap<String, StatusItem>>>,
    id: String,
}
impl StatusSource for CachedStatus {
    fn current(&self) -> Option<StatusItem> {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&self.id)
            .cloned()
    }
}
#[derive(Debug)]
struct GuestTool {
    session: Arc<Mutex<Session>>,
    id: String,
    spec: ToolSpec,
}
#[async_trait::async_trait]
impl Tool for GuestTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }
    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let input = w::ToolInput {
            session: ctx.session.to_string(),
            turn: ctx
                .turn
                .as_ref()
                .map_or_else(String::new, ToString::to_string),
            request: ctx.request.to_string(),
            call_id: ctx.call_id.to_string(),
            arguments_json: prepared.into_arguments().to_string(),
        };
        let session = self.session.clone();
        let id = self.id.clone();
        let session_id = ctx.session.clone();
        tokio::task::spawn_blocking(move || {
            let mut session = session.lock().unwrap_or_else(|e| e.into_inner());
            session.bind_session(session_id);
            let result = (|| {
                if let Some(reason) = session.health.failure() {
                    return Err(WasmError::Disabled(reason));
                }
                bounded(&input, &session.store.data().limits)?;
                let deadline = session.store.data().limits.tool_deadline;
                let output = session
                    .call(deadline, |guest, store| {
                        guest.call_call_tool(store, &id, &input)
                    })?
                    .map_err(|error| guest_error(&error, &session.store.data().limits))?;
                if output.len() > session.store.data().limits.payload_bytes {
                    return Err(WasmError::OversizedPayload("tool output".into()));
                }
                let outcome: ToolOutcome = serde_json::from_str(&output)
                    .map_err(|_| WasmError::MalformedResult("invalid ToolOutcome".into()))?;
                crate::limits::bounded_outcome(&outcome, &session.store.data().limits)?;
                Ok(outcome)
            })();
            session
                .finish(result)
                .map_err(|e| RuntimeError::tool(e.to_string()))
        })
        .await
        .map_err(|_| RuntimeError::tool("module host worker failed"))?
    }
}
#[derive(Debug)]
struct GuestPipeline {
    session: Arc<Mutex<Session>>,
    id: String,
    descriptor: ComponentDescriptor,
}
#[async_trait::async_trait]
impl ContextContributor for GuestPipeline {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor.clone()
    }
    async fn contribute(&self, view: &ContextView) -> Result<ContextPatch, RuntimeError> {
        let session = self.session.clone();
        let id = self.id.clone();
        let view = view.clone();
        Ok(tokio::task::spawn_blocking(move || {
            let mut session = session.lock().unwrap_or_else(|e| e.into_inner());
            session.bind_session(view.session.clone());
            // A contributor without history authority still runs on an empty
            // history. Host-side view limits must not disable healthy guests.
            let input = (|| {
                let history = if session
                    .store
                    .data()
                    .granted(ModuleCapability::HostRead, "session-history")
                {
                    crate::projection::messages(&view.history)?
                } else {
                    Vec::new()
                };
                let input = w::ContextView {
                    session: view.session.to_string(),
                    turn: view.turn.to_string(),
                    history,
                    activation: view.activation.as_str().into(),
                    state: crate::projection::state(&view.state)?,
                };
                bounded(&input, &session.store.data().limits)?;
                Ok::<_, WasmError>(input)
            })();
            let input = match input {
                Ok(input) => input,
                Err(error) => {
                    tracing::warn!(error = %error, "context view withheld from sandboxed module");
                    return ContextPatch::default();
                }
            };
            let result = (|| {
                let deadline = session.store.data().limits.component_deadline;
                let patch = session
                    .call(deadline, |guest, store| {
                        guest.call_contribute_context(store, &id, &input)
                    })?
                    .map_err(|error| guest_error(&error, &session.store.data().limits))?;
                bounded(&patch, &session.store.data().limits)?;
                Ok(ContextPatch::new(crate::projection::fragments(
                    patch,
                    &session.namespace,
                )?))
            })();
            session.finish(result).unwrap_or_default()
        })
        .await
        .unwrap_or_default())
    }
}
#[async_trait::async_trait]
impl ToolOutputProcessor for GuestPipeline {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor.clone()
    }
    async fn process(
        &self,
        view: &ToolOutputView,
        outcome: ToolOutcome,
    ) -> Result<ToolOutputPatch, RuntimeError> {
        let session = self.session.clone();
        let id = self.id.clone();
        let view = view.clone();
        let fallback = outcome.clone();
        let output = tokio::task::spawn_blocking(move || {
            let mut session = session.lock().unwrap_or_else(|e| e.into_inner());
            session.bind_session(view.session.clone());
            let result = (|| {
                if let Some(reason) = session.health.failure() {
                    return Err(WasmError::Disabled(reason));
                }
                let input = w::ToolOutputView {
                    session: view.session.to_string(),
                    turn: view.turn.to_string(),
                    request: view.request.to_string(),
                    call: w::ToolCall {
                        id: view.call.id.to_string(),
                        name: view.call.name.clone(),
                        arguments_json: view.call.arguments.to_string(),
                    },
                    state: crate::projection::state(&view.state)?,
                    usage: crate::projection::usage(&view.usage)?,
                    now_monotonic_ms: session.started.elapsed().as_millis().min(u64::MAX as u128)
                        as u64,
                    outcome_json: serde_json::to_string(&outcome)
                        .map_err(|_| WasmError::MalformedResult("outcome encoding".into()))?,
                };
                bounded(&input, &session.store.data().limits)?;
                let deadline = session.store.data().limits.component_deadline;
                let patch = session
                    .call(deadline, |guest, store| {
                        guest.call_process_tool_output(store, &id, &input)
                    })?
                    .map_err(|error| guest_error(&error, &session.store.data().limits))?;
                bounded(&patch, &session.store.data().limits)?;
                let result: ToolOutcome =
                    serde_json::from_str(&patch.outcome_json).map_err(|_| {
                        WasmError::MalformedResult("invalid processed ToolOutcome".into())
                    })?;
                crate::limits::bounded_outcome(&result, &session.store.data().limits)?;
                Ok(result)
            })();
            session.finish(result)
        })
        .await;
        Ok(ToolOutputPatch::outcome(
            output.ok().and_then(Result::ok).unwrap_or(fallback),
        ))
    }
}

fn guest_error(error: &w::ModuleError, limits: &WasmLimits) -> WasmError {
    match bounded(error, limits) {
        Err(error) => error,
        Ok(()) => WasmError::MalformedResult(format!("guest returned {:?}", error.kind)),
    }
}

fn private_layout(config: &mut WasmModuleConfig) -> Result<(), WasmError> {
    let io = |_| WasmError::Load("private module directories unavailable".into());
    std::fs::create_dir_all(&config.host_state_dir).map_err(io)?;
    config.host_state_dir = config.host_state_dir.canonicalize().map_err(io)?;
    let reserved = absolute_location(&config.host_state_dir.join(COMPONENT_CACHE_DIRECTORY))?;
    if !reserved.starts_with(&config.host_state_dir) {
        return Err(WasmError::Load("reserved cache escapes host state".into()));
    }
    // Validate the caller's cache location before reading executable cache bytes.
    // Canonicalize an existing ancestor, preserving a not-yet-created suffix.
    let absolute = absolute_location(&config.cache_dir)?;
    if !absolute.starts_with(&reserved) {
        return Err(WasmError::Load(
            "component cache must be inside the reserved host cache directory".into(),
        ));
    }
    std::fs::create_dir_all(&config.data_dir).map_err(io)?;
    config.data_dir = config.data_dir.canonicalize().map_err(io)?;
    if config.data_dir.starts_with(&reserved) || reserved.starts_with(&config.data_dir) {
        return Err(WasmError::Load(
            "plugin data must not overlap the reserved component cache".into(),
        ));
    }
    config.cache_dir = absolute;
    Ok(())
}
fn absolute_location(path: &std::path::Path) -> Result<PathBuf, WasmError> {
    if let Ok(path) = path.canonicalize() {
        return Ok(path);
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| WasmError::Load("invalid cache directory".into()))?;
    Ok(absolute_location(parent)?.join(name))
}
