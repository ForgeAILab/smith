//! Immutable Smith harness resolution and extension trust classification.
//!
//! This module separates declarations, grants, and executable adapters before
//! the single runtime factory performs provider I/O. In-process Rust remains a
//! trusted embedding interface; user-installed executable extensions are
//! rejected until the capability-brokered process transport exists.
#![allow(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use agent_runtime::registry::Permission;
use agent_runtime_core::tool::Tool;
use sha2::{Digest, Sha256};
use smith_config::resolve::Source;

use crate::factory::RuntimeRequest;

/// Smith client-visible identity of one immutable harness composition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessIdentity {
    /// Stable digest of policy-bearing resolved inputs.
    pub revision: String,
}

/// Stable module identity within one harness.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModuleId(String);

impl ModuleId {
    /// Parses a bounded lowercase module identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self, HarnessResolutionError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 96
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_./".contains(&byte)
            })
        {
            return Err(HarnessResolutionError::InvalidModuleId(value));
        }
        Ok(Self(value))
    }

    /// String form used in evidence and protocol records.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Exact implementation revision of a module declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRevision(String);

impl ModuleRevision {
    /// Parses a nonempty bounded revision.
    pub fn parse(value: impl Into<String>) -> Result<Self, HarnessResolutionError> {
        let value = value.into();
        if value.trim().is_empty() || value.len() > 128 {
            return Err(HarnessResolutionError::InvalidModuleRevision);
        }
        Ok(Self(value))
    }

    /// Revision text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Who supplied a module declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleProvenance {
    /// Smith's compiled product.
    BuiltIn,
    /// A host that already executes inside Smith's process.
    TrustedHost(String),
    /// A user-installed manifest, which can never select native execution.
    UserManifest(String),
    /// An explicitly trusted MCP server declaration.
    McpServer(String),
}

/// Execution and trust tier of a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleTrust {
    /// In-process Rust with ordinary ambient same-user authority.
    TrustedNative,
    /// Capability-brokered subprocess. Not enabled until the broker exists.
    BrokeredProcess,
    /// Parsed bounded content with no executable handle.
    ContentOnly,
    /// Declarative client data with no renderer or runtime handle.
    Declarative,
    /// Explicitly trusted external MCP execution.
    TrustedMcp,
}

/// Authority categories a module may request and a host may grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    WorkspaceRead,
    WorkspaceWrite,
    HostRead,
    HostWrite,
    Process,
    Network,
    ExternalRead,
    ExternalWrite,
    DataEgress,
    Credential,
    Approval,
    Provider,
}

/// Deterministic capability set.
pub type CapabilitySet = BTreeSet<Capability>;

/// A module declaration independent from its requested or granted authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contribution {
    /// One tool schema; `required` is validated against both request and grant.
    Tool {
        name: String,
        required: CapabilitySet,
    },
    /// Bounded instruction content. It receives no executable handle.
    Skill { name: String },
    /// A host command declaration.
    Command { name: String },
    /// A redaction-safe event observer declaration.
    Observer { name: String },
    /// Bounded declarative panel data.
    Panel { name: String },
    /// Reserved subprocess tool contribution.
    ProcessTool {
        name: String,
        required: CapabilitySet,
    },
    /// Unsupported native dynamic-library request.
    NativeLibrary { path: String },
}

impl Contribution {
    fn key(&self) -> (&'static str, &str) {
        match self {
            Self::Tool { name, .. } => ("tool", name),
            Self::Skill { name } => ("skill", name),
            Self::Command { name } => ("command", name),
            Self::Observer { name } => ("observer", name),
            Self::Panel { name } => ("panel", name),
            Self::ProcessTool { name, .. } => ("tool", name),
            Self::NativeLibrary { path } => ("native_library", path),
        }
    }

