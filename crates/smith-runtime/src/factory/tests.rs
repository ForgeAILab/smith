use super::*;

use super::authority::ScopedAutoApprovalRule;
use super::authority::approval;
use super::capabilities::goal_component_eligible;
use super::context_policy::{
    compaction_policy, context_policy, effective_output_budget, loop_config, policy_revision,
    skill_instruction_budget,
};
use super::provider::adapter::{Adapter, adapter, apply_adapter_cache_capability, endpoint};
use super::provider::{cache_endpoint_identity, prepare as prepare_factory_inputs};

use agent_runtime_core::approval::{ApprovalOrigin, ApprovalRequest};
use agent_runtime_core::catalog::ModelLimits;
use agent_runtime_core::clock::Deadline;
use agent_runtime_core::content::Message;
use agent_runtime_core::ids::{AttemptId, RequestId, SessionId, ToolCallId};
use agent_runtime_core::provider::{ProviderAttemptPurpose, ProviderCallContext, ProviderRequest};
use agent_runtime_core::tool::{PreparedToolCall, ToolCallDisplay, ToolEffects};
use futures_util::StreamExt;
use smith_config::model::{AutoApprovalOperation, CacheMaintenanceMode, ProfileUse};
use smith_config::resolve::{
    AutoApprovalRule, ResolvedAgent, ResolvedAgentMode, ResolvedAgentProfile, ResolvedCachePolicy,
    ResolvedChildAgents, ResolvedCommandProvider, ResolvedContext, Source, Sourced,
    SyntheticCacheSpendAuthority,
};
use smith_host::{HeadlessApproval, ProjectWorkspace};

const TOKEN: &str = "sk-live-4kQm2ZpX8vRt7nLb1cWs9aYe";

#[cfg(unix)]
struct FixtureEnvironment;

#[cfg(unix)]
impl smith_config::credential::Environment for FixtureEnvironment {
    fn value(&self, name: &str) -> Option<Secret> {
        (name == "BRIDGE_TOKEN_SOURCE").then(|| Secret::new("fixture-secret"))
    }
}

fn sourced<T>(value: T) -> Sourced<T> {
    Sourced::new(value, Source::built_in("test"))
}

fn image_generation() -> smith_config::resolve::ResolvedImageGeneration {
    smith_config::resolve::ResolvedImageGeneration {
        enabled: sourced(false),
        model: sourced("gpt-image-2".to_owned()),
        quality: sourced("auto".to_owned()),
        size: sourced("auto".to_owned()),
    }
}

fn auto_rule(
    operations: Vec<AutoApprovalOperation>,
    permissions: Vec<AutoApprovalPermission>,
    paths: &[&str],
) -> AutoApprovalRule {
    AutoApprovalRule {
        revision: 1,
        tool: "smith/edit".to_owned(),
        operations,
        permissions,
        max_risk: AutoApprovalRisk::Medium,
        mount: AutoApprovalMount::Workspace,
        paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        expires_at_unix_ms: None,
        max_uses: None,
    }
}

fn prepared_edit(
    mount: &str,
    operation: &str,
    path: &[&str],
    permissions: impl IntoIterator<Item = Permission>,
) -> PreparedToolCall {
    let permissions: PermissionSet = permissions.into_iter().collect();
    let effects = ToolEffects::read_only().with_write(path.join("/"));
    PreparedToolCall::new(
        ToolCallId::new(format!("call-{operation}-{}", path.join("-"))),
        "edit",
        serde_json::json!({"operation": operation, "path": path.join("/")}),
        permissions,
        SecurityResource::filesystem(
            mount,
            path.iter().map(|segment| (*segment).to_owned()).collect(),
        ),
        effects,
        ToolCallDisplay::new(format!("{operation} {}", path.join("/"))),
    )
}

#[test]
fn standard_summary_route_uses_active_provider_and_custom_route_uses_declared_provider() {
    let standard = SmithSemanticSummaryConfig::standard();
    assert_eq!(
        summary_route_provider(&standard, "active-provider"),
        "active-provider"
    );

    let mut custom = standard;
    custom.provider = Some("summary-provider".to_owned());
    assert_eq!(
        summary_route_provider(&custom, "active-provider"),
        "summary-provider"
    );
}

fn cache() -> ResolvedCachePolicy {
    ResolvedCachePolicy {
        requested_maintenance: sourced(CacheMaintenanceMode::Off),
        effective_maintenance: sourced(CacheMaintenanceMode::Off),
        narrowing_reason: None,
        inactivity_limit_ms: sourced(3_600_000),
        max_hold_while_child_ms: sourced(3_600_000),
        max_maintenance_calls: sourced(1),
        max_maintenance_input_tokens: sourced(0),
        max_maintenance_output_tokens: sourced(256),
        maintenance_deadline_ms: sourced(30_000),
        keepalive_margin_ms: sourced(120_000),
        keepalive_jitter_percent: sourced(10),
        handoff_checkpoint: sourced(true),
        idle_compaction: sourced(true),
        resume_capsule: sourced(true),
    }
}

fn child_agents() -> ResolvedChildAgents {
    ResolvedChildAgents {
        wait_default_timeout_ms: sourced(300_000),
        wait_max_timeout_ms: sourced(300_000),
    }
}

fn provider(kind: &str, base_url: Option<&str>) -> ResolvedProvider {
    ResolvedProvider {
        name: sourced("acme".to_owned()),
        kind: sourced(kind.to_owned()),
        base_url: base_url.map(|url| sourced(url.to_owned())),
        credentials: Vec::new(),
        rotate_at_percent: None,
        api_key: None,
        headers: Default::default(),
        response: Default::default(),
        command: None,
    }
}

#[cfg(unix)]
fn command_config(executable: std::path::PathBuf) -> ResolvedConfig {
    let mut config = resolved_config();
    config.provider = provider(KIND_COMMAND_JSONL, None);
    config.provider.command = Some(ResolvedCommandProvider {
        executable: sourced(executable),
        args: None,
        cwd: Some(sourced(CommandWorkingDirectory::Workspace)),
        env: Default::default(),
    });
    config.model_limits.context_tokens = Some(sourced(32_768));
    config.model_limits.max_input_tokens = Some(sourced(28_672));
    config.model_limits.max_output_tokens = Some(sourced(4_096));
    config
}