    fn required(&self) -> CapabilitySet {
        match self {
            Self::Tool { required, .. } | Self::ProcessTool { required, .. } => required.clone(),
            Self::Skill { .. }
            | Self::Command { .. }
            | Self::Observer { .. }
            | Self::Panel { .. }
            | Self::NativeLibrary { .. } => CapabilitySet::new(),
        }
    }
}

/// One declaration before trust and grants are validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleSpec {
    pub id: ModuleId,
    pub revision: ModuleRevision,
    pub provenance: ModuleProvenance,
    pub trust: ModuleTrust,
    pub contributions: Vec<Contribution>,
    pub requested_capabilities: CapabilitySet,
    pub granted_capabilities: CapabilitySet,
}

/// A validated module record retained as composition evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModule {
    pub id: ModuleId,
    pub revision: ModuleRevision,
    pub provenance: ModuleProvenance,
    pub trust: ModuleTrust,
    pub contributions: Vec<Contribution>,
    pub requested_capabilities: CapabilitySet,
    pub granted_capabilities: CapabilitySet,
}

/// Provider/model selection evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProviderRecord {
    pub name: String,
    pub model: String,
    pub revision: String,
    pub provenance: Source,
}

/// One revisioned policy-domain record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPolicyRecord {
    pub revision: String,
    pub provenance: Source,
}

/// Bounded, non-secret resolution evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessResolutionReport {
    pub entries: Vec<String>,
}

/// Declarative input to harness resolution.
#[derive(Debug)]
pub struct HarnessSpec {
    request: RuntimeRequest,
    modules: Vec<ModuleSpec>,
    broker_running: bool,
}

impl HarnessSpec {
    /// Starts from a trusted host's typed runtime declaration.
    pub fn trusted(request: RuntimeRequest) -> Self {
        Self {
            request,
            modules: Vec::new(),
            broker_running: false,
        }
    }

    /// Adds a declaration; it grants no authority by being present.
    pub fn with_module(mut self, module: ModuleSpec) -> Self {
        self.modules.push(module);
        self
    }
}

/// Immutable input accepted by the single runtime composition root.
#[derive(Debug)]
pub struct ResolvedHarness {
    pub identity: HarnessIdentity,
    pub provider: ResolvedProviderRecord,
    pub authority: ResolvedPolicyRecord,
    pub persistence: ResolvedPolicyRecord,
    pub context: ResolvedPolicyRecord,
    pub delegation: ResolvedPolicyRecord,
    pub modules: Vec<ResolvedModule>,
    pub report: HarnessResolutionReport,
    request: RuntimeRequest,
}

impl ResolvedHarness {
    pub(crate) fn into_request(self) -> RuntimeRequest {
        self.request
    }
}

/// Compatibility conversion into the immutable harness boundary.
///
/// Standard hosts resolve explicitly; this trait keeps existing trusted test
/// embedders source-compatible during the documented protocol-v1 migration.
pub trait IntoResolvedHarness {
    /// Resolves or returns an already-resolved harness.
    fn into_resolved_harness(self) -> Result<ResolvedHarness, HarnessResolutionError>;
}

impl IntoResolvedHarness for ResolvedHarness {
    fn into_resolved_harness(self) -> Result<ResolvedHarness, HarnessResolutionError> {
        Ok(self)
    }
}

impl IntoResolvedHarness for RuntimeRequest {
    fn into_resolved_harness(self) -> Result<ResolvedHarness, HarnessResolutionError> {
        resolve(HarnessSpec::trusted(self))
    }
}

/// A trusted in-process extension contribution.
///
/// This is not a plugin sandbox. Code installed here can make ambient syscalls
/// with Smith's full process authority.
#[derive(Default, Clone)]
pub struct TrustedNativeModule {
    tools: Vec<Arc<dyn Tool>>,
}

impl TrustedNativeModule {
    /// Adds a trusted in-process tool.
    pub fn add_tool(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }

    /// Trusted tools contributed by the embedding host.
    pub fn tools(&self) -> &[Arc<dyn Tool>] {
        &self.tools
    }
}

impl fmt::Debug for TrustedNativeModule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrustedNativeModule")
            .field(
                "tools",
                &self
                    .tools
                    .iter()
                    .map(|tool| tool.spec().name)
                    .collect::<Vec<_>>(),
            )
            .field("trust", &ModuleTrust::TrustedNative)
            .finish()
    }
}

/// Harness resolution failure, always before provider I/O.
#[derive(Debug, thiserror::Error)]
pub enum HarnessResolutionError {
    #[error("invalid module id `{0}`")]
    InvalidModuleId(String),
    #[error("module revision must be nonempty and at most 128 bytes")]
    InvalidModuleRevision,
    #[error("module `{0}` uses trusted-native execution from an untrusted provenance")]
    InvalidNativeTrust(String),
    #[error("module `{0}` uses MCP execution without MCP-server provenance")]
    InvalidMcpTrust(String),
    #[error("native dynamic library loading is unsupported for module `{0}`")]
    NativeLibraryUnsupported(String),
    #[error("the subprocess capability broker is not running for module `{0}`")]
    BrokerUnavailable(String),
    #[error("contribution `{kind}/{name}` collides between modules")]
    ContributionCollision { kind: String, name: String },
    #[error("module `{module}` contribution `{contribution}` exceeds its requested capabilities")]
    UndeclaredCapability {
        module: String,
        contribution: String,
    },
    #[error("module `{module}` contribution `{contribution}` exceeds its host grant")]
    UngrantedCapability {
        module: String,
        contribution: String,
    },
}

/// Resolves and validates all module declarations before the factory performs
/// provider I/O.
pub fn resolve(spec: HarnessSpec) -> Result<ResolvedHarness, HarnessResolutionError> {
    let HarnessSpec {
        request,
        mut modules,
        broker_running,
    } = spec;

    let trusted = trusted_native_spec(&request)?;
    if !trusted.contributions.is_empty() {
        modules.push(trusted);
    }
    modules.extend(mcp_specs(&request)?);

    let mut contribution_owners = BTreeMap::<(String, String), ModuleId>::new();
    let mut resolved = Vec::with_capacity(modules.len());
    for module in modules {
        validate_module(&module, broker_running)?;
        for contribution in &module.contributions {
            let (kind, name) = contribution.key();
            let key = (kind.to_owned(), name.to_owned());
            if contribution_owners.insert(key, module.id.clone()).is_some() {
                return Err(HarnessResolutionError::ContributionCollision {
                    kind: kind.to_owned(),
                    name: name.to_owned(),
                });
            }
        }
        resolved.push(ResolvedModule {
            id: module.id,
            revision: module.revision,
            provenance: module.provenance,
            trust: module.trust,
            contributions: module.contributions,
            requested_capabilities: module.requested_capabilities,
            granted_capabilities: module.granted_capabilities,
        });
    }
    resolved.sort_by(|left, right| left.id.cmp(&right.id));

    let provider_revision_inputs = [
        format!("{:?}", request.config.provider),
        format!("{:?}", request.config.model),
        format!("{:?}", request.config.model_limits),
        format!("{:?}", request.config.model_reasoning),
        format!("{:?}", request.config.reasoning),
        format!("{:?}", request.config.max_output_tokens),
    ];
    let provider = ResolvedProviderRecord {
        name: request.config.provider.name.value.clone(),
        model: request.config.model.value.clone(),
        revision: digest(provider_revision_inputs.iter().map(String::as_str)),
        provenance: request.config.provider.name.source.clone(),
    };
    let authority_inputs = [
        format!("{:?}", request.config.approval),
        format!("{:?}", request.config.agent.profile),
        format!("{:?}", request.config.synthetic_cache_spend),
        format!("built_in_tools={}", request.built_in_tools),
        format!(
            "workspace={}",
            request
                .workspace
                .as_ref()
                .map_or("<unresolved>", |workspace| workspace.root())
        ),
    ];
    let authority = policy_record(
        "authority-v2",
        authority_inputs.iter().map(String::as_str),
        request.config.approval.mode.source.clone(),
    );
    let persistence_inputs = [format!("{:?}", request.config.persistence)];
    let persistence = policy_record(
        "persistence-v2",
        persistence_inputs.iter().map(String::as_str),
        request.config.persistence.enabled.source.clone(),
    );
    let context_inputs = [format!("{:?}", request.config.context)];
    let context = policy_record(
        "context-v2",
        context_inputs.iter().map(String::as_str),
        request.config.context.reasoning_reserve.source.clone(),
    );
    let mut delegation_inputs = vec![format!("{:?}", request.config.child_agents)];
    delegation_inputs.extend(request.child_profiles.iter().map(|profile| {
        format!(
            "profile={:?};provider={:?};model={:?};agent={:?}",
            profile.config.profile,
            profile.config.provider,
            profile.config.model,
            profile.config.agent.profile
        )
    }));
    let delegation = policy_record(
        "delegation-v2",
        delegation_inputs.iter().map(String::as_str),
        request
            .config
            .child_agents
            .wait_default_timeout_ms
            .source
            .clone(),
    );
    let policy_revisions = [
        authority.revision.as_str(),
        persistence.revision.as_str(),
        context.revision.as_str(),
        delegation.revision.as_str(),
    ];
    let module_records = resolved
        .iter()
        .map(|module| format!("{module:?}"))
        .collect::<Vec<_>>();
    let identity = HarnessIdentity {
        revision: digest(
            std::iter::once(provider.revision.as_str())
                .chain(policy_revisions)
                .chain(module_records.iter().map(String::as_str)),
        ),
    };
    let mut entries = vec![
        format!("harness={}", identity.revision),
        format!("provider={}/{}", provider.name, provider.model),
    ];
    entries.extend(resolved.iter().take(62).map(|module| {
        format!(
            "module={} revision={} trust={:?} contributions={}",
            module.id.as_str(),
            module.revision.as_str(),
            module.trust,
            module.contributions.len()
        )
    }));

    Ok(ResolvedHarness {
        identity,
        provider,
        authority,
        persistence,
        context,
        delegation,
        modules: resolved,
        report: HarnessResolutionReport { entries },
        request,
    })
}