#[cfg(unix)]
fn executable_fixture(root: &std::path::Path, probe_model: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let executable = root.join("smith-command-fixture.sh");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--smith-provider-probe" ]; then
  printf '%s\n' '{{"protocol":"smith-command-provider","schema_version":1,"model":"{probe_model}","implementation":"factory-fixture","implementation_version":"1.0.0"}}'
  exit 0
fi
if [ "$1" = "--smith-provider-attempt" ]; then
  IFS= read -r request
  if [ "$BRIDGE_TOKEN" = "fixture-secret" ] && [ -z "${{CARGO_MANIFEST_DIR+x}}" ]; then
    text="isolated"
  else
    text="leaked"
  fi
  printf '%s\n' "{{\"protocol\":\"smith-command-provider\",\"schema_version\":1,\"type\":\"text_delta\",\"text\":\"$text\"}}"
  printf '%s\n' '{{"protocol":"smith-command-provider","schema_version":1,"type":"usage","input_tokens":2,"output_tokens":1}}'
  printf '%s\n' '{{"protocol":"smith-command-provider","schema_version":1,"type":"finish","reason":"stop"}}'
  exit 0
fi
exit 2
"#
    );
    std::fs::write(&executable, script).expect("a command-provider fixture");
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
        .expect("an executable fixture");
    executable
}

fn context(output_reserve: Option<u32>, reasoning_reserve: u32) -> ResolvedContext {
    ResolvedContext {
        tool_output_inline_bytes: sourced(8 * 1024),
        output_reserve: output_reserve.map(sourced),
        reasoning_reserve: sourced(reasoning_reserve),
        capability_budget: None,
        max_estimated_slack: None,
        compaction_high_watermark_percent: sourced(85),
        compaction_low_watermark_percent: sourced(60),
        idle_compaction_ms: sourced(3_600_000),
        cache: cache(),
    }
}

fn profile(limits: ModelLimits) -> ResolvedModelProfile {
    ResolvedModelProfile::explicit("acme", ModelId::new("example-model"), limits)
}

fn resolved_config() -> ResolvedConfig {
    ResolvedConfig {
        user_dir: std::path::PathBuf::from("/tmp/smith"),
        profile: None,
        agent: agent(AgentPosture::Build),
        provider: provider(KIND_FAKE, None),
        harness: None,
        model: sourced("example-model".to_owned()),
        max_output_tokens: None,
        context_window: None,
        model_limits: Default::default(),
        reasoning: Default::default(),
        model_reasoning: Default::default(),
        context: context(None, 0),
        limits: limits(),
        synthetic_cache_spend: SyntheticCacheSpendAuthority::Deny,
        child_agents: child_agents(),
        persistence: persistence(),
        approval: approval_config(ApprovalMode::Ask),
        background: background(),
        mcp: Default::default(),
        image_generation: image_generation(),
    }
}

#[test]
fn reasoning_boolean_remains_fixed_and_omission_preserves_provider_default() {
    let config = resolved_config();
    let mut model_profile = profile(ModelLimits::new(128_000, 124_000, 4_096));
    model_profile.capabilities.reasoning = agent_runtime_core::provider::ReasoningSupport::Fixed;

    let policy = resolve_reasoning_policy(&config, &model_profile, None, None)
        .expect("presence-only profile");
    assert_eq!(
        policy.support,
        agent_runtime_core::provider::ReasoningSupport::Fixed
    );
    assert_eq!(
        policy.switch,
        crate::reasoning::ReasoningSwitch::Unavailable
    );
    assert!(policy.efforts.is_empty());
    assert!(policy.request_config().is_none());
}

#[test]
fn explicit_reasoning_metadata_accepts_only_advertised_efforts() {
    let mut config = resolved_config();
    config.model_reasoning = smith_config::resolve::ResolvedModelReasoning {
        toggle: sourced(true).into(),
        mandatory: sourced(false).into(),
        efforts: sourced(vec!["none".to_owned(), "low".to_owned(), "high".to_owned()]).into(),
        default_enabled: sourced(true).into(),
        default_effort: sourced("low".to_owned()).into(),
        dialect: sourced(smith_config::model::ReasoningDialect::OpenaiEffort).into(),
    };
    config.reasoning.enabled = Some(sourced(true));
    config.reasoning.effort = Some(sourced("high".to_owned()));
    let model_profile = profile(ModelLimits::new(128_000, 124_000, 4_096));

    let policy =
        resolve_reasoning_policy(&config, &model_profile, None, None).expect("advertised effort");
    assert_eq!(policy.effective_state(), "on");
    assert_eq!(policy.effective_effort(), "high");
    assert_eq!(
        policy
            .request_config()
            .and_then(|reasoning| reasoning.effort),
        Some("high".to_owned())
    );

    config.reasoning.effort = Some(sourced("extreme".to_owned()));
    let error = resolve_reasoning_policy(&config, &model_profile, None, None)
        .expect_err("unadvertised effort");
    assert!(error.contains("extreme"), "{error}");
    assert!(error.contains("none, low, high"), "{error}");
}