fn trusted_native_spec(request: &RuntimeRequest) -> Result<ModuleSpec, HarnessResolutionError> {
    let mut required = CapabilitySet::new();
    let revision = tool_specs_revision(
        "trusted-native-v2",
        request
            .trusted_native
            .tools()
            .iter()
            .map(|tool| tool.spec()),
    );
    let contributions = request
        .trusted_native
        .tools()
        .iter()
        .map(|tool| {
            let capabilities = tool_capabilities(tool.as_ref());
            required.extend(capabilities.iter().copied());
            Contribution::Tool {
                name: tool.spec().name,
                required: capabilities,
            }
        })
        .collect();
    Ok(ModuleSpec {
        id: ModuleId::parse("smith/trusted-native")?,
        revision: ModuleRevision::parse(revision)?,
        provenance: ModuleProvenance::TrustedHost("runtime-request".into()),
        trust: ModuleTrust::TrustedNative,
        contributions,
        requested_capabilities: required.clone(),
        granted_capabilities: required,
    })
}

fn mcp_specs(request: &RuntimeRequest) -> Result<Vec<ModuleSpec>, HarnessResolutionError> {
    let Some(supervisor) = request.mcp.as_ref() else {
        return Ok(Vec::new());
    };
    let tools = supervisor.tools();
    supervisor
        .reports()
        .into_iter()
        .map(|report| {
            let prefix = format!("mcp__{}__", report.name);
            let mut required = CapabilitySet::new();
            let server_tools = tools
                .iter()
                .filter(|tool| tool.spec().name.starts_with(&prefix))
                .collect::<Vec<_>>();
            let contributions = server_tools
                .iter()
                .map(|tool| {
                    let capabilities = tool_capabilities(tool.as_ref());
                    required.extend(capabilities.iter().copied());
                    Contribution::Tool {
                        name: tool.spec().name,
                        required: capabilities,
                    }
                })
                .collect::<Vec<_>>();
            let revision = tool_specs_revision(
                &format!("mcp-v2:{}", report.transport),
                server_tools.iter().map(|tool| tool.spec()),
            );
            Ok(ModuleSpec {
                id: ModuleId::parse(format!("mcp/{}", report.name))?,
                revision: ModuleRevision::parse(revision)?,
                provenance: ModuleProvenance::McpServer(report.name),
                trust: ModuleTrust::TrustedMcp,
                contributions,
                requested_capabilities: required.clone(),
                granted_capabilities: required,
            })
        })
        .collect()
}

fn validate_module(
    module: &ModuleSpec,
    broker_running: bool,
) -> Result<(), HarnessResolutionError> {
    if module.trust == ModuleTrust::TrustedNative
        && !matches!(
            module.provenance,
            ModuleProvenance::BuiltIn | ModuleProvenance::TrustedHost(_)
        )
    {
        return Err(HarnessResolutionError::InvalidNativeTrust(
            module.id.as_str().to_owned(),
        ));
    }
    if module.trust == ModuleTrust::TrustedMcp
        && !matches!(module.provenance, ModuleProvenance::McpServer(_))
    {
        return Err(HarnessResolutionError::InvalidMcpTrust(
            module.id.as_str().to_owned(),
        ));
    }
    for contribution in &module.contributions {
        if matches!(contribution, Contribution::NativeLibrary { .. }) {
            return Err(HarnessResolutionError::NativeLibraryUnsupported(
                module.id.as_str().to_owned(),
            ));
        }
        if matches!(contribution, Contribution::ProcessTool { .. }) && !broker_running {
            return Err(HarnessResolutionError::BrokerUnavailable(
                module.id.as_str().to_owned(),
            ));
        }
        let required = contribution.required();
        if !required.is_subset(&module.requested_capabilities) {
            return Err(HarnessResolutionError::UndeclaredCapability {
                module: module.id.as_str().to_owned(),
                contribution: contribution.key().1.to_owned(),
            });
        }
        if !required.is_subset(&module.granted_capabilities) {
            return Err(HarnessResolutionError::UngrantedCapability {
                module: module.id.as_str().to_owned(),
                contribution: contribution.key().1.to_owned(),
            });
        }
    }
    Ok(())
}

fn tool_capabilities(tool: &dyn Tool) -> CapabilitySet {
    tool.spec()
        .permission_upper_bound
        .iter()
        .map(|permission| match permission {
            Permission::FsRead => Capability::WorkspaceRead,
            Permission::FsWrite | Permission::FsCreate | Permission::FsDelete => {
                Capability::WorkspaceWrite
            }
            Permission::HostFsRead => Capability::HostRead,
            Permission::HostFsWrite => Capability::HostWrite,
            Permission::ProcessSpawn => Capability::Process,
            Permission::NetHttp => Capability::Network,
            Permission::CredentialUse => Capability::Credential,
            Permission::DataEgress => Capability::DataEgress,
            Permission::ClockRead => Capability::WorkspaceRead,
            Permission::ExternalRead => Capability::ExternalRead,
            Permission::ExternalWrite => Capability::ExternalWrite,
            Permission::StdioRead | Permission::RandomRead => Capability::HostRead,
            Permission::StdioWrite | Permission::Other(_) => Capability::HostWrite,
        })
        .collect()
}

fn policy_record<'a>(
    schema: &str,
    values: impl IntoIterator<Item = &'a str>,
    provenance: Source,
) -> ResolvedPolicyRecord {
    let source = format!("{provenance:?}");
    let mut revision_values = vec![schema.to_owned()];
    revision_values.extend(values.into_iter().map(str::to_owned));
    revision_values.push(source);
    ResolvedPolicyRecord {
        revision: digest(revision_values.iter().map(String::as_str)),
        provenance,
    }
}