#[test]
fn an_adapter_this_build_does_not_ship_is_never_routed_elsewhere() {
    let err = adapter(&provider("grpc-frontier", None)).expect_err("unavailable");
    assert!(matches!(err, FactoryError::AdapterUnavailable { .. }));
    assert!(err.to_string().contains("grpc-frontier"));
    assert!(err.to_string().contains(KIND_OPENAI_COMPATIBLE));
    assert!(err.to_string().contains(KIND_ANTHROPIC_MESSAGES));

    assert_eq!(
        adapter(&provider(
            KIND_OPENAI_COMPATIBLE,
            Some("https://api.example.test/v1")
        ))
        .expect("a known kind"),
        Adapter::OpenAiCompatible
    );
    assert_eq!(
        adapter(&provider(KIND_ANTHROPIC_MESSAGES, None)).expect("a known kind"),
        Adapter::AnthropicMessages
    );
    assert_eq!(
        adapter(&provider(
            KIND_CHATGPT_RESPONSES,
            Some(smith_config::setup::CHATGPT_ENDPOINT),
        ))
        .expect("a known kind"),
        Adapter::ChatGptResponses
    );
    assert_eq!(
        adapter(&provider(KIND_GEMINI_INTERACTIONS, None)).expect("a known kind"),
        Adapter::GeminiInteractions
    );
    assert_eq!(
        adapter(&provider(KIND_COMMAND_JSONL, None)).expect("a known kind"),
        Adapter::CommandJsonl
    );
    assert_eq!(
        adapter(&provider(KIND_FAKE, None)).expect("a known kind"),
        Adapter::Fake
    );
}