fn tool_specs_revision(
    schema: &str,
    specs: impl IntoIterator<Item = agent_runtime_core::tool::ToolSpec>,
) -> String {
    let mut encoded = specs
        .into_iter()
        .map(|spec| serde_json::to_string(&spec).expect("ToolSpec is JSON serializable"))
        .collect::<Vec<_>>();
    encoded.sort_unstable();
    digest(std::iter::once(schema).chain(encoded.iter().map(String::as_str)))
}

fn digest<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(
        trust: ModuleTrust,
        provenance: ModuleProvenance,
        contribution: Contribution,
        requested: CapabilitySet,
        granted: CapabilitySet,
    ) -> ModuleSpec {
        ModuleSpec {
            id: ModuleId::parse("example/module").unwrap(),
            revision: ModuleRevision::parse("v1").unwrap(),
            provenance,
            trust,
            contributions: vec![contribution],
            requested_capabilities: requested,
            granted_capabilities: granted,
        }
    }

    #[test]
    fn declarations_do_not_widen_requests_or_grants() {
        let required = CapabilitySet::from([Capability::Network]);
        let spec = module(
            ModuleTrust::TrustedNative,
            ModuleProvenance::TrustedHost("test".into()),
            Contribution::Tool {
                name: "remote".into(),
                required,
            },
            CapabilitySet::new(),
            CapabilitySet::new(),
        );
        let error = validate_module(&spec, false).unwrap_err();
        assert!(matches!(
            error,
            HarnessResolutionError::UndeclaredCapability { .. }
        ));
    }

    #[test]
    fn user_manifests_cannot_select_native_or_dynamic_library_execution() {
        let native = module(
            ModuleTrust::TrustedNative,
            ModuleProvenance::UserManifest("plugin.toml".into()),
            Contribution::Skill {
                name: "instructions".into(),
            },
            CapabilitySet::new(),
            CapabilitySet::new(),
        );
        assert!(matches!(
            validate_module(&native, false),
            Err(HarnessResolutionError::InvalidNativeTrust(_))
        ));

        let library = module(
            ModuleTrust::BrokeredProcess,
            ModuleProvenance::UserManifest("plugin.toml".into()),
            Contribution::NativeLibrary {
                path: "plugin.dylib".into(),
            },
            CapabilitySet::new(),
            CapabilitySet::new(),
        );
        assert!(matches!(
            validate_module(&library, true),
            Err(HarnessResolutionError::NativeLibraryUnsupported(_))
        ));
    }

    #[test]
    fn executable_process_contributions_are_gated_on_the_broker() {
        let required = CapabilitySet::from([Capability::Process]);
        let process = module(
            ModuleTrust::BrokeredProcess,
            ModuleProvenance::UserManifest("plugin.toml".into()),
            Contribution::ProcessTool {
                name: "worker".into(),
                required: required.clone(),
            },
            required.clone(),
            required,
        );
        assert!(matches!(
            validate_module(&process, false),
            Err(HarnessResolutionError::BrokerUnavailable(_))
        ));
        validate_module(&process, true).unwrap();
    }

    #[test]
    fn tool_descriptor_changes_receive_distinct_module_revisions() {
        let first = agent_runtime_core::tool::ToolSpec::new(
            "example",
            "first reviewed contract",
            serde_json::json!({"type": "object"}),
            agent_runtime_core::tool::ToolEffects::read_only(),
        );
        let second = agent_runtime_core::tool::ToolSpec::new(
            "example",
            "second reviewed contract",
            serde_json::json!({
                "type": "object",
                "properties": {"value": {"type": "string"}}
            }),
            agent_runtime_core::tool::ToolEffects::read_only(),
        );

        assert_ne!(
            tool_specs_revision("module-v1", [first]),
            tool_specs_revision("module-v1", [second])
        );
    }
}