#[cfg(unix)]
#[tokio::test]
async fn command_factory_preflights_and_streams_through_the_shared_provider() {
    let workspace_root = tempfile::tempdir().expect("a workspace");
    let executable = executable_fixture(workspace_root.path(), "example-model");
    let mut config = command_config(executable);
    config.provider.command.as_mut().unwrap().env.insert(
        "BRIDGE_TOKEN".to_owned(),
        sourced(McpValue::Credential("env:BRIDGE_TOKEN_SOURCE".to_owned())),
    );
    let mut request = RuntimeRequest::new(config, HostSurface::Headless);
    request.workspace = Some(Arc::new(
        ProjectWorkspace::new(workspace_root.path()).expect("a project workspace"),
    ));
    let redactor = DefaultRedactor::new();
    request.persistence_redactor = Some(redactor.clone());
    request.credentials = Some(
        CredentialResolver::new(workspace_root.path().join("user-state"))
            .with_environment(Arc::new(FixtureEnvironment)),
    );

    let evidence = preflight(&request)
        .await
        .expect("command preflight evidence");
    assert_eq!(evidence.provider_kind, KIND_COMMAND_JSONL);
    assert_eq!(
        evidence.command_implementation.as_deref(),
        Some("factory-fixture/1.0.0")
    );
    assert!(evidence.endpoint.is_none());
    assert!(evidence.credential.is_none());
    assert!(evidence.model_profile.capabilities.streaming);
    assert!(evidence.model_profile.capabilities.tools);
    assert!(evidence.model_profile.capabilities.usage);
    assert_eq!(
        evidence.model_profile.capabilities.reasoning,
        agent_runtime_core::provider::ReasoningSupport::Unsupported
    );
    assert!(!evidence.model_profile.capabilities.cache);
    assert_eq!(evidence.model_profile.input_modalities, [Modality::Text]);
    assert_eq!(evidence.model_profile.output_modalities, [Modality::Text]);
    assert_eq!(
        redactor.redacted_clone(&serde_json::json!({"text":"fixture-secret"})),
        serde_json::json!({"text":"[redacted]"})
    );

    let prepared = prepare_factory_inputs(&request)
        .await
        .expect("prepared command inputs");
    let provider = prepared.command.expect("a command provider").provider;
    let stream = provider
        .stream(
            ProviderRequest::new(ModelId::new("example-model"), vec![Message::user("hello")]),
            ProviderCallContext {
                session: SessionId::new("session-command"),
                request_id: RequestId::new("request-command"),
                attempt_id: AttemptId::new("attempt-command"),
                cache_identity: None,
                purpose: ProviderAttemptPurpose::Ordinary,
                cancel: Cancellation::new(),
                deadline: Deadline::never(),
            },
        )
        .await
        .expect("one supervised command attempt");
    let events = stream.collect::<Vec<_>>().await;
    assert!(matches!(
        &events[0],
        ProviderStreamEvent::TextDelta { text } if text == "isolated"
    ));
    assert!(matches!(
        events.last(),
        Some(ProviderStreamEvent::Finish {
            reason: FinishReason::Stop
        })
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn command_paths_fail_before_environment_credential_resolution() {
    let workspace_root = tempfile::tempdir().expect("a workspace");
    let missing = workspace_root.path().join("missing-bridge");
    let mut config = command_config(missing);
    config.provider.command.as_mut().unwrap().env.insert(
        "BRIDGE_TOKEN".to_owned(),
        sourced(McpValue::Credential("env:MUST_NOT_BE_READ".to_owned())),
    );
    let mut request = RuntimeRequest::new(config, HostSurface::Headless);
    request.workspace = Some(Arc::new(
        ProjectWorkspace::new(workspace_root.path()).expect("a project workspace"),
    ));

    let error = match prepare_factory_inputs(&request).await {
        Ok(_) => panic!("the missing executable must fail first"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        FactoryError::CommandConfiguration(CommandConfigError::UnresolvablePath {
            field: "executable"
        })
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn incompatible_command_probe_stops_before_runtime_construction() {
    let workspace_root = tempfile::tempdir().expect("a workspace");
    let executable = executable_fixture(workspace_root.path(), "different-model");
    let config = command_config(executable);
    let mut request = RuntimeRequest::new(config, HostSurface::Headless);
    request.workspace = Some(Arc::new(
        ProjectWorkspace::new(workspace_root.path()).expect("a project workspace"),
    ));

    let error = preflight(&request)
        .await
        .expect_err("an exact model mismatch");
    assert!(matches!(error, FactoryError::CommandIncompatible));
}

#[test]
fn only_official_openai_responses_grants_synthetic_safety() {
    let mut ordinary = Capabilities::basic_streaming();
    apply_adapter_cache_capability(
        Adapter::OpenAiResponses,
        Some("https://responses.example.test/v1"),
        false,
        &mut ordinary,
    );
    assert!(ordinary.cache);
    assert_eq!(ordinary.prompt_cache, PromptCacheControl::Implicit);
    let contract = ordinary.cache_contract.expect("ordinary contract");
    assert_eq!(contract.behavior, ProviderCacheBehavior::ImplicitPrefix);
    assert!(!contract.supports_synthetic(ProviderAttemptPurpose::CacheKeepalive));

    let mut official = Capabilities::basic_streaming();
    apply_adapter_cache_capability(
        Adapter::OpenAiResponses,
        Some(OPENAI_ENDPOINT),
        false,
        &mut official,
    );
    let contract = official.cache_contract.expect("official contract");
    assert!(contract.supports_synthetic(ProviderAttemptPurpose::CacheKeepalive));
    assert!(contract.supports_synthetic(ProviderAttemptPurpose::CacheHandoffCheckpoint));
    assert!(!contract.supports_synthetic(ProviderAttemptPurpose::IdleCompaction));

    let mut unsupported = Capabilities::basic_streaming();
    unsupported.cache = true;
    unsupported.prompt_cache = PromptCacheControl::Implicit;
    unsupported.cache_contract = Some(ProviderCacheContract::default());
    apply_adapter_cache_capability(
        Adapter::OpenAiResponses,
        Some(OPENAI_ENDPOINT),
        false,
        &mut unsupported,
    );
    assert!(!unsupported.cache);
    assert_eq!(unsupported.prompt_cache, PromptCacheControl::None);
    assert_eq!(
        unsupported
            .cache_contract
            .expect("unsupported contract")
            .behavior,
        ProviderCacheBehavior::Unsupported
    );
}

#[test]
fn credential_rotation_disables_cache_identity_and_synthetic_work() {
    let mut capabilities = Capabilities::basic_streaming();
    apply_adapter_cache_capability(
        Adapter::OpenAiResponses,
        Some(OPENAI_ENDPOINT),
        true,
        &mut capabilities,
    );

    assert!(!capabilities.cache);
    assert_eq!(capabilities.prompt_cache, PromptCacheControl::None);
    let contract = capabilities.cache_contract.expect("fail-closed contract");
    assert_eq!(contract.behavior, ProviderCacheBehavior::Unsupported);
    assert!(contract.maintenance.is_empty());
    assert_eq!(contract.conformance, None);
}

#[test]
fn an_anthropic_provider_defaults_to_the_official_endpoint() {
    let defaulted = endpoint(
        &provider(KIND_ANTHROPIC_MESSAGES, None),
        Some(smith_config::model::ANTHROPIC_DEFAULT_ENDPOINT),
    )
    .expect("the default endpoint");
    assert_eq!(defaulted, "https://api.anthropic.com/v1");

    // A configured endpoint (e.g. a gateway) still wins and is validated.
    let configured = endpoint(
        &provider(
            KIND_ANTHROPIC_MESSAGES,
            Some("https://claude-gw.example.test/v1/"),
        ),
        Some(smith_config::model::ANTHROPIC_DEFAULT_ENDPOINT),
    )
    .expect("a configured endpoint");
    assert_eq!(configured, "https://claude-gw.example.test/v1");

    let err = endpoint(
        &provider(
            KIND_ANTHROPIC_MESSAGES,
            Some("https://key@claude-gw.example.test/v1"),
        ),
        Some(smith_config::model::ANTHROPIC_DEFAULT_ENDPOINT),
    )
    .expect_err("credentials in the URL are refused even with a default");
    assert!(err.to_string().contains("unusable"));
}

#[test]
fn an_endpoint_keeps_its_path_and_loses_its_trailing_slash() {
    let endpoint = endpoint(
        &provider(
            KIND_OPENAI_COMPATIBLE,
            Some("https://api.example.test:8443/v1/"),
        ),
        None,
    )
    .expect("an endpoint");
    assert_eq!(endpoint, "https://api.example.test:8443/v1");
}

#[test]
fn cache_endpoint_partition_is_opaque_and_credential_scoped() {
    let first = cache_endpoint_identity(
        "openai",
        KIND_OPENAI_RESPONSES,
        Some(OPENAI_ENDPOINT),
        Some("keychain:primary"),
    )
    .expect("endpoint identity");
    let same = cache_endpoint_identity(
        "openai",
        KIND_OPENAI_RESPONSES,
        Some(OPENAI_ENDPOINT),
        Some("keychain:primary"),
    )
    .expect("endpoint identity");
    let other_credential = cache_endpoint_identity(
        "openai",
        KIND_OPENAI_RESPONSES,
        Some(OPENAI_ENDPOINT),
        Some("keychain:secondary"),
    )
    .expect("endpoint identity");
    assert_eq!(first, same);
    assert_ne!(first, other_credential);
    let rendered = format!("{first:?}");
    assert!(!rendered.contains(OPENAI_ENDPOINT));
    assert!(!rendered.contains("keychain:primary"));
    assert!(cache_endpoint_identity("fake", KIND_FAKE, None, None).is_none());
}

#[test]
fn an_endpoint_carrying_a_credential_is_refused_without_being_quoted() {
    for url in [
        &format!("https://smith:{TOKEN}@api.example.test/v1"),
        &format!("https://api.example.test/v1?api_key={TOKEN}"),
    ] {
        let err = endpoint(&provider(KIND_OPENAI_COMPATIBLE, Some(url)), None)
            .expect_err("a refused endpoint");
        let rendered = format!("{err} {err:?}");
        assert!(!rendered.contains(TOKEN), "{rendered}");
    }
}

#[test]
fn an_endpoint_must_be_an_absolute_http_url() {
    for url in ["api.example.test/v1", "ftp://api.example.test/v1"] {
        assert!(
            endpoint(&provider(KIND_OPENAI_COMPATIBLE, Some(url)), None).is_err(),
            "{url}"
        );
    }
}

#[test]
fn an_absent_request_budget_is_derived_and_shared_with_context_policy() {
    let profile = profile(ModelLimits::new(128_000, 124_000, 4_096));
    let mut config = ResolvedConfig {
        user_dir: std::path::PathBuf::from("/tmp/smith"),
        profile: None,
        agent: agent(AgentPosture::Build),
        provider: provider(KIND_FAKE, None),
        harness: None,
        model: sourced("example-model".to_owned()),
        max_output_tokens: None,
        context_window: None,
        model_limits: Default::default(),
        reasoning: Default::default(),
        model_reasoning: Default::default(),
        context: context(None, 0),
        limits: limits(),
        synthetic_cache_spend: SyntheticCacheSpendAuthority::Deny,
        child_agents: child_agents(),
        persistence: persistence(),
        approval: approval_config(ApprovalMode::Ask),
        background: background(),
        mcp: Default::default(),
        image_generation: image_generation(),
    };

    // The small model ceiling is below every automatic bound.
    let budget = effective_output_budget(&config, &profile).expect("a budget");
    let policy = context_policy(&config, &profile.limits, &budget);
    assert_eq!(budget.request_tokens, 4_096);
    assert_eq!(policy.output_reserve, 4_096);

    // The profile's generation ask outranks the automatic value.
    config.max_output_tokens = Some(sourced(1_024));
    let budget = effective_output_budget(&config, &profile).expect("a budget");
    assert_eq!(budget.request_tokens, 1_024);
    assert_eq!(
        context_policy(&config, &profile.limits, &budget).output_reserve,
        1_024
    );

    // And an explicit reserve outranks both.
    config.context = context(Some(8_192), 0);
    let budget = effective_output_budget(&config, &profile).expect("a budget");
    assert_eq!(budget.request_tokens, 1_024);
    assert_eq!(
        context_policy(&config, &profile.limits, &budget).output_reserve,
        8_192
    );
}

/// The capability budget follows the model, because the same absolute
/// count is a comfortable allowance on one window and an instant failure
/// on another. Configuration still wins when it names a value.
#[test]
fn an_unconfigured_capability_budget_scales_with_the_model_window() {
    let budget_for = |limits: ModelLimits| {
        let profile = profile(limits);
        let config = resolved_config();
        let output = effective_output_budget(&config, &profile).expect("an output budget");
        let policy = context_policy(&config, &profile.limits, &output);
        ContextBudget::from_limits(&profile.limits, &policy).capability_budget
    };

    let small = budget_for(ModelLimits::new(128_000, 124_000, 4_096));
    let large = budget_for(ModelLimits::new(1_000_000, 1_000_000, 32_768));
    assert!(
        small < large,
        "a larger window must earn a larger capability budget: {small} vs {large}"
    );
    for budget in [small, large] {
        assert!(
            (MIN_DERIVED_CAPABILITY_BUDGET..=MAX_DERIVED_CAPABILITY_BUDGET).contains(&budget),
            "{budget} is outside the derived clamps"
        );
    }

    // A tiny window is bounded by the window itself, not by the floor.
    let tiny = budget_for(ModelLimits::new(8_000, 8_000, 4_096));
    assert!(tiny < MIN_DERIVED_CAPABILITY_BUDGET, "{tiny}");

    // An explicit value still outranks the derivation.
    let profile = profile(ModelLimits::new(1_000_000, 1_000_000, 32_768));
    let mut config = resolved_config();
    config.context.capability_budget = Some(sourced(12_000));
    let output = effective_output_budget(&config, &profile).expect("an output budget");
    let policy = context_policy(&config, &profile.limits, &output);
    assert_eq!(
        ContextBudget::from_limits(&profile.limits, &policy).capability_budget,
        12_000
    );
}

/// The skill share exists to stop instruction prose crowding out tool
/// schemas, but it is useless if it cannot hold one reference section.
#[test]
fn the_skill_share_always_holds_a_reference_section_without_taking_over() {
    /// The largest single built-in reference section, in tokens.
    const LARGEST_SECTION: u32 = 2_800;

    for capability_budget in [
        12_000,
        MIN_DERIVED_CAPABILITY_BUDGET,
        32_768,
        MAX_DERIVED_CAPABILITY_BUDGET,
    ] {
        let share = skill_instruction_budget(capability_budget);
        assert!(
            share >= LARGEST_SECTION,
            "a {capability_budget} budget leaves {share} for skills, too little \
                 for a reference section"
        );
        assert!(
            share <= capability_budget / 2,
            "a {capability_budget} budget gives skills {share}, crowding the \
                 tool schemas"
        );
    }

    // A budget too small to seat a section still refuses to hand skills
    // more than half of it.
    let cramped = skill_instruction_budget(2_000);
    assert_eq!(cramped, 1_000);
}

#[test]
fn automatic_budget_reaches_both_loop_and_context_policy() {
    let profile = profile(ModelLimits::new(500_000, 500_000, 500_000));
    let config = resolved_config();
    let budget = effective_output_budget(&config, &profile).expect("an automatic budget");
    let policy = context_policy(&config, &profile.limits, &budget);
    let request = RuntimeRequest::new(config, HostSurface::Terminal);
    let loop_config = loop_config(&request, &ModelId::new("example-model"), &budget);

    assert_eq!(budget.request_tokens, 32_768);
    assert_eq!(policy.output_reserve, 32_768);
    assert_eq!(loop_config.max_output_tokens, Some(32_768));
}

#[test]
fn reserves_that_consume_the_whole_window_fail_instead_of_planning() {
    let profile = profile(ModelLimits::new(8_000, 8_000, 4_096));
    let config = ResolvedConfig {
        user_dir: std::path::PathBuf::from("/tmp/smith"),
        profile: None,
        agent: agent(AgentPosture::Build),
        provider: provider(KIND_FAKE, None),
        harness: None,
        model: sourced("example-model".to_owned()),
        max_output_tokens: None,
        context_window: None,
        model_limits: Default::default(),
        reasoning: Default::default(),
        model_reasoning: Default::default(),
        context: context(Some(6_000), 2_000),
        limits: limits(),
        synthetic_cache_spend: SyntheticCacheSpendAuthority::Deny,
        child_agents: child_agents(),
        persistence: persistence(),
        approval: approval_config(ApprovalMode::Ask),
        background: background(),
        mcp: Default::default(),
        image_generation: image_generation(),
    };

    let err = effective_output_budget(&config, &profile).expect_err("no room to plan");
    assert!(matches!(err, FactoryError::ContextReserve { .. }));
}

#[test]
fn a_changed_reserve_changes_the_policy_revision() {
    assert_ne!(
        policy_revision(4_096, 0, None, None),
        policy_revision(8_192, 0, None, None)
    );
    assert_ne!(
        policy_revision(4_096, 0, None, None),
        policy_revision(4_096, 0, Some(12_000), None)
    );
    assert!(policy_revision(4_096, 0, None, None).starts_with(CONTEXT_POLICY_REVISION));
}

#[test]
fn compaction_watermarks_are_derived_from_the_enforced_input_budget() {
    let profile = profile(ModelLimits::new(1_000, 900, 200));
    let mut config = ResolvedConfig {
        user_dir: std::path::PathBuf::from("/tmp/smith"),
        profile: None,
        agent: agent(AgentPosture::Build),
        provider: provider(KIND_FAKE, None),
        harness: None,
        model: sourced("example-model".to_owned()),
        max_output_tokens: None,
        context_window: None,
        model_limits: Default::default(),
        reasoning: Default::default(),
        model_reasoning: Default::default(),
        context: context(Some(100), 0),
        limits: limits(),
        synthetic_cache_spend: SyntheticCacheSpendAuthority::Deny,
        child_agents: child_agents(),
        persistence: persistence(),
        approval: approval_config(ApprovalMode::Ask),
        background: background(),
        mcp: Default::default(),
        image_generation: image_generation(),
    };

    let budget = effective_output_budget(&config, &profile).expect("an output budget");
    let context_policy = context_policy(&config, &profile.limits, &budget);
    let compact = compaction_policy(&config, &profile, &context_policy);
    assert_eq!(
        ContextBudget::from_limits(&profile.limits, &context_policy).input_budget,
        900
    );
    assert_eq!(compact.high_watermark, 765);
    assert_eq!(compact.low_watermark, 540);
    assert_eq!(
        compact.revision.as_str(),
        "smith-compaction-policy-1/high=765/low=540"
    );

    config.context.compaction_low_watermark_percent = sourced(50);
    let changed = compaction_policy(&config, &profile, &context_policy);
    assert_eq!(changed.low_watermark, 450);
    assert_ne!(compact.revision, changed.revision);
}

#[tokio::test]
async fn scoped_auto_approval_is_shared_factory_policy_and_falls_back() {
    let mut config = ResolvedConfig {
        user_dir: std::path::PathBuf::from("/tmp/smith"),
        profile: None,
        agent: agent(AgentPosture::Build),
        provider: provider(KIND_FAKE, None),
        harness: None,
        model: sourced("example-model".to_owned()),
        max_output_tokens: None,
        context_window: None,
        model_limits: Default::default(),
        reasoning: Default::default(),
        model_reasoning: Default::default(),
        context: context(None, 0),
        limits: limits(),
        synthetic_cache_spend: SyntheticCacheSpendAuthority::Deny,
        child_agents: child_agents(),
        persistence: persistence(),
        approval: approval_config(ApprovalMode::Ask),
        background: background(),
        mcp: Default::default(),
        image_generation: image_generation(),
    };
    config.approval.auto.push(sourced(auto_rule(
        vec![AutoApprovalOperation::Replace],
        vec![
            AutoApprovalPermission::FsRead,
            AutoApprovalPermission::FsWrite,
        ],
        &["src/**"],
    )));
    let fallback = Arc::new(HeadlessApproval::new());
    let mut request = RuntimeRequest::new(config, HostSurface::Headless);
    let workspace_root = tempfile::tempdir().expect("workspace root");
    let workspace = ProjectWorkspace::new(workspace_root.path()).expect("workspace");
    let mount = workspace.root().to_owned();
    request.workspace = Some(Arc::new(workspace));
    request.approval = Some(fallback.clone());
    let policy = approval(&request).expect("a composed policy");
    let call = |tool: &str, operation: &str, segments: &[&str]| {
        let mut prepared = prepared_edit(
            &mount,
            operation,
            segments,
            [Permission::FsRead, Permission::FsWrite],
        );
        if tool != "edit" {
            prepared = PreparedToolCall::new(
                ToolCallId::new(format!("call-{tool}")),
                tool,
                prepared.arguments().clone(),
                prepared.required_permissions().clone(),
                prepared.resource().clone(),
                prepared.effects().clone(),
                ToolCallDisplay::new(format!("Run {tool}")),
            );
        }
        ApprovalRequest::new(
            prepared,
            Deadline::never(),
            ApprovalOrigin::new(
                agent_runtime_core::ids::SessionId::new("session-1"),
                agent_runtime_core::ids::RequestId::new("request-1"),
            ),
        )
    };

    assert!(
        policy
            .decide(&call("edit", "replace", &["src", "lib.rs"]))
            .await
            .is_allowed()
    );
    assert!(
        fallback.required().is_none(),
        "the explicit allowlist consulted the fallback"
    );
    assert!(
        !policy
            .decide(&call("edit", "create", &["src", "new.rs"]))
            .await
            .is_allowed()
    );
    assert!(
        !policy
            .decide(&call("shell", "replace", &["src", "lib.rs"]))
            .await
            .is_allowed()
    );
    assert_eq!(
        fallback.required().expect("a denied fallback").tool,
        "edit",
        "the first non-matching prepared operation reached the fallback"
    );
}

#[test]
fn scoped_auto_approval_matches_exact_prepared_authority_and_consumes_uses() {
    let mut rule = auto_rule(
        vec![AutoApprovalOperation::Replace],
        vec![
            AutoApprovalPermission::FsRead,
            AutoApprovalPermission::FsWrite,
            AutoApprovalPermission::FsDelete,
        ],
        &["src/**"],
    );
    rule.max_uses = Some(1);
    let compiled = ScopedAutoApprovalRule::compile(&sourced(rule.clone())).expect("rule");
    let replace = prepared_edit(
        "/repo",
        "replace",
        &["src", "lib.rs"],
        [Permission::FsRead, Permission::FsWrite],
    );
    assert!(compiled.matches_and_consumes(&replace, "/repo"));
    assert!(
        !compiled.matches_and_consumes(&replace, "/repo"),
        "the single use must be consumed atomically"
    );

    let compiled = ScopedAutoApprovalRule::compile(&sourced(rule.clone())).expect("rule");
    assert!(!compiled.matches_and_consumes(
        &prepared_edit(
            "/repo",
            "create",
            &["src", "new.rs"],
            [Permission::FsCreate]
        ),
        "/repo"
    ));
    assert!(!compiled.matches_and_consumes(
        &prepared_edit(
            "/repo",
            "replace",
            &["other", "lib.rs"],
            [Permission::FsRead, Permission::FsWrite]
        ),
        "/repo"
    ));
    assert!(!compiled.matches_and_consumes(
        &prepared_edit(
            "/repo",
            "replace",
            &["src", "lib.rs"],
            [
                Permission::FsRead,
                Permission::FsWrite,
                Permission::FsDelete
            ]
        ),
        "/repo"
    ));
    assert!(!compiled.matches_and_consumes(&replace, "/different-mount"));
}

#[test]
fn scoped_auto_approval_rejects_expired_and_categorical_authority() {
    let mut expired = auto_rule(
        vec![AutoApprovalOperation::Replace],
        vec![
            AutoApprovalPermission::FsRead,
            AutoApprovalPermission::FsWrite,
        ],
        &["src/**"],
    );
    expired.expires_at_unix_ms = Some(0);
    let expired = ScopedAutoApprovalRule::compile(&sourced(expired)).expect("rule");
    let replace = prepared_edit(
        "/repo",
        "replace",
        &["src", "lib.rs"],
        [Permission::FsRead, Permission::FsWrite],
    );
    assert!(!expired.matches_and_consumes(&replace, "/repo"));

    let broad = ScopedAutoApprovalRule::compile(&sourced(auto_rule(
        vec![AutoApprovalOperation::Replace],
        vec![
            AutoApprovalPermission::HostFsRead,
            AutoApprovalPermission::HostFsWrite,
            AutoApprovalPermission::ProcessSpawn,
            AutoApprovalPermission::NetHttp,
            AutoApprovalPermission::DataEgress,
        ],
        &["**"],
    )))
    .expect("rule");
    let effects = ToolEffects::new(Vec::new())
        .with_host_read(smith_tools::HOST_SHELL_RESOURCE_KIND)
        .with_host_write(smith_tools::HOST_SHELL_RESOURCE_KIND, "host:filesystem")
        .with_spawn()
        .with_network()
        .with_data_egress_to("host-network:any");
    let host = PreparedToolCall::new(
        ToolCallId::new("call-host-edit"),
        "edit",
        serde_json::json!({"operation": "replace", "path": "src/lib.rs"}),
        [
            Permission::HostFsRead,
            Permission::HostFsWrite,
            Permission::ProcessSpawn,
            Permission::NetHttp,
            Permission::DataEgress,
        ]
        .into_iter()
        .collect(),
        SecurityResource::other(smith_tools::HOST_SHELL_RESOURCE_KIND, "sha256:action"),
        effects,
        ToolCallDisplay::new("host action"),
    );
    assert!(!broad.matches_and_consumes(&host, "/repo"));
}

#[tokio::test]
async fn host_shell_reaches_policy_and_only_explicit_allow_all_allows_it() {
    let mut config = resolved_config();
    config.approval.mode = sourced(ApprovalMode::Ask);
    config.approval.auto.push(sourced(auto_rule(
        vec![AutoApprovalOperation::Replace],
        vec![
            AutoApprovalPermission::HostFsRead,
            AutoApprovalPermission::HostFsWrite,
            AutoApprovalPermission::ProcessSpawn,
            AutoApprovalPermission::NetHttp,
            AutoApprovalPermission::DataEgress,
        ],
        &["**"],
    )));
    let headless = Arc::new(HeadlessApproval::new());
    let mut request = RuntimeRequest::new(config.clone(), HostSurface::Headless);
    let workspace_root = tempfile::tempdir().expect("workspace root");
    request.workspace = Some(Arc::new(
        ProjectWorkspace::new(workspace_root.path()).expect("workspace"),
    ));
    request.approval = Some(headless.clone());
    let policy = approval(&request).expect("a composed policy");
    let effects = ToolEffects::new(vec![])
        .with_host_read(smith_tools::HOST_SHELL_RESOURCE_KIND)
        .with_host_write(smith_tools::HOST_SHELL_RESOURCE_KIND, "host:filesystem")
        .with_spawn()
        .with_network()
        .with_data_egress_to("host-network:any");
    let call = ApprovalRequest::new(
        PreparedToolCall::new(
            ToolCallId::new("call-shell"),
            "shell",
            serde_json::json!({"command": "cat ~/.ssh/id_ed25519"}),
            [
                Permission::HostFsRead,
                Permission::HostFsWrite,
                Permission::ProcessSpawn,
                Permission::NetHttp,
                Permission::DataEgress,
            ]
            .into_iter()
            .collect(),
            agent_runtime_core::security::SecurityResource::other(
                smith_tools::HOST_SHELL_RESOURCE_KIND,
                "sha256:action",
            ),
            effects,
            ToolCallDisplay::new("Run host shell"),
        ),
        Deadline::never(),
        ApprovalOrigin::new(
            agent_runtime_core::ids::SessionId::new("session-1"),
            agent_runtime_core::ids::RequestId::new("request-1"),
        ),
    );

    assert!(!policy.decide(&call).await.is_allowed());
    assert_eq!(
        headless.required().expect("approval was required").tool,
        "shell"
    );

    config.approval.auto.clear();
    let mut request = RuntimeRequest::new(config, HostSurface::Headless);
    request.approval = Some(Arc::new(AllowAll));
    assert!(
        approval(&request)
            .expect("allow-all policy")
            .decide(&call)
            .await
            .is_allowed()
    );
}

#[test]
fn persistent_goals_are_composed_only_for_persisted_root_surfaces() {
    let root = RuntimeRequest::new(resolved_config(), HostSurface::Terminal);
    assert!(goal_component_eligible(&root));

    let child = RuntimeRequest::new(resolved_config(), HostSurface::Child);
    assert!(!goal_component_eligible(&child));

    let mut config = resolved_config();
    config.persistence.enabled = sourced(false);
    let ephemeral = RuntimeRequest::new(config, HostSurface::Headless);
    assert!(!goal_component_eligible(&ephemeral));
}

#[test]
fn equivalent_terminal_and_headless_inputs_resolve_the_same_harness_policy() {
    let terminal = crate::harness::resolve(crate::harness::HarnessSpec::trusted(
        RuntimeRequest::new(resolved_config(), HostSurface::Terminal),
    ))
    .unwrap();
    let headless = crate::harness::resolve(crate::harness::HarnessSpec::trusted(
        RuntimeRequest::new(resolved_config(), HostSurface::Headless),
    ))
    .unwrap();

    assert_eq!(terminal.identity, headless.identity);
    assert_eq!(terminal.provider, headless.provider);
    assert_eq!(terminal.authority, headless.authority);
    assert_eq!(terminal.persistence, headless.persistence);
    assert_eq!(terminal.context, headless.context);
    assert_eq!(terminal.delegation, headless.delegation);
    assert_eq!(terminal.modules, headless.modules);
}

#[test]
fn policy_changes_receive_distinct_harness_identities() {
    let ask = crate::harness::resolve(crate::harness::HarnessSpec::trusted(RuntimeRequest::new(
        resolved_config(),
        HostSurface::Terminal,
    )))
    .unwrap();
    let mut deny_config = resolved_config();
    deny_config.approval.mode = sourced(ApprovalMode::Deny);
    let deny = crate::harness::resolve(crate::harness::HarnessSpec::trusted(RuntimeRequest::new(
        deny_config,
        HostSurface::Terminal,
    )))
    .unwrap();

    assert_eq!(ask.provider, deny.provider);
    assert_ne!(ask.authority, deny.authority);
    assert_ne!(ask.identity, deny.identity);
}

#[test]
fn differently_resolved_harnesses_keep_modules_and_grants_isolated() {
    fn declared(name: &str, capability: crate::harness::Capability) -> crate::harness::ModuleSpec {
        let capabilities = crate::harness::CapabilitySet::from([capability]);
        crate::harness::ModuleSpec {
            id: crate::harness::ModuleId::parse(format!("test/{name}")).unwrap(),
            revision: crate::harness::ModuleRevision::parse("v1").unwrap(),
            provenance: crate::harness::ModuleProvenance::TrustedHost("test".into()),
            trust: crate::harness::ModuleTrust::TrustedNative,
            contributions: vec![crate::harness::Contribution::Tool {
                name: name.into(),
                required: capabilities.clone(),
            }],
            requested_capabilities: capabilities.clone(),
            granted_capabilities: capabilities,
        }
    }

    let first = crate::harness::resolve(
        crate::harness::HarnessSpec::trusted(RuntimeRequest::new(
            resolved_config(),
            HostSurface::Terminal,
        ))
        .with_module(declared(
            "reader",
            crate::harness::Capability::WorkspaceRead,
        )),
    )
    .unwrap();
    let second = crate::harness::resolve(
        crate::harness::HarnessSpec::trusted(RuntimeRequest::new(
            resolved_config(),
            HostSurface::Headless,
        ))
        .with_module(declared("network", crate::harness::Capability::Network)),
    )
    .unwrap();

    assert_ne!(first.identity, second.identity);
    assert_eq!(first.modules.len(), 1);
    assert_eq!(second.modules.len(), 1);
    assert!(
        first.modules[0]
            .granted_capabilities
            .contains(&crate::harness::Capability::WorkspaceRead)
    );
    assert!(
        !first.modules[0]
            .granted_capabilities
            .contains(&crate::harness::Capability::Network)
    );
    assert!(
        second.modules[0]
            .granted_capabilities
            .contains(&crate::harness::Capability::Network)
    );
    assert!(
        !second.modules[0]
            .granted_capabilities
            .contains(&crate::harness::Capability::WorkspaceRead)
    );
}

fn limits() -> smith_config::resolve::ResolvedLimits {
    smith_config::resolve::ResolvedLimits {
        max_retries: sourced(2),
        max_tool_steps: sourced(0),
        turn_time_limit_ms: sourced(0),
        tool_output_limit_bytes: sourced(65_536),
    }
}

fn persistence() -> smith_config::resolve::ResolvedPersistence {
    smith_config::resolve::ResolvedPersistence {
        enabled: sourced(true),
        sessions_dir: sourced("/state/sessions".into()),
        journal_events: sourced(true),
        checkpoint_key: None,
        checkpoint_key_credential: None,
    }
}

fn agent(posture: AgentPosture) -> ResolvedAgent {
    let name = posture.as_str().to_owned();
    let profile = ResolvedAgentProfile {
        name: name.clone(),
        posture: sourced(posture),
        description: None,
        instructions: None,
        delegation: sourced(true),
        uses: sourced(vec![ProfileUse::Main]),
        provider: None,
        model: None,
        revision: format!("test-{name}-profile-1"),
        legacy: false,
    };
    ResolvedAgent {
        active: sourced(name.clone()),
        order: sourced(vec![name.clone()]),
        modes: std::collections::BTreeMap::from([(
            name.clone(),
            ResolvedAgentMode {
                posture: sourced(posture),
                description: None,
            },
        )]),
        child_presets: Default::default(),
        profile: profile.clone(),
        profiles: std::collections::BTreeMap::from([(name.clone(), profile)]),
        profile_order: sourced(vec![name]),
    }
}

fn approval_config(mode: ApprovalMode) -> smith_config::resolve::ResolvedApproval {
    smith_config::resolve::ResolvedApproval {
        mode: sourced(mode),
        auto_approve: None,
        auto: Vec::new(),
    }
}

fn background() -> smith_config::resolve::ResolvedBackground {
    smith_config::resolve::ResolvedBackground {
        exit_policy: sourced(smith_config::model::BackgroundExit::Error),
        max_children: sourced(4),
        max_monitors: sourced(8),
    }
}
